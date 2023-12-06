use dialoguer::FuzzySelect;
use dialoguer::theme::ColorfulTheme;
use eyre::{Result, bail};
use toml::Table;
use std::fs::read_to_string;
use std::path::{Path, PathBuf};
use std::str;
use std::env::current_dir;
use walkdir::WalkDir;
use httparse::Status::*;
use minreq::{Method, Request, Response};
use serde_json::Value;
use log::{log_enabled, info, Level, error};

mod cli;

#[macro_use]
mod util;
use util::truncate;

mod logging;

mod env;
use env::{select_env, find_root_dir, load_env, update_env};

mod extract;
use extract::extract_variables;

mod substitute;
use substitute::{substitute, SubstituteError};

mod prompt;
use prompt::set_interactive_mode;

fn main() -> Result<()> {
    let args = cli::parse_args();

    logging::init(args.verbose, args.quiet)?;

    let Some(root_dir) = find_root_dir()? else {
        bail!("No hitman.toml found");
    };

    if args.select {
        select_env(&root_dir)?;
        return Ok(());
    }

    set_interactive_mode(true);

    let cwd = current_dir()?;

    if let Some(file_path) = args.name {
        let file_path = cwd.join(file_path);
        let env = load_env(&root_dir, &file_path, &args.options)?;
        make_request(&file_path, &env)
    } else {
        loop {
            let files = find_available_requests(&cwd)?;
            let display_names = files.iter().map(|p| p.display()).collect::<Vec<_>>();

            eprintln!();
            let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
                .with_prompt("Make request")
                .items(&display_names)
                .interact_opt()?;

            match selection {
                Some(index) => {
                    let file_path = &files[index];

                    let env = load_env(&root_dir, file_path, &args.options)?;

                    let result = make_request(&cwd.join(file_path), &env);
                    if !args.repeat {
                        break result;
                    }

                    match result {
                        Ok(()) => (),
                        Err(e) => {
                            match e.downcast_ref() {
                                Some(SubstituteError::UserCancelled) => {},
                                _ => {
                                    error!("{}", e);
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

fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let buf = substitute(&read_to_string(file_path)?, &env)?;

    print_request(&buf);

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
        info!("HTTP/1.1 {} {}", res.status_code, res.reason_phrase);

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

fn find_available_requests(cwd: &Path) -> Result<Vec<PathBuf>, eyre::Error> {
    let files: Vec<_> = WalkDir::new(cwd)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name().to_str().map(|s| s.ends_with(".http")).unwrap_or(false)
        })
        .map(|p| {
            // Convert to relative path, based on depth
            let components: Vec<_> = p.path().components().collect();
            let relative_components: Vec<_> = components[(components.len() - p.depth())..]
                .iter()
                .map(|c| c.as_os_str())
                .collect();

            PathBuf::from_iter(&relative_components)
        })
        .collect();

    Ok(files)
}

