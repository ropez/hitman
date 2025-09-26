use anyhow::{bail, Result};
use fuzzy_matcher::skim::SkimMatcherV2;
use inquire::{list_option::ListOption, DateSelect, MultiSelect, Select, Text};
use std::{collections::HashMap, env, string::ToString};
use toml::Value;

use crate::{
    request::HitmanRequest,
    resolve::Resolved,
    scope::{Replacement, Scope},
    substitute::{
        prepare_request,
        Substitution::{Complete, ValueMissing},
        SubstitutionValue,
    },
};

fn set_boolean(name: &str, value: bool) {
    env::set_var(name, if value { "y" } else { "n" });
}

fn get_boolean(name: &str) -> bool {
    env::var(name).is_ok_and(|v| v == "y")
}

pub fn set_interactive_mode(enable: bool) {
    set_boolean("interactive", enable);
}

pub fn is_interactive_mode() -> bool {
    get_boolean("interactive")
}

pub fn fuzzy_match(filter: &str, value: &str) -> Option<i64> {
    let matcher = SkimMatcherV2::default();
    let fuzzy_score = matcher.fuzzy(value, filter, true);
    fuzzy_score.map(|(score, _)| score)
}

pub fn get_interaction() -> Box<dyn UserInteraction> {
    if is_interactive_mode() {
        Box::new(CliUserInteraction)
    } else {
        Box::new(NoUserInteraction)
    }
}

pub trait UserInteraction {
    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<String>;
    fn select(&self, key: &str, values: &[Value]) -> Result<String>;
    fn select_multiple(
        &self,
        key: &str,
        values: &[Value],
    ) -> Result<Vec<String>>;
}

pub fn prepare_request_interactive<I>(
    resolved: &Resolved,
    scope: &Scope,
    interaction: &I,
) -> Result<HitmanRequest>
where
    I: UserInteraction + ?Sized,
{
    let mut vars = HashMap::new();

    loop {
        match prepare_request(resolved, &vars)? {
            Complete(req) => return Ok(req),
            ValueMissing {
                key,
                fallback,
                multiple,
            } => {
                let value = match scope.lookup(&key)? {
                    Replacement::Value(value) => {
                        SubstitutionValue::Single(value)
                    }
                    Replacement::ValueNotFound { key } => {
                        SubstitutionValue::Single(
                            interaction.prompt(&key, fallback.as_deref())?,
                        )
                    }
                    Replacement::MultipleValuesFound { key, values } => {
                        if multiple {
                            SubstitutionValue::Multiple(
                                interaction.select_multiple(&key, &values)?,
                            )
                        } else {
                            SubstitutionValue::Single(
                                interaction.select(&key, &values)?,
                            )
                        }
                    }
                };

                vars.insert(key, value);
            }
        }
    }
}

pub struct NoUserInteraction;

impl NoUserInteraction {
    fn get_suggestions(&self, key: &str, values: &[toml::Value]) -> String {
        values
            .iter()
            .take(10)
            .filter_map(|v| match (v.get("value"), v.get("name")) {
                (Some(v), Some(n)) => Some(format!("{key}={v} => {n}")),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl UserInteraction for NoUserInteraction {
    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<String> {
        if let Some(val) = fallback.map(ToString::to_string) {
            return Ok(val);
        }

        bail!("Replacement not found: {key}");
    }

    fn select(&self, key: &str, values: &[toml::Value]) -> Result<String> {
        let suggestions = self.get_suggestions(key, values);
        bail!("Replacement not selected: {key}\nSuggestions:\n{suggestions}");
    }

    fn select_multiple(
        &self,
        key: &str,
        values: &[toml::Value],
    ) -> Result<Vec<String>> {
        let suggestions = self.get_suggestions(key, values);
        bail!("Replacement not selected: {key}\nSuggestions:\n{suggestions}");
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

    fn select_multiple(
        &self,
        key: &str,
        values: &[Value],
    ) -> Result<Vec<String>> {
        select_replacement_multiple(key, values)
    }
}

fn prompt_user(key: &str, fallback: Option<&str>) -> Result<String> {
    let fb = fallback.unwrap_or("");

    if key.ends_with("_date") || key.ends_with("Date") {
        if let Some(date) = prompt_for_date(key)? {
            return Ok(date);
        }
    }

    let input = Text::new(&format!("Enter value for {key}"))
        .with_default(fb)
        .prompt()?;

    Ok(input)
}

fn prompt_for_date(key: &str) -> Result<Option<String>> {
    let msg = format!("Select a date for {key}");
    let formatter =
        |date: chrono::NaiveDate| date.format("%Y-%m-%d").to_string();

    let res = DateSelect::new(&msg)
        .with_week_start(chrono::Weekday::Mon)
        .with_formatter(&formatter)
        .prompt_skippable()?;

    Ok(res.map(formatter))
}

fn select_replacement(key: &str, values: &[Value]) -> Result<String> {
    let list_options = values_to_list_options(values);
    let selected =
        Select::new(&format!("Select value for {key}"), list_options)
            .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
            .with_page_size(15)
            .prompt()?;

    list_option_to_string(key, values, &selected)
}

fn select_replacement_multiple(
    key: &str,
    values: &[Value],
) -> Result<Vec<String>> {
    let list_options = values_to_list_options(values);
    let selected =
        MultiSelect::new(&format!("Select value for {key}"), list_options)
            .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
            .with_page_size(15)
            .prompt()?;

    selected
        .iter()
        .map(|item| list_option_to_string(key, values, item))
        .collect()
}

fn values_to_list_options(values: &[Value]) -> Vec<ListOption<String>> {
    values
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
        .collect()
}

fn list_option_to_string(
    key: &str,
    values: &[Value],
    selected: &ListOption<String>,
) -> Result<String> {
    match &values[selected.index] {
        Value::Table(t) => match t.get("value") {
            Some(Value::String(value)) => Ok(value.clone()),
            Some(value) => Ok(value.to_string()),
            _ => bail!("Replacement not found: {key}"),
        },
        other => Ok(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_true_for_identical() {
        assert!(fuzzy_match("a", "a").is_some());
    }

    #[test]
    fn returns_false_for_different() {
        assert!(fuzzy_match("a", "b").is_none());
    }

    #[test]
    fn returns_false_for_different_length() {
        assert!(fuzzy_match("ab", "a").is_none());
    }

    #[test]
    fn returns_true_for_different_case() {
        assert!(fuzzy_match("a", "A").is_some());
    }

    #[test]
    fn returns_true_if_filter_is_empty() {
        assert!(fuzzy_match("", "a").is_some());
    }

    #[test]
    fn returns_false_if_value_is_empty() {
        assert!(fuzzy_match("a", "").is_none());
    }

    #[test]
    fn returns_true_value_contains_filter() {
        assert!(fuzzy_match("a", "ab").is_some());
    }

    #[test]
    fn returns_true_if_value_contains_all_letters_in_filter_in_the_same_order()
    {
        assert!(fuzzy_match("abc", "uaaxbycz").is_some());
    }
}
