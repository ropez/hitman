use anyhow::{bail, Result};
use inquire::Select;
use log::warn;
use reqwest::cookie::CookieStore;
use reqwest::Url;
use std::env::current_dir;
use std::fs::{self, read_to_string};
use std::path::{Path, PathBuf};
use toml::{Table as TomlTable, Value};
use walkdir::WalkDir;

use crate::prompt::fuzzy_match;

const CONFIG_FILE: &str = "hitman.toml";
const LOCAL_CONFIG_FILE: &str = "hitman.local.toml";
const TARGET_FILE: &str = ".hitman-target";
const DATA_FILE: &str = ".hitman-data.toml";

const COOKIE_KEY: &str = "Cookies";
pub struct HitmanCookieJar;

impl CookieStore for HitmanCookieJar {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        _: &Url,
    ) {
        let cookies = cookie_headers
            .filter_map(|c| {
                let s = std::str::from_utf8(c.as_bytes()).ok()?;
                Some(Value::String(s.to_string()))
            })
            .collect::<Vec<_>>();

        let mut out = TomlTable::new();
        out.insert(COOKIE_KEY.to_string(), Value::Array(cookies));

        let _ = update_data(&out);
    }

    fn cookies(&self, _: &Url) -> Option<reqwest::header::HeaderValue> {
        let root_dir = find_root_dir().ok()??;
        let data_file = read_toml(&root_dir.join(DATA_FILE)).ok()?;

        match data_file.get(COOKIE_KEY)? {
            Value::Array(arr) => {
                let headers = arr
                    .iter()
                    .filter_map(|it| cookie::Cookie::parse(it.as_str()?).ok())
                    .map(|cookie| {
                        format!("{}={}", cookie.name(), cookie.value())
                    })
                    .collect::<Vec<_>>()
                    .join("; ");

                reqwest::header::HeaderValue::from_str(&headers).ok()
            }
            _ => None,
        }
    }
}

pub fn select_env(root_dir: &Path) -> Result<()> {
    let items = find_environments(root_dir)?;

    let selected = Select::new("Select target", items.clone())
        .with_page_size(15)
        .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
        .prompt()?;

    set_target(root_dir, &selected)?;

    Ok(())
}

pub fn set_target(root_dir: &Path, selected: &str) -> Result<()> {
    fs::write(root_dir.join(TARGET_FILE), selected)?;
    warn!("Target set to {}", selected);

    Ok(())
}

// The root dir is where we find hitman.toml,
// scanning parent directories until we find it
pub fn find_root_dir() -> Result<Option<PathBuf>> {
    let mut dir = current_dir()?;
    let res = loop {
        if dir.join(CONFIG_FILE).exists() {
            break Some(dir);
        }
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        } else {
            break None;
        }
    };

    Ok(res)
}

pub fn find_environments(root_dir: &Path) -> Result<Vec<String>> {
    let config = read_and_merge_config(root_dir)?;
    let keys: Vec<String> = config
        .keys()
        .filter(|k| !k.starts_with('_'))
        .filter(|k| config.get(*k).expect("key must exist").is_table())
        .map(|k| k.to_string())
        .collect();

    Ok(keys)
}

/// Get all files to watch for changes in watch mode.
///
/// This includes all files used by the request, except the data file.
/// Trying to watch the data file just causes loops.
pub fn watch_list(root_dir: &Path, file_path: &Path) -> Vec<PathBuf> {
    vec![
        file_path.into(),
        file_path.with_extension("http.toml"),
        root_dir.join(TARGET_FILE),
        root_dir.join(CONFIG_FILE),
        root_dir.join(LOCAL_CONFIG_FILE),
    ]
}

pub fn load_env(
    root_dir: &Path,
    target: &str,
    file_path: &Path,
    options: &[(String, String)],
) -> Result<TomlTable> {
    use Value::Table;

    let mut env = TomlTable::new();

    let config = read_and_merge_config(root_dir)?;

    // Global defaults
    env.extend(
        config
            .clone()
            .into_iter()
            .filter(|(_, v)| !v.is_table())
            .collect::<Vec<_>>(),
    );

    if let Some(Table(t)) = config.get(target) {
        env.extend(t.clone());
    } else {
        bail!("`{}` not found in config", target);
    }

    if let Ok(content) = read_toml(&file_path.with_extension("http.toml")) {
        env.extend(content)
    }

    // FIXME state per environment
    if let Ok(content) = read_toml(&root_dir.join(DATA_FILE)) {
        env.extend(content)
    }

    // Extra values passed on the command line
    for (k, v) in options {
        env.insert(k.clone(), Value::String(v.clone()));
    }

    Ok(env)
}

