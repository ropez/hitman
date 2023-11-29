use std::fs;
use std::io;
use std::str;
use std::env;
use std::error::Error;
use httparse::Status::*;
use colored::*;
use minreq::{Method, Request, Response};
use serde_json::Value;
use toml::Table as TomlTable;
use derive_more::{Display, Error};
use hittup::{load, extract_variables};

// √ HTTP Request files
// √ Show stylized response
// √ Save response for reference
// √ Toml config with sections
// √ Generalized variable substitution
// √ Structured storage
// - Error handling
// - Verbosity control
// - Select scope
// - Structured code
// - High performance
// - Enterprise cloud server?
// - Workflows (batch, pipelines)?
// - Interactive mode?
// - HTTP cookes?

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    let file_path = &args[1];

    let scope = build_scope(file_path)?;

    let buf = load(file_path, &scope)?;

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
    let vars = extract_variables(&json, &scope).unwrap();

    // If vars is not empty, then open the file '.state', parse it as toml,
    // merge vars into the result, and write that back to the file as toml.
    if !vars.is_empty() {
        let content = fs::read_to_string(".state.toml").unwrap_or("".to_string());
        let mut state = toml::from_str::<TomlTable>(&content).unwrap_or(TomlTable::new());
        state.extend(vars);
        fs::write(".state.toml", toml::to_string_pretty(&state).unwrap())?;
    }

    Ok(())
}

fn build_scope(file_path: &str) -> Result<TomlTable, ReadTomlError> {
    use toml::Value::Table;

    let mut scope = TomlTable::new();

    let config = read_toml("hittup.toml")?;

    match config.get("default") {
        Some(Table(t)) => {
            scope.extend(t.clone());
        },
        Some(_) => panic!("`default` has unexpected type in config"),
        None => panic!("`default` not found in config"),
    };

    match read_toml(".state.toml").ok() {
        Some(content) => scope.extend(content),
        None => (),
    }

    match read_toml(&format!("{}.toml", file_path)).ok() {
        Some(content) => scope.extend(content),
        None => (),
    }

    Ok(scope)
}

fn read_toml(file_path: &str) -> Result<TomlTable, ReadTomlError> {
    let content = fs::read_to_string(file_path).map_err(|err| ReadTomlError::IoError(err))?;

    let cfg = toml::from_str::<TomlTable>(&content).map_err(|err| ReadTomlError::TomlError(err))?;

    Ok(cfg)
}

#[derive(Debug, Display, Error)]
enum ReadTomlError {
    IoError(io::Error),
    TomlError(toml::de::Error),
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

    Ok(())
}

