use eyre::{bail, Result};
use inquire::{list_option::ListOption, Select};
use log::{error, info};
use notify::EventKind;
use request::{flurry_attack, make_request};
use std::env::current_dir;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use walkdir::WalkDir;

mod cli;
mod env;
mod logging;
mod request;
mod util;
mod watcher;
use env::{find_root_dir, load_env, select_env};
use watcher::Watcher;

mod extract;

mod substitute;

mod prompt;
use prompt::set_interactive_mode;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::parse_args();

    logging::init(args.verbose, args.quiet, args.flurry.is_some())?;

    set_interactive_mode(!(args.non_interactive || args.watch));

    let Some(root_dir) = find_root_dir()? else {
        bail!("No hitman.toml found");
    };

    if args.select {
        select_env(&root_dir)?;
        return Ok(());
    }

    let cwd = current_dir()?;

    let result = if let Some(file_path) = args.name {
        let file_path = cwd.join(file_path);

        if let Some(flurry_size) = args.flurry {
            let env = load_env(&root_dir, &file_path, &args.options)?;
            flurry_attack(
                &file_path,
                flurry_size,
                args.connections.unwrap_or(10),
                &env,
            )
            .await
        } else {
            let res = run_once(&root_dir, &file_path, &args.options).await;

            if args.watch {
                watch_mode(&root_dir, &file_path, &args.options).await
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
                .with_filter(&|filter, _, value, _| prompt::fuzzy_match(filter, value))
                .with_page_size(15)
                .prompt()?;

            let file_path = &files[selected.index];

            let result = run_once(&root_dir, file_path, &args.options).await;

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

    // FIXME Must be a way to make this nicer
    match &result {
        Err(e) => {
            if is_user_cancelation(e) {
                Ok(())
            } else {
                result
            }
        }
        _ => result,
    }
}

fn is_user_cancelation(err: &eyre::Report) -> bool {
    use inquire::InquireError::*;
    false
        || matches!(err.downcast_ref(), Some(OperationCanceled))
        || matches!(err.downcast_ref(), Some(OperationInterrupted))
}

fn find_available_requests(cwd: &Path) -> Result<Vec<PathBuf>> {
    let files: Vec<_> = WalkDir::new(cwd)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.ends_with(".http"))
                .unwrap_or(false)
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

async fn run_once(root_dir: &Path, file_path: &Path, options: &[(String, String)]) -> Result<()> {
    let env = load_env(root_dir, file_path, options)?;

    make_request(file_path, &env).await
}

async fn watch_mode(root_dir: &Path, file_path: &Path, options: &[(String, String)]) -> Result<()> {
    let (tx, mut rx) = mpsc::channel(1);

    let paths = env::watch_list(root_dir, file_path);
    let mut watcher = Watcher::new(tx, paths)?;

    watcher.watch_all()?;

    loop {
        info!("# Watching for changes...");
        if let Some(event) = rx.recv().await {
            if let EventKind::Modify(_) = event.kind {
                watcher.unwatch_all()?;
                if let Err(err) = run_once(root_dir, file_path, options).await {
                    error!("# {}", err)
                }
                watcher.watch_all()?;
            }
        }
    }
}
