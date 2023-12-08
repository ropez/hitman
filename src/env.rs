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
        .filter(|k| !k.starts_with("_"))
        .filter(|k| config.get(*k).expect("key must exist").is_table())
        .map(|k| k.to_string())
        .collect();

    return Ok(keys);
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

    let config = read_and_merge_config(&root_dir)?;

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

    match read_toml(&file_path.with_extension("http.toml")).ok() {
        Some(content) => env.extend(content),
        None => (),
    }

    // FIXME state per environment
    match read_toml(&root_dir.join(DATA_FILE)).ok() {
        Some(content) => env.extend(content),
        None => (),
    }

    for (k, v) in options {
        env.insert(k.clone(), Value::String(v.clone()));
    }

    Ok(env)
}

pub fn update_env(vars: &TomlTable) -> Result<()> {
    if vars.is_empty() {
        return Ok(());
    }

    let content = fs::read_to_string(DATA_FILE).unwrap_or("".to_string());

    let mut state = toml::from_str::<TomlTable>(&content).unwrap_or(TomlTable::new());

    state.extend(vars.clone());
    fs::write(DATA_FILE, toml::to_string_pretty(&state)?)?;

    Ok(())
}

fn read_and_merge_config(root_dir: &Path) -> Result<TomlTable> {
    let mut config = TomlTable::new();

    config.extend(read_toml(&root_dir.join(CONFIG_FILE))?);

    if let Some(local) = read_toml(&root_dir.join(LOCAL_CONFIG_FILE)).ok() {
        config.extend(local);
    }

    Ok(config)
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
