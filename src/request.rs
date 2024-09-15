use anyhow::{Context, Result};
use httparse::Status::*;
use log::{info, log_enabled, warn, Level};
use reqwest::{Client, Method, Response, Url};
use serde_json::Value;
use spinoff::{spinners, Color, Spinner, Streams};
use std::{
    fs::read_to_string,
    path::Path,
    str::{self, FromStr},
    sync::Arc,
    time::Duration,
};
use toml::Table;

use crate::{
    env::{update_data, HitmanCookieJar},
    extract::extract_variables,
    prompt::{get_interaction, substitute_interactive},
    util::truncate,
};

static USER_AGENT: &str =
    concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

pub fn build_client() -> Result<Client> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .cookie_provider(Arc::new(HitmanCookieJar))
        .build()?;
    Ok(client)
}

pub async fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let client = build_client()?;

    let interaction = get_interaction();

    let buf = substitute_interactive(
        &read_to_string(file_path)?,
        env,
        interaction.as_ref(),
    )?;

    clear_screen();
    print_request(&buf);

    let mut spinner = Spinner::new_with_stream(
        spinners::BouncingBar,
        "",
        Color::Yellow,
        Streams::Stderr,
    );
    let (response, elapsed) = do_request(&client, &buf).await?;
    spinner.stop();

    print_response(&response)?;

    if let Ok(json) = response.json::<Value>().await {
        println!("{}", serde_json::to_string_pretty(&json)?);
        let vars = extract_variables(&json, env)?;
        update_data(&vars)?;
    }

    warn!("# Request completed in {:.2?}", elapsed);

    Ok(())
}

fn clear_screen() {
    if cfg!(windows) {
        std::process::Command::new("cmd")
            .args(["/c", "cls"])
            .spawn()
            .expect("cls command failed to start")
            .wait()
            .expect("failed to wait");
    } else {
        // Untested!
        println!("\x1B[2J");
    }
}

pub async fn do_request(
    client: &Client,
    buf: &str,
) -> Result<(Response, Duration)> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let parse_result = req
        .parse(buf.as_bytes())
        .context("Invalid input: malformed request")?;

    let method = req.method.context("Invalid input: HTTP method not found")?;
    let url = req.path.context("Invalid input: URL not found")?;

    let method = Method::from_str(method)?;
    let url = Url::parse(url)?;

    let mut builder = client.request(method, url);

    if let Complete(offset) = parse_result {
        let body = &buf[offset..];
        builder = builder.body(body.to_owned());
    }

    for header in req.headers {
        // The parse_http crate is weird, it fills the array with empty headers
        // if a partial request is parsed.
        if header.name.is_empty() {
            break;
        }
        let value = str::from_utf8(header.value)?;

        builder = builder.header(String::from(header.name), value);
    }

    let t = std::time::Instant::now();
    let response = builder.send().await?;

    let elapsed = t.elapsed();

    Ok((response, elapsed))
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
        let status = res.status();
        info!(
            "< HTTP/1.1 {} {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("")
        );

        let mut head = String::new();
        for (name, value) in res.headers() {
            head.push_str(&format!("{}: {}\n", name, value.to_str()?));
        }

        for line in head.lines() {
            info!("< {}", truncate(line));
        }

        info!("");
    }

    Ok(())
}
