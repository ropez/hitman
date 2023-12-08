use std::fs::read_to_string;
use std::path::Path;
use eyre::Result;
use toml::Table;
use std::str;
use httparse::Status::*;
use minreq::{Method, Request, Response};
use serde_json::Value;
use log::{log_enabled, info, warn, Level};

use crate::util::truncate;
use crate::extract::extract_variables;
use crate::substitute::substitute;
use crate::env::update_env;

pub fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let buf = substitute(&read_to_string(file_path)?, &env)?;

    print_request(&buf);

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    // FIXME: Should request directly instead of parsing and spoon-feeding minreq?
    let Complete(offset) = req.parse(&buf.as_bytes())? else {
        panic!("Incomplete input")
    };

    let url = req.path.expect("Path should be valid");
    let method = req.method.expect("Method should be valid");

    let body = &buf[offset..];

    let mut request = Request::new(to_method(method), url.to_string()).with_body(body);

    for header in req.headers {
        let value = str::from_utf8(header.value)?;

        request = request.with_header(String::from(header.name), value);
    }

    let t = std::time::Instant::now();
    let response = request.send()?;

    let elapsed = t.elapsed();

    print_response(&response)?;
    warn!("# Request completed in {:.2?}", elapsed);

    if let Ok(json) = response.json::<Value>() {
        let vars = extract_variables(&json, &env)?;
        update_env(&vars)?;
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

