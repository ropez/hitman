use std::fs::{self, read_to_string};
use eyre::{Result, eyre};
use toml::Table as TomlTable;

// - Should use globals in toml by default (don't enforce multiple environments)
// - Should use globals as fallback when specifying an environment (instead of _default)
// - Rename "target" so something else, like current environment

// - Provide --select options to fuzzy search environments
// - Show '*' messages about variable extraction
// - Support collection multiple variables from arrays
// - Prompt user for missing substitution values
// - Prompt user with fuzzy search when we have multiple values
// - Prompt user for text input for missing values
// √ Support fallback values in placeholders

const CONFIG_FILE: &str = "hittup.toml";
const TARGET_FILE: &str = ".hittup-target";
const STATE_FILE: &str = ".hittup-state.toml";

pub fn load_env(file_path: &str) -> Result<TomlTable> {
    use toml::Value::Table;

    let target = read_to_string(TARGET_FILE)
        .map(|t| t.trim().to_string())
        .unwrap_or("default".to_string());

    let mut env = TomlTable::new();

    // FIXME Search from file_path, traverse upwards

    let config = read_toml(CONFIG_FILE)?;

    if let Some(v) = config.get("_default") {
        if let Table(t) = v {
            env.extend(t.clone());
        } else {
            return Err(eyre!("`_default` has unexpected type in config"));
        }
    }

    if let Some(Table(t)) = config.get(&target) {
        env.extend(t.clone());
    } else {
        return Err(eyre!("`{}` not found in config", target));
    }

    match read_toml(&format!("{}.toml", file_path)).ok() {
        Some(content) => env.extend(content),
        None => (),
    }

    match read_toml(STATE_FILE).ok() {
        Some(content) => env.extend(content),
        None => (),
    }

    Ok(env)
}

pub fn update_env(vars: &TomlTable) -> Result<()> {
    if vars.is_empty() { 
        return Ok(()); 
    }

    let content = fs::read_to_string(STATE_FILE).unwrap_or("".to_string());

    let mut state = toml::from_str::<TomlTable>(&content).unwrap_or(TomlTable::new());

    state.extend(vars.clone());
    fs::write(STATE_FILE, toml::to_string_pretty(&state)?)?;

    Ok(())
}


fn read_toml(file_path: &str) -> Result<TomlTable> {
    let content = fs::read_to_string(file_path)?;

    let cfg = toml::from_str::<TomlTable>(&content)?;

    Ok(cfg)
}

