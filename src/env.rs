use std::fs::{self, read_to_string};
use eyre::{Result, eyre};
use toml::{Table as TomlTable, Value};
use dialoguer::{FuzzySelect, theme::ColorfulTheme};

const CONFIG_FILE: &str = "hittup.toml";
const TARGET_FILE: &str = ".hittup-target";
const STATE_FILE: &str = ".hittup-state.toml";

pub fn select_env() -> Result<()> {
    let items = find_environments(&read_toml(CONFIG_FILE)?)?;

    // Alternative crate: inquire

    let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt("Select environment")
        .items(&items)
        .interact()?;

    fs::write(TARGET_FILE, &items[selection])?;
    eprintln!("Current environment is now {}", items[selection]);

    Ok(())
}

fn find_environments(config: &TomlTable) -> Result<Vec<String>> {
    let keys: Vec<String> = config.keys()
        .filter(|k| !k.starts_with("_"))
        .filter(|k| config.get(*k).expect("key must exist").is_table())
        .map(|k| k.to_string()).collect();

    return Ok(keys);
}

pub fn load_env(file_path: &str) -> Result<TomlTable> {
    use Value::Table;

    let target = read_to_string(TARGET_FILE)
        .map(|t| t.trim().to_string())
        .unwrap_or("default".to_string());

    let mut env = TomlTable::new();

    // FIXME Search from file_path, traverse upwards

    let config = read_toml(CONFIG_FILE)?;

    // Global defaults
    env.extend(config.clone()
        .into_iter().filter(|(_, v)| !v.is_table())
        .collect::<Vec<_>>());

    if let Some(Table(t)) = config.get(&target) {
        env.extend(t.clone());
    } else {
        return Err(eyre!("`{}` not found in config", target));
    }

    match read_toml(&format!("{}.toml", file_path)).ok() {
        Some(content) => env.extend(content),
        None => (),
    }

    // FIXME state per environment
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_environments() {
        let config = toml::from_str(r#"
        global = "foo"

        [foo]
        value = "koko"

        [bar]

        [_default]
        fallback = "self"

        "#).unwrap();

        let envs = find_environments(&config).unwrap();

        assert_eq!(envs, vec!["bar", "foo"]);
    }
}

