use anyhow::Result;
use hitman::resolve::{resolve_path, Resolved};
use inquire::{list_option::ListOption, Select};
use log::{error, info};
use notify::EventKind;
use std::env::current_dir;
use std::path::Path;
use tokio::sync::mpsc;

use hitman::env::{
    find_available_requests, find_root_dir, get_target, load_env,
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

    let root_dir = find_root_dir()?;

    if let Some(arg) = args.select {
        match arg {
            Some(target) => set_target(&root_dir, &target)?,
            None => select_target(&root_dir)?,
        }
        return Ok(());
    }

    let target = args.target.unwrap_or_else(|| get_target(&root_dir));

    let cwd = current_dir()?;

    let result = if let Some(file_path) = args.name {
        let file_path = cwd.join(file_path);
        let resolved = resolve_path(&file_path)?;

        if let Some(flurry_size) = args.flurry {
            let scope = load_env(&root_dir, &target, &resolved, &args.options)?;
            flurry_attack(
                &resolved,
                flurry_size,
                args.connections.unwrap_or(10),
                &scope,
            )
            .await
        } else if let Some(delay_seconds) = args.monitor {
            let scope = load_env(&root_dir, &target, &resolved, &args.options)?;
            monitor(&resolved, delay_seconds, &scope).await
        } else {
            let res =
                run_once(&root_dir, &target, &resolved, &args.options).await;

            if args.watch {
                watch_mode(&root_dir, &target, &resolved, &args.options).await
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

            let result =
                run_once(&root_dir, &target, &resolved, &args.options).await;

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
    root_dir: &Path,
    target: &str,
    resolved: &Resolved,
    options: &[(String, String)],
) -> Result<()> {
    let scope = load_env(root_dir, target, resolved, options)?;

    make_request(resolved, &scope).await
}

async fn watch_mode(
    root_dir: &Path,
    target: &str,
    resolved: &Resolved,
    options: &[(String, String)],
) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(1);

    let paths = watch_list(root_dir, resolved);
    let mut watcher = Watcher::new(tx, paths)?;

    watcher.watch_all()?;

    loop {
        info!("# Watching for changes...");
        if let Some(event) = rx.recv().await {
            if let EventKind::Modify(_) = event.kind {
                watcher.unwatch_all()?;
                if let Err(err) =
                    run_once(root_dir, target, resolved, options).await
                {
                    error!("# {err}");
                }
                watcher.watch_all()?;
            }
        }
    }
}
