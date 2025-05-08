use anyhow::{Context, Result};
use hitman::resolve::{find_root_dir, resolve_path, Resolved};
use inquire::{list_option::ListOption, Select};
use log::{error, info};
use notify::EventKind;
use std::env::current_dir;
use tokio::sync::mpsc;

use hitman::env::{
    find_available_requests, get_target, load_env,
    select_target, set_target, watch_list,
};
use hitman::flurry::flurry_attack;
use hitman::monitor::monitor;
use hitman::prompt::{fuzzy_match, set_interactive_mode};
use hitman::request::make_request;

use watcher::Watcher;

mod cli;
mod logging;
mod watcher;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::parse_args();

    logging::init(
        args.verbose,
        args.quiet,
        args.flurry.is_some() || args.monitor.is_some(),
    )?;

    set_interactive_mode(!(args.non_interactive || args.watch));

    if let Some(arg) = args.select {
        let root_dir = find_root_dir(&current_dir()?)?.context("No hitman.toml found")?;

        match arg {
            Some(target) => set_target(&root_dir, &target)?,
            None => select_target(&root_dir)?,
        }
        return Ok(());
    }

    let cwd = current_dir()?;

    let result = if let Some(file_path) = args.name {
        let file_path = cwd.join(file_path);
        let resolved = resolve_path(&file_path)?;

        let target = args.target.clone().unwrap_or_else(|| get_target(&resolved.root_dir));

        if let Some(flurry_size) = args.flurry {
            let scope = load_env(&target, &resolved, &args.options)?;
            flurry_attack(
                &resolved,
                flurry_size,
                args.connections.unwrap_or(10),
                &scope,
            )
            .await
        } else if let Some(delay_seconds) = args.monitor {
            let scope = load_env(&target, &resolved, &args.options)?;
            monitor(&resolved, delay_seconds, &scope).await
        } else {
            let res =
                run_once(&target, &resolved, &args.options).await;

            if args.watch {
                watch_mode(&target, &resolved, &args.options).await
            } else {
                res
            }
        }
    } else {
        loop {
            let files = find_available_requests(&cwd)?;
            let options: Vec<ListOption<String>> = files
                .iter()
                .enumerate()
                .map(|(i, p)| ListOption::new(i, p.display().to_string()))
                .collect::<Vec<_>>();

            eprintln!();
            let selected = Select::new("Select request", options)
                .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
                .with_page_size(15)
                .prompt()?;

            let file_path = &files[selected.index];
            let resolved = resolve_path(file_path)?;
            let target = args.target.clone().unwrap_or_else(|| get_target(&resolved.root_dir));

            let result =
                run_once(&target, &resolved, &args.options).await;

            if !args.repeat {
                break result;
            }

            match result {
                Ok(()) => (),
                Err(e) => {
                    if !is_user_cancelation(&e) {
                        error!("{}", e);
                    }
                }
            }
        }
    };

    result.or_else(|e| {
        if is_user_cancelation(&e) {
            Ok(())
        } else {
            Err(e)
        }
    })
}

fn is_user_cancelation(err: &anyhow::Error) -> bool {
    use inquire::InquireError::*;
    matches!(
        err.downcast_ref(),
        Some(OperationCanceled | OperationInterrupted)
    )
}

async fn run_once(
    target: &str,
    resolved: &Resolved,
    options: &[(String, String)],
) -> Result<()> {
    let scope = load_env(target, resolved, options)?;

    make_request(resolved, &scope).await
}

async fn watch_mode(
    target: &str,
    resolved: &Resolved,
    options: &[(String, String)],
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(1);

    let paths = watch_list(&resolved.root_dir, resolved);
    let mut watcher = Watcher::new(tx, paths)?;

    watcher.watch_all()?;

    loop {
        info!("# Watching for changes...");
        if let Some(event) = rx.recv().await {
            if let EventKind::Modify(_) = event.kind {
                watcher.unwatch_all()?;
                if let Err(err) =
                    run_once(target, resolved, options).await
                {
                    error!("# {err}");
                }
                watcher.watch_all()?;
            }
        }
    }
}
