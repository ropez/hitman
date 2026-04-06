use anyhow::{Result, bail};
use fuzzy_matcher::skim::SkimMatcherV2;
use inquire::{DateSelect, MultiSelect, Select, Text, list_option::ListOption};
use minijinja::Value as JinjaValue;
use std::{
    collections::HashMap,
    env,
    string::ToString,
    sync::{Arc, RwLock},
};
use toml::Value;

use crate::{
    scope::{Replacement, Scope},
    substitute::{SubstituteProvider, SubstituteValue},
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

pub fn get_interaction(
    scope: Scope,
) -> Arc<dyn SubstituteProvider + Send + Sync + 'static> {
    if is_interactive_mode() {
        Arc::new(CliUserInteraction::new(scope))
    } else {
        Arc::new(NoUserInteraction::new(scope))
    }
}

pub struct NoUserInteraction {
    scope: Scope,
}

impl NoUserInteraction {
    pub fn new(scope: Scope) -> Self {
        Self { scope }
    }

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

impl SubstituteProvider for NoUserInteraction {
    fn lookup_value(&self, key: &str) -> Option<SubstituteValue> {
        match self.scope.lookup(key).ok()? {
            Replacement::ValueNotFound { .. } => None,
            Replacement::Value(v) => {
                Some(SubstituteValue::Single(JinjaValue::from(v)))
            }
            Replacement::MultipleValuesFound { key: _, values } => {
                Some(SubstituteValue::Multiple(values))
            }
        }
    }

    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<JinjaValue> {
        if let Some(val) = fallback.map(ToString::to_string) {
            return Ok(JinjaValue::from(val));
        }
        bail!("Replacement not found: {key}");
    }

    fn select_single(
        &self,
        key: &str,
        values: &[toml::Value],
    ) -> Result<JinjaValue> {
        let suggestions = self.get_suggestions(key, values);
        bail!("Replacement not selected: {key}\nSuggestions:\n{suggestions}");
    }

    fn select_multiple(
        &self,
        key: &str,
        values: &[toml::Value],
    ) -> Result<Vec<JinjaValue>> {
        let suggestions = self.get_suggestions(key, values);
        bail!("Replacement not selected: {key}\nSuggestions:\n{suggestions}");
    }
}

pub struct CliUserInteraction {
    scope: Scope,
    vars: RwLock<HashMap<String, SubstituteValue>>,
}

impl CliUserInteraction {
    pub fn new(scope: Scope) -> Self {
        Self {
            scope,
            vars: Default::default(),
        }
    }
}

impl SubstituteProvider for CliUserInteraction {
    fn lookup_value(&self, key: &str) -> Option<SubstituteValue> {
        if let Some(v) = self.vars.read().unwrap().get(key) {
            return Some(v.clone());
        }

        match self.scope.lookup(key).ok()? {
            Replacement::ValueNotFound { .. } => None,
            Replacement::Value(v) => {
                Some(SubstituteValue::Single(JinjaValue::from(v)))
            }
            Replacement::MultipleValuesFound { key: _, values } => {
                Some(SubstituteValue::Multiple(values))
            }
        }
    }

    fn prompt(&self, key: &str, fallback: Option<&str>) -> Result<JinjaValue> {
        let val = prompt_user(key, fallback)?;
        let value = JinjaValue::from(val);

        let mut vars_mut = self.vars.write().unwrap();
        vars_mut
            .insert(key.to_string(), SubstituteValue::Single(value.clone()));
        Ok(value)
    }

    fn select_single(
        &self,
        key: &str,
        values: &[toml::Value],
    ) -> Result<JinjaValue> {
        select_replacement(key, values)
    }

    fn select_multiple(
        &self,
        key: &str,
        values: &[toml::Value],
    ) -> Result<Vec<JinjaValue>> {
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

fn select_replacement(key: &str, values: &[Value]) -> Result<JinjaValue> {
    let list_options = values_to_list_options(values);
    let selected =
        Select::new(&format!("Select value for {key}"), list_options)
            .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
            .with_page_size(15)
            .prompt()?;

    Ok(JinjaValue::from(list_option_to_string(
        key, values, &selected,
    )?))
}

fn select_replacement_multiple(
    key: &str,
    values: &[Value],
) -> Result<Vec<JinjaValue>> {
    let list_options = values_to_list_options(values);
    let selected =
        MultiSelect::new(&format!("Select value for {key}"), list_options)
            .with_scorer(&|filter, _, value, _| fuzzy_match(filter, value))
            .with_page_size(15)
            .prompt()?;

    let values = selected
        .iter()
        .map(|item| {
            list_option_to_string(key, values, item).map(JinjaValue::from)
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(values)
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
