use eyre::Result;
use std::fs::read_to_string;
use std::str;
use httparse::Status::*;
use colored::*;
use minreq::{Method, Request, Response};
use serde_json::Value;

mod env;
use env::{load_env, update_env};

mod extract;
use extract::extract_variables;

mod substitute;
use substitute::substitute;

// √ HTTP Request files
// √ Show stylized response
// √ Save response for reference
// √ Toml config with environments
// √ Generalized variable substitution
// √ Structured storage
// √ Select environment
// - 'env' should be a plain string-> string map
// - Verbosity control
// - Structured code
// - Error handling
// - High performance
// - Enterprise cloud server?
// - Workflows (batch, pipelines)?
// - Interactive mode?
// - HTTP cookes?

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file_path = args.get(1).expect("argument should be provided");

    let env = load_env(file_path)?;

    let buf = substitute(&read_to_string(file_path)?, &env)?;

    // println!("{}\n--\n", buf.blue());

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    // FIXME: Should request directly instead of parsing and spoon-feeding minreq?
    let Complete(offset) = req.parse(&buf.as_bytes())? else {
        panic!("Incomplete input")
    };

    let path = req.path.expect("Path should be valid");
    let method = req.method.expect("Method should be valid");

    let body = &buf[offset..];

    let mut request = Request::new(to_method(method), path.to_string())
        .with_body(body);

    for header in req.headers {
        let value = str::from_utf8(header.value)?;

        request = request.with_header(String::from(header.name), value);
    }

    let response = request.send()?;

    print_response(&response)?;

    let json: Value = response.json()?;
    let vars = extract_variables(&json, &env).unwrap();

    update_env(&vars)?;

    Ok(())
}

fn to_method(input: &str) -> Method {
    Method::Custom(input.to_uppercase())
}

fn print_response(res: &Response) -> Result<()> {
    let status = format!("HTTP/1.1 {} {}", res.status_code, res.reason_phrase);
    eprintln!("{}", status.cyan());

    let mut head = String::new();
    for (name, value) in &res.headers {
        head.push_str(&format!("{}: {}\n", name, value));
    }

    eprintln!("{}", head.cyan());

    // FIXME Check if content type is JSON

    let json: Value = res.json()?;

    println!("{}", serde_json::to_string_pretty(&json)?);

    Ok(())
}

