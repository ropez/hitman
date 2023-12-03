use dialoguer::FuzzySelect;
use dialoguer::theme::ColorfulTheme;
use eyre::Result;
use std::fs::{read_to_string, read_dir};
use std::str;
use httparse::Status::*;
use colored::*;
use minreq::{Method, Request, Response};
use serde_json::Value;

#[macro_use]
mod util;
use util::truncate;

mod env;
use env::{select_env, load_env, update_env};

mod extract;
use extract::extract_variables;

mod substitute;
use substitute::{substitute, SubstituteError};

mod prompt;
use prompt::set_interactive_mode;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a.eq("--select")) {
        select_env()?;
        return Ok(());
    }

    set_interactive_mode(true);

    if let Some(file_path) = args.get(1) {
        make_request(file_path)
    } else {
        loop {
            let files = find_available_requests()?;

            eprintln!();
            let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
                .with_prompt("Make request")
                .items(&files)
                .interact_opt()?;

            match selection {
                Some(index) => {
                    let file_path = &files[index];

                    match make_request(file_path) {
                        Ok(()) => (),
                        Err(e) => {
                            match e.downcast_ref() {
                                Some(SubstituteError::UserCancelled) => {},
                                _ => {
                                    eprintln!("{}", e);
                                }
                            }
                        }
                    }
                },
                None => break Ok(()),
            };
        }
    }
}

fn make_request(file_path: &str) -> Result<()> {
    let env = load_env(file_path)?;

    let buf = substitute(&read_to_string(file_path)?, &env)?;

    print_request(&buf)?;

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

    if let Ok(json) = response.json::<Value>() {
        let vars = extract_variables(&json, &env)?;
        update_env(&vars)?;
    }

    Ok(())
}

fn to_method(input: &str) -> Method {
    Method::Custom(input.to_uppercase())
}

fn print_request(buf: &str) -> Result<()> {
    for line in buf.lines() {
        eprintln!("> {}", truncate(line).blue());
    }

    eprintln!("");

    Ok(())
}

fn print_response(res: &Response) -> Result<()> {
    let status = format!("HTTP/1.1 {} {}", res.status_code, res.reason_phrase);
    eprintln!("< {}", status.cyan());

    let mut head = String::new();
    for (name, value) in &res.headers {
        head.push_str(&format!("{}: {}\n", name, value));
    }

    for line in head.lines() {
        eprintln!("< {}", truncate(line).cyan());
    }

    // FIXME Check if content type is JSON

    if let Ok(json) = res.json::<Value>() {
        eprintln!();
        println!("{}", serde_json::to_string_pretty(&json)?);
    }

    eprintln!();

    Ok(())
}

fn find_available_requests() -> Result<Vec<String>, eyre::Error> {
    let files: Vec<_> = read_dir(".")?
        .into_iter()
        .filter(|r| r.is_ok())
        .map(|r| r.expect("is ok").path())
        .filter(|p| {
            if let Some(ext) = p.extension() { 
                ext == "http"
            } else {
                false
            }
        })
        .map(|p| String::from(p.file_name().unwrap().to_str().unwrap()))
        .collect();

    Ok(files)
}

