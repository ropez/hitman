use anyhow::{bail, Result};
use futures::future::join_all;
use log::warn;
use spinoff::{spinners, Color, Spinner, Streams};
use std::convert::identity;
use std::fs::read_to_string;
use std::path::Path;
use std::time::Duration;
use tokio::spawn;
use toml::Table;

use crate::prompt::get_interaction;
use crate::request::{build_client, do_request};
use crate::substitute::substitute;
use crate::util::{split_work, IterExt};

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

    let interaction = get_interaction();
    let buf = substitute(&read_to_string(file_path)?, env, interaction.as_ref())?;

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
