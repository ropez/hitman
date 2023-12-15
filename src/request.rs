use counter::Counter;
use eyre::{ContextCompat, Result};
use futures::future::join_all;
use httparse::Status::*;
use log::{info, log_enabled, warn, Level};
use reqwest::{Client, Method, Request, RequestBuilder, Response, Url};
use serde_json::Value;
use std::fs::read_to_string;
use std::path::Path;
use std::str;
use std::str::FromStr;
use std::time::Duration;
use tokio::task::spawn;
use toml::Table;

use crate::env::update_data;
use crate::extract::extract_variables;
use crate::logging;
use crate::substitute::substitute;
use crate::util::truncate;

pub async fn batch_requests(file_path: &Path, batch: i32, env: &Table) -> Result<()> {
    let buf = substitute(&read_to_string(file_path)?, env)?;

    warn!("# Sending {batch} parallel requests...");

    let t = std::time::Instant::now();

    // This is probably not a very performant way to do this,
    // but it works for now.
    let handles = (0..batch).map(|_| {
        let buf = buf.clone();
        spawn(async move {
            match do_request(&buf).await {
                Ok((res, elapsed)) => Some((res.status().as_u16(), elapsed)),
                Err(_) => None,
            }
        })
    });

    let results: Vec<_> = join_all(handles)
        .await
        .into_iter()
        .filter_map(|h| h.unwrap())
        .collect();

    let elapsed = t.elapsed();

    let average = results.iter().map(|(_, d)| d).sum::<Duration>() / batch as u32;

    let statuses: Counter<_> = results.iter().map(|(s, _)| s).collect();
    let statuses = statuses
        .iter()
        .map(|(s, c)| format!("{}x{}", s, c))
        .collect::<Vec<_>>()
        .join(", ");

    warn!("# Finished in {:.2?}", elapsed);
    warn!("# {} of {} requests completed", results.len(), batch);
    warn!("# Results: {}", statuses);
    warn!("# Average: {:.2?}", average);
    warn!("# Slowest: {:.2?}", Iterator::max(results.iter()).unwrap());
    warn!("# Fastest: {:.2?}", Iterator::min(results.iter()).unwrap());

    Ok(())
}

pub async fn make_request(file_path: &Path, env: &Table) -> Result<()> {
    let buf = substitute(&read_to_string(file_path)?, env)?;

    logging::clear_screen();
    print_request(&buf);

    let (response, elapsed) = do_request(&buf).await?;

    print_response(&response)?;

    if let Ok(json) = response.json::<Value>().await {
        println!("{}", serde_json::to_string_pretty(&json)?);
        let vars = extract_variables(&json, env)?;
        update_data(&vars)?;
    }

    warn!("# Request completed in {:.2?}", elapsed);

    Ok(())
}

async fn do_request(buf: &str) -> Result<(Response, Duration)> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let parse_result = req.parse(buf.as_bytes())?;

    let url = req.path.context("Path should be valid")?;
    let method = req.method.context("Method should be valid")?;

    let method = Method::from_str(method)?;
    let url = Url::parse(url)?;
    let client = Client::new();

    let mut builder = RequestBuilder::from_parts(client, Request::new(method, url));

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
