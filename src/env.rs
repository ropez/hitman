use eyre::{bail, Result};
use inquire::Select;
use log::warn;
use std::env::current_dir;
use std::fs::{self, read_to_string};
use std::path::{Path, PathBuf};
use toml::{Table as TomlTable, Value};

use crate::prompt::fuzzy_match;

const CONFIG_FILE: &str = "hitman.toml";
const LOCAL_CONFIG_FILE: &str = "hitman.local.toml";
const TARGET_FILE: &str = ".hitman-target";
const DATA_FILE: &str = ".hitman-data.toml";

pub fn select_env(root_dir: &Path) -> Result<()> {
    let config = read_and_merge_config(root_dir)?;
    let items = find_environments(&config)?;

    let selected = Select::new("Select target", items.clone())
        .with_page_size(15)
        .with_filter(&|filter, _, value, _| fuzzy_match(filter, value))
        .prompt()?;

    fs::write(root_dir.join(TARGET_FILE), &selected)?;
    warn!("Target set to {}", &selected);

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

fn find_environments(config: &TomlTable) -> Result<Vec<String>> {
    let keys: Vec<String> = config
        .keys()
        .filter(|k| !k.starts_with('_'))
        .filter(|k| config.get(*k).expect("key must exist").is_table())
        .map(|k| k.to_string())
        .collect();

    Ok(keys)
}

pub fn load_env(
    root_dir: &Path,
    file_path: &Path,
    options: &Vec<(String, String)>,
) -> Result<TomlTable> {
    use Value::Table;

    let target = read_to_string(root_dir.join(TARGET_FILE))
        .map(|t| t.trim().to_string())
        .unwrap_or("default".to_string());

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

    if let Some(Table(t)) = config.get(&target) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_environments() {
        let config = toml::from_str(
            r#"
        global = "foo"

        [foo]
        value = "koko"

        [bar]

        [_default]
        fallback = "self"

        "#,
        )
        .unwrap();

        let envs = find_environments(&config).unwrap();

        assert_eq!(envs, vec!["bar", "foo"]);
    }
}