pub fn get_target(root_dir: &Path) -> String {
    let target = read_to_string(root_dir.join(TARGET_FILE))
        .map(|t| t.trim().to_string())
        .unwrap_or("default".to_string());
    target
}

pub fn update_data(vars: &TomlTable) -> Result<()> {
    if vars.is_empty() {
        return Ok(());
    }

    let root_dir = find_root_dir()?;
    let Some(root_dir) = root_dir else {
        bail!("Could not find project root");
    };
    let data_file = root_dir.join(DATA_FILE);

    let content = fs::read_to_string(&data_file).unwrap_or("".to_string());

    let mut state = toml::from_str::<TomlTable>(&content).unwrap_or_default();

    state.extend(vars.clone());
    fs::write(&data_file, toml::to_string_pretty(&state)?)?;

    Ok(())
}

fn read_and_merge_config(root_dir: &Path) -> Result<TomlTable> {
    let mut config = TomlTable::new();

    merge(&mut config, read_toml(&root_dir.join(CONFIG_FILE))?);

    if let Ok(local) = read_toml(&root_dir.join(LOCAL_CONFIG_FILE)) {
        merge(&mut config, local);
    }

    Ok(config)
}

/// Merge Toml tables recursively, merging child tables into
/// existing child tables.
fn merge(config: &mut TomlTable, other: TomlTable) {
    other.into_iter().for_each(move |(k, v)| {
        match v {
            Value::Table(t) => {
                let cur = config.get_mut(&k);
                if let Some(Value::Table(ref mut ext)) = cur {
                    merge(ext, t);
                } else {
                    config.insert(k, Value::Table(t));
                };
            }
            _ => {
                config.insert(k, v);
            }
        };
    });
}

fn read_toml(file_path: &Path) -> Result<TomlTable> {
    let content = fs::read_to_string(file_path)?;

    let cfg = toml::from_str::<TomlTable>(&content)?;

    Ok(cfg)
}

pub fn find_available_requests(cwd: &Path) -> Result<Vec<PathBuf>> {
    let files: Vec<_> = WalkDir::new(cwd)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| {
                    // Ignore special _graphql.http file
                    s != "_graphql.http"
                        && (s.ends_with(".http")
                            || s.ends_with(".gql")
                            || s.ends_with(".graphql"))
                })
                .unwrap_or(false)
        })
        .map(|p| {
            // Convert to relative path, based on depth
            let components: Vec<_> = p.path().components().collect();
            let relative_components: Vec<_> = components
                [(components.len() - p.depth())..]
                .iter()
                .map(|c| c.as_os_str())
                .collect();

            PathBuf::from_iter(&relative_components)
        })
        .collect();

    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! toml {
        ($($tt:tt)*) => {
            toml::from_str($($tt)*).unwrap()
        }
    }

    #[test]
    fn test_find_environments() {
        let config: PathBuf = toml! {
        r#"
            global = "foo"

            [foo]
            value = "koko"

            [bar]

            [_default]
            fallback = "self"
        "#
        };

        let envs = find_environments(&config).unwrap();

        assert_eq!(envs, vec!["bar", "foo"]);
    }

    #[test]
    fn merges_mested_tables() {
        let shared = toml! {
        r#"
            global0 = "shared_global0"
            global1 = "shared_global1"

            [thing]
            thing0 = "shared_thing0"
            thing1 = "shared_thing1"
        "#
        };

        let private = toml! {
        r#"
            global0 = "private_global0"
            global2 = "private_global2"

            [thing]
            thing0 = "private_thing0"
            thing2 = "private_thing2"
        "#
        };

        let mut merged = TomlTable::new();
        merge(&mut merged, shared);
        merge(&mut merged, private);

        let expected = toml! {
        r#"
            global0 = "private_global0"
            global1 = "shared_global1"
            global2 = "private_global2"

            [thing]
            thing0 = "private_thing0"
            thing1 = "shared_thing1"
            thing2 = "private_thing2"
        "#
        };

        assert_eq!(merged, expected);
    }
}
