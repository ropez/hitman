use std::fs;
use std::str;
use std::env;
use std::error::Error;
use httparse::Status::*;
use colored::*;
use minreq::{Method, Request, Response};
use serde_json::Value;
use toml::Table;

// √ HTTP Request files
// √ Show stylized response
// √ Save response for reference
// √ Toml config with sections
// - Generalized variable substitution
// - Verbosity control
// - Select scope
// - Structured code
// - Structured storage
// - High performance
// - Enterprise cloud server
// - Workflows
// - Interactive mode?
// - HTTP cookes?

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let file_path = &args[1];

    let config = match fs::read_to_string("hittup.toml") {
        Ok(cfg) => {
            toml::from_str::<Table>(&cfg)?
        },
        Err(_) => {
            panic!("hittup.toml not found");
        },
    };

    let section = match config.get("default") {
        Some(v) => match v {
            toml::Value::Table(t) => t,
            _ => panic!("`dafault` has unexpected type in config"),
        },
        _ => panic!("`dafault` not found in config"),
    };

    println!("{:?}", config);

    let buf = fs::read(file_path)?;

    let token = fs::read_to_string(".token").ok();

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let Complete(offset) = req.parse(&buf)? else {
        panic!("Incomplete input")
    };

    let Some(path) = req.path else {
        panic!("Path not found")
    };

    let Some(method) = req.method else {
        panic!("HTTP method not found")
    };

    let body = &buf[offset..];

    let newpath = path.replace("{{baseUrl}}", match section.get("baseUrl") {
        Some(v) => match v {
            toml::Value::String(url) => url,
            _ => panic!("baseUrl not recognized"),
        },
        None => panic!("baseUrl not found"),
    });

    let mut request = Request::new(to_method(method), newpath.to_string())
        .with_body(body);

    for header in req.headers {
        let original = str::from_utf8(header.value)?;
        let value = match token {
            Some(ref t) => original.replace("{{token}}", &t),
            None => original.to_string(),
        };
        let value2 = value.replace("{{baseUrl}}", match section.get("baseUrl") {
            Some(v) => match v {
                toml::Value::String(url) => url,
                _ => panic!("baseUrl not recognized"),
            },
            None => panic!("baseUrl not found"),
        });
        request = request.with_header(String::from(header.name), value2);
    }

    let response = request.send()?;

    print_response(&response)?;

    Ok(())
}

fn to_method(input: &str) -> Method {
    Method::Custom(input.to_uppercase())
}

fn print_response(res: &Response) -> Result<(), Box<dyn Error>> {
    let status = format!("HTTP/1.1 {} {}", res.status_code, res.reason_phrase);
    println!("{}", status.dimmed());

    let mut head = String::new();
    for (name, value) in &res.headers {
        head.push_str(&format!("{}: {}\n", name, value));
    }

    println!("{}", head.dimmed());

    // FIXME Check if content type is JSON

    let json: Value = res.json()?;

    println!("{}", serde_json::to_string_pretty(&json)?);

    if json["AccessToken"].is_string() {
        let token = json["AccessToken"].as_str();
        fs::write(".token", token.unwrap())?;
    }

    Ok(())
}

