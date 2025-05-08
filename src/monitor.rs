use anyhow::{bail, Result};
use log::warn;
use tokio::time::sleep;

use std::time::Duration;

use crate::{
    prompt::{get_interaction, prepare_request_interactive},
    request::{build_client, do_request},
    resolve::Resolved,
    scope::Scope,
};

pub async fn monitor(
    resolved: &Resolved,
    delay_seconds: i32,
    scope: &Scope,
) -> Result<()> {
    let Ok(delay) = u64::try_from(delay_seconds) else {
        bail!("Invalid delay");
    };

    let client = build_client(&resolved.root_dir)?;

    warn!("# Repeating every {delay} seconds, until interrupted...");

    let interaction = get_interaction();
    let req =
        prepare_request_interactive(resolved, scope, interaction.as_ref())?;

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
        }

        sleep(Duration::from_secs(delay)).await;
    }
}
