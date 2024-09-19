use anyhow::{bail, Result};
use log::warn;
use tokio::time::sleep;

use std::path::Path;
use std::time::Duration;
use toml::Table;

use crate::prompt::{get_interaction, prepare_request_interactive};
use crate::request::{build_client, do_request};

pub async fn monitor(
    file_path: &Path,
    delay_seconds: i32,
    env: &Table,
) -> Result<()> {
    if delay_seconds < 0 {
        bail!("Invalid delay");
    }

    let client = build_client()?;

    warn!("# Repeating every {delay_seconds} seconds, until interrupted...");

    let interaction = get_interaction();
    let req =
        prepare_request_interactive(file_path, env, interaction.as_ref())?;

    loop {
        let res = do_request(&client, &req).await;

        match res {
            Ok((res, elapsed)) => {
                let ts = chrono::Utc::now();
                println!("{}, {}, {:.2?}", ts, res.status().as_u16(), elapsed);
            }
            Err(e) => {
                eprintln!("{e}");
            }
        };

        sleep(Duration::from_secs(delay_seconds as u64)).await;
    }
}
