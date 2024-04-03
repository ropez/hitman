use crate::{substitute::{SubstituteError, UserInteraction}, prompt::fuzzy_match};
use anyhow::{bail, Result};
use inquire::{DateSelect, Text, list_option::ListOption, Select};
use toml::Value;

pub struct NoUserInteraction;

impl UserInteraction for NoUserInteraction {
    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<String> {
        fallback
            .map(|f| f.to_string())
            .ok_or(SubstituteError::ReplacementNotFound { key: key.into() }.into())
    }

    fn select(&self, key: &str, values: &[toml::Value]) -> Result<String> {
        let suggestions: Vec<String> = values
            .iter()
            .take(10)
            .filter_map(|v| match (v.get("value"), v.get("name")) {
                (Some(v), Some(n)) => Some(format!("{key}={v} => {n}")),
                _ => None,
            })
            .collect();
        let suggestions = suggestions.join("\n");
        bail!(SubstituteError::ReplacementNotSelected {
            key: key.into(),
            suggestions
        })
    }
}

pub struct CliUserInteraction;

impl UserInteraction for CliUserInteraction {
    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<String> {
        prompt_user(key, fallback)
    }

    fn select(&self, key: &str, values: &[toml::Value]) -> Result<String> {
        select_replacement(key, values)
    }
}

fn prompt_user(key: &str, fallback: Option<&str>) -> Result<String> {
    let fb = fallback.unwrap_or("");

    if key.ends_with("_date") || key.ends_with("Date") {
        if let Some(date) = prompt_for_date(key)? {
            return Ok(date);
        }
    }

    let input = Text::new(&format!("Enter value for {}", key))
        .with_default(fb)
        .prompt()?;

    Ok(input)
}

fn prompt_for_date(key: &str) -> Result<Option<String>> {
    let msg = format!("Select a date for {}", key);
    let formatter = |date: chrono::NaiveDate| date.format("%Y-%m-%d").to_string();

    let res = DateSelect::new(&msg)
        .with_week_start(chrono::Weekday::Mon)
        .with_formatter(&formatter)
        .prompt_skippable()?;

    Ok(res.map(formatter))
}

fn select_replacement(key: &str, values: &[Value]) -> Result<String> {
    let list_options: Vec<ListOption<String>> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            ListOption::new(
                i,
                match v {
                    Value::Table(t) => match t.get("name") {
                        Some(Value::String(value)) => value.clone(),
                        Some(value) => value.to_string(),
                        None => t.to_string(),
                    },
                    other => other.to_string(),
                },
            )
        })
        .collect();

    let selected = Select::new(&format!("Select value for {}", key), list_options.clone())
        .with_filter(&|filter, _, value, _| fuzzy_match(filter, value))
        .with_page_size(15)
        .prompt()?;

    match &values[selected.index] {
        Value::Table(t) => match t.get("value") {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(value) => Ok(value.to_string()),
            _ => bail!(SubstituteError::ReplacementNotFound { key: key.into() }),
        },
        other => Ok(other.to_string()),
    }
}
