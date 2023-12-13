use eyre::Result;
use httparse::Status::*;
use log::{info, log_enabled, warn, Level};
use minreq::{Method, Request, Response};
use serde_json::Value;
use std::fs::read_to_string;
use std::path::Path;
use std::str;
use toml::Table;

use crate::env::update_data;
use crate::extract::extract_variables;
use crate::substitute::substitute;
use crate::util::truncate;

pub fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let buf = substitute(&read_to_string(file_path)?, env)?;

    print_request(&buf);

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let parse_result = req.parse(buf.as_bytes())?;

    let url = req.path.expect("Path should be valid");
    let method = req.method.expect("Method should be valid");

    let mut request = Request::new(to_method(method), url.to_string());

    if let Complete(offset) = parse_result {
        let body = &buf[offset..];
        request = request.with_body(body);
    }

    for header in req.headers {
        // The parse_http crate is weird, it fills the array with empty headers
        // if a partial request is parsed.
        if header.name.is_empty() {
            break;
        }
        let value = str::from_utf8(header.value)?;

        request = request.with_header(String::from(header.name), value);
    }

    let t = std::time::Instant::now();
    let response = request.send()?;

    let elapsed = t.elapsed();

    print_response(&response)?;
    warn!("# Request completed in {:.2?}", elapsed);

    if let Ok(json) = response.json::<Value>() {
        let vars = extract_variables(&json, env)?;
        update_data(&vars)?;
    }

    Ok(())
}

fn to_method(input: &str) -> Method {
    Method::Custom(input.to_uppercase())
}

fn print_request(buf: &str) {
    if log_enabled!(Level::Info) {
        for line in buf.lines() {
            info!("> {}", truncate(line));
        }

        info!("");
    }
}

fn print_response(res: &Response) -> Result<()> {
    if log_enabled!(Level::Info) {
        info!("< HTTP/1.1 {} {}", res.status_code, res.reason_phrase);

        let mut head = String::new();
        for (name, value) in &res.headers {
            head.push_str(&format!("{}: {}\n", name, value));
        }

        for line in head.lines() {
            info!("< {}", truncate(line));
        }

        info!("");
    }

    // FIXME Check if content type is JSON

    if let Ok(json) = res.json::<Value>() {
        println!("{}", serde_json::to_string_pretty(&json)?);
    }

    Ok(())
}
