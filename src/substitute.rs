use crate::prompt::{fuzzy_match, is_interactive_mode};
use anyhow::{bail, Result};
use inquire::{list_option::ListOption, DateSelect, Select, Text};
use std::str;
use thiserror::Error;
use toml::{Table, Value};

#[derive(Error, Debug, Clone)]
pub enum SubstituteError {
    #[error("Replacement not found: {key}")]
    ReplacementNotFound { key: String },

    #[error("Replacement not selected: {key}\nSuggestions:\n{suggestions}")]
    ReplacementNotSelected { key: String, suggestions: String },

    #[error("Syntax error")]
    SyntaxError,

    #[error("Type not supported")]
    TypeNotSupported,
}

pub fn substitute(input: &str, env: &Table) -> Result<String> {
    let mut output = String::new();

    for line in input.lines() {
        output.push_str(&substitute_line(line, env)?);
        output.push('\n');
    }

    Ok(output)
}

fn substitute_line(line: &str, env: &Table) -> Result<String> {
    let mut output = String::new();
    let mut slice = line;
    loop {
        match slice.find("{{") {
            None => {
                if slice.contains("}}") {
                    bail!(SubstituteError::SyntaxError)
                }
                output.push_str(slice);
                break;
            }
            Some(pos) => {
                output.push_str(&slice[..pos]);
                slice = &slice[pos..];

                let Some(end) = slice.find("}}").map(|i| i + 2) else {
                    bail!(SubstituteError::SyntaxError);
                };

                let rep = find_replacement(&slice[2..end - 2], env)?;

                // Nested substitution
                let rep = substitute_line(&rep, env)?;
                output.push_str(&rep);

                slice = &slice[end..];
            }
        }
    }

    Ok(output)
}

// Only valid with ascii_alphabetic, ascii_digit or underscores in key name
fn valid_character(c: &char) -> bool {
    c.is_ascii_alphabetic() || c.is_ascii_digit() || *c == '_'
}

