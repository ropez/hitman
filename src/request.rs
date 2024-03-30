use anyhow::{bail, Context, Result};
use futures::future::join_all;
use httparse::Status::*;
use log::{info, log_enabled, warn, Level};
use reqwest::{Client, Method, Response, Url};
use serde_json::Value;
use spinoff::{spinners, Color, Spinner, Streams};
use std::convert::identity;
use std::fs::read_to_string;
use std::path::Path;
use std::str;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tokio::spawn;
use toml::Table;

use crate::env::{update_data, HitmanCookieJar};
use crate::extract::extract_variables;
use crate::substitute::substitute;
use crate::util::{split_work, truncate, IterExt};

static USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

fn build_client() -> Result<Client> {
    let client = Client::builder()
        .user_agent(USER_AGENT)
        .cookie_provider(Arc::new(HitmanCookieJar))
        .build()?;
    Ok(client)
}

pub async fn flurry_attack(
    file_path: &Path,
    flurry_size: i32,
    connections: i32,
    env: &Table,
) -> Result<()> {
    if flurry_size < 1 {
        bail!("Flurry size must be at least 1");
    }
    if connections < 1 {
        bail!("Connections must be at least 1");
    }

    let client = build_client()?;

    warn!("# Sending {flurry_size} requests on {connections} parallel connections...");

    let buf = substitute(&read_to_string(file_path)?, env)?;

    let t = std::time::Instant::now();
    let mut spinner =
        Spinner::new_with_stream(spinners::BouncingBall, "", Color::Yellow, Streams::Stderr);

    // Run each request in a separate tokio task.
    // It might make it more efficient, if we let each task run a series
    // of requests using a single connection.
    let handles = split_work(flurry_size, connections).map(|size| {
        let buf = buf.clone();
        let client = client.clone();
        spawn(async move {
            let mut results = Vec::new();
            for _ in 0..size {
                let res = match do_request(&client, &buf).await {
                    Ok((res, elapsed)) => Some((res.status().as_u16(), elapsed)),
                    Err(_) => None,
                };
                results.push(res);
            }
            results
        })
    });

    let results: Vec<_> = join_all(handles)
        .await
        .into_iter()
        .filter_map(|h| h.ok())
        .flatten()
        .filter_map(identity)
        .collect();

    spinner.stop();
    let elapsed = t.elapsed();

    let average = results.iter().map(|(_, d)| d).sum::<Duration>() / results.len() as u32;

    let statuses = results.iter().map(|(s, _)| s).counted();
    let statuses = statuses
        .iter()
        .map(|(s, c)| format!("{} ({})", s, c))
        .collect::<Vec<_>>()
        .join(", ");

    warn!("# Finished in {:.2?}", elapsed);
    warn!("# {} of {} requests completed", results.len(), flurry_size);
    warn!("# Results: {}", statuses);
    warn!("# Average: {:.2?}", average);
    warn!("# Slowest: {:.2?}", Iterator::max(results.iter()).unwrap());
    warn!("# Fastest: {:.2?}", Iterator::min(results.iter()).unwrap());

    Ok(())
}

pub async fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let client = build_client()?;

    let buf = substitute(&read_to_string(file_path)?, env)?;

    clear_screen();
    print_request(&buf);

    // let mut spinner =
    //     Spinner::new_with_stream(spinners::BouncingBar, "", Color::Yellow, Streams::Stderr);
    let (response, elapsed) = do_request(&client, &buf).await?;
    // spinner.stop();

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

async fn do_request(client: &Client, buf: &str) -> Result<(Response, Duration)> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let parse_result = req.parse(buf.as_bytes())?;

    let url = req.path.context("Path should be valid")?;
    let method = req.method.context("Method should be valid")?;

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
