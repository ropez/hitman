use anyhow::{bail, Result};
use futures::future::join_all;
use log::warn;
use spinoff::{spinners, Color, Spinner, Streams};
use toml::Table;

use std::path::Path;
use std::time::Duration;
use tokio::spawn;

use crate::prompt::{get_interaction, prepare_request_interactive};
use crate::request::{build_client, do_request};
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
    let req =
        prepare_request_interactive(file_path, &env.clone().into(), interaction.as_ref())?;

    let t = std::time::Instant::now();
    let mut spinner = Spinner::new_with_stream(
        spinners::BouncingBall,
        "",
        Color::Yellow,
        Streams::Stderr,
    );

    // Run each request in a separate tokio task.
    // It might make it more efficient, if we let each task run a series
    // of requests using a single connection.
    let handles = split_work(flurry_size, connections).map(|size| {
        let client = client.clone();
        let req = req.clone();
        spawn(async move {
            let mut results = Vec::new();
            for _ in 0..size {
                let res = match do_request(&client, &req).await {
                    Ok((res, elapsed)) => {
                        Some((res.status().as_u16(), elapsed))
                    }
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
        .filter_map(Result::ok)
        .flatten()
        .flatten()
        .collect();

    spinner.stop();
    let elapsed = t.elapsed();

    let average =
        results.iter().map(|(_, d)| d).sum::<Duration>() / u32::try_from(results.len())?;

    let statuses = results.iter().map(|(s, _)| s).counted();
    let statuses = statuses
        .iter()
        .map(|(s, c)| format!("{s} ({c})"))
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