fn find_replacement(placeholder: &str, env: &Table) -> Result<String> {
    let mut parts = placeholder.split('|');

    let key = parts.next().unwrap_or("").trim();
    let parsed_key = key.chars().filter(valid_character).collect::<String>();

    let parse = |v: &str| key.replace(&parsed_key, v);

    match env.get(&parsed_key) {
        Some(Value::String(v)) => Ok(parse(v)),
        Some(Value::Integer(v)) => Ok(parse(&v.to_string())),
        Some(Value::Float(v)) => Ok(parse(&v.to_string())),
        Some(Value::Boolean(v)) => Ok(parse(&v.to_string())),
        Some(Value::Array(arr)) => {
            if is_interactive_mode() {
                Ok(select_replacement(&parsed_key, arr)?)
            } else {
                let suggestions: Vec<String> = arr
                    .iter()
                    .take(10)
                    .filter_map(|v| match (v.get("value"), v.get("name")) {
                        (Some(v), Some(n)) => Some(format!("{key}={v} => {n}")),
                        _ => None,
                    })
                    .collect();
                let suggestions = suggestions.join("\n");
                bail!(SubstituteError::ReplacementNotSelected {
                    key: parsed_key,
                    suggestions
                })
            }
        }
        Some(_) => bail!(SubstituteError::TypeNotSupported),
        None => {
            let fallback = parts.next().map(|fb| fb.trim());

            if is_interactive_mode() {
                prompt_user(&parsed_key, fallback)
            } else {
                fallback
                    .map(|f| f.to_string())
                    .ok_or(SubstituteError::ReplacementNotFound { key: parsed_key }.into())
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_env() -> Table {
        toml::from_str(
            r#"
            url = "example.com"
            token = "abc123"
            integer = 42
            float = 99.99
            boolean = true
            api_url1 = "foo.com"

            nested = "the answer is {{integer}}"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn returns_the_input_unchanged() {
        let env = create_env();
        let res = substitute("foo\nbar\n", &env).unwrap();

        assert_eq!(&res, "foo\nbar\n");
    }

    #[test]
    fn substitutes_single_variable() {
        let env = create_env();
        let res = substitute("foo {{url}}\nbar\n", &env).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitutes_integer() {
        let env = create_env();
        let res = substitute("foo={{integer}}", &env).unwrap();

        assert_eq!(&res, "foo=42\n");
    }

    #[test]
    fn substitutes_float() {
        let env = create_env();
        let res = substitute("foo={{float}}", &env).unwrap();

        assert_eq!(&res, "foo=99.99\n");
    }

    #[test]
    fn substitutes_boolean() {
        let env = create_env();
        let res = substitute("foo: {{boolean}}", &env).unwrap();

        assert_eq!(&res, "foo: true\n");
    }

    #[test]
    fn substitutes_placeholder_with_default_value() {
        let env = create_env();
        let res = substitute("foo: {{url | fallback.com}}\n", &env).unwrap();

        assert_eq!(&res, "foo: example.com\n");
    }

    #[test]
    fn substitutes_default_value() {
        let env = create_env();
        let res = substitute("foo: {{href | fallback.com }}\n", &env).unwrap();

        assert_eq!(&res, "foo: fallback.com\n");
    }

    #[test]
    fn substitutes_single_variable_with_speces() {
        let env = create_env();
        let res = substitute("foo {{ url  }}\nbar\n", &env).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitutes_one_variable_per_line() {
        let env = create_env();
        let res = substitute("foo {{url}}\nbar {{token}}\n", &env).unwrap();

        assert_eq!(&res, "foo example.com\nbar abc123\n");
    }

    #[test]
    fn substitutes_variable_on_the_same_line() {
        let env = create_env();
        let res = substitute("foo {{url}}, bar {{token}}\n", &env).unwrap();

        assert_eq!(&res, "foo example.com, bar abc123\n");
    }

    #[test]
    fn substitutes_nested_variable() {
        let env = create_env();
        let res = substitute("# {{nested}}!\n", &env).unwrap();

        assert_eq!(&res, "# the answer is 42!\n");
    }

    #[test]
    fn substitutes_only_characters_inside_quotes() {
        let env = create_env();
        let res = substitute("foo: {{ \"boolean\" }}", &env).unwrap();

        assert_eq!(&res, "foo: \"true\"\n");
    }

    #[test]
    fn substitutes_only_characters_inside_list() {
        let env = create_env();
        let res = substitute("foo: {{ [url] }}", &env).unwrap();

        assert_eq!(&res, "foo: [example.com]\n");
    }

    #[test]
    fn substitutes_only_characters_inside_list_inside_quotes() {
        let env = create_env();
        let res = substitute("foo: {{ [\"url\"] }}", &env).unwrap();

        assert_eq!(&res, "foo: [\"example.com\"]\n");
    }

    #[test]
    fn substitutes_variable_on_the_same_line_in_list() {
        let env = create_env();
        let res = substitute("foo: [{{ \"url\" }}, {{ \"integer\" }}]", &env).unwrap();

        assert_eq!(&res, "foo: [\"example.com\", \"42\"]\n");
    }

    #[test]
    fn substitutes_only_numbers_inside_quote() {
        let env = create_env();
        let res = substitute("foo: {{ \"integer\" }}", &env).unwrap();

        assert_eq!(&res, "foo: \"42\"\n");
    }

    #[test]
    fn substitutes_variable_with_underscore_and_number_in_name() {
        let env = create_env();
        let res = substitute("foo: {{ api_url1 }}", &env).unwrap();

        assert_eq!(&res, "foo: foo.com\n");
    }

    #[test]
    fn fails_for_unmatched_open() {
        let env = create_env();
        let res = substitute("foo {{url\n", &env);

        assert!(res.is_err())
    }

    #[test]
    fn fails_for_unmatched_close() {
        let env = create_env();
        let res = substitute("foo url}} bar\n", &env);

        assert!(res.is_err())
    }
}
