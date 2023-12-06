use std::str;
use eyre::{Result, bail};
use toml::{Table, Value};
use derive_more::{Display, Error};
use dialoguer::{Input, theme::ColorfulTheme, FuzzySelect};
use inquire::DateSelect;
use crate::prompt::is_interactive_mode;

#[derive(Display, Error, Debug, Clone)]
pub enum SubstituteError {
    #[display(fmt = "Could not find replacement")]
    ReplacementNotFound,

    #[display(fmt = "Syntax error")]
    SyntaxError,

    #[display(fmt = "Type not supported")]
    TypeNotSupported,

    #[display(fmt = "User cancelled")]
    UserCancelled,
}

pub fn substitute(input: &str, env: &Table) -> Result<String> {
    let mut output = String::new();

    for line in input.lines() {
        let mut slice = line;
        loop {
            match slice.find("{{") {
                None => {
                    if slice.find("}}").is_some() {
                        bail!(SubstituteError::SyntaxError)
                    }
                    output.push_str(slice);
                    break;
                },
                Some(pos) => {
                    output.push_str(&slice[..pos]);
                    slice = &slice[pos..];

                    let Some(end) = slice.find("}}").map(|i| i + 2) else {
                        bail!(SubstituteError::SyntaxError);
                    };

                    let rep = find_replacement(&slice[2 .. end - 2], env)?;
                    output.push_str(&rep);

                    slice = &slice[end..];
                }
            }
        }

        output.push_str("\n");
    }

    Ok(output)
}

fn find_replacement(placeholder: &str, env: &Table) -> Result<String> {
    let mut parts = placeholder.split("|");

    let key = parts.next().unwrap_or("").trim();
    match env.get(key) {
        Some(Value::String(v)) => Ok(v.to_string()),
        Some(Value::Integer(v)) => Ok(v.to_string()),
        Some(Value::Float(v)) => Ok(v.to_string()),
        Some(Value::Boolean(v)) => Ok(v.to_string()),
        Some(Value::Array(arr)) => {
            if is_interactive_mode() {
                Ok(select_replacement(key, &arr)?)
            } else {
                bail!(SubstituteError::ReplacementNotFound)
            }
        }
        Some(_) => bail!(SubstituteError::TypeNotSupported),
        None => {
            let fallback = parts.next().map(|fb| fb.trim());

            if is_interactive_mode() {
                Ok(prompt_user(key, fallback)?)
            } else {
                fallback
                    .map(|f| f.to_string())
                    .ok_or(SubstituteError::ReplacementNotFound.into())
            }
        },
    }
}

fn select_replacement(key: &str, values: &Vec<Value>) -> Result<String> {
    let display_names: Vec<String> = values
        .clone()
        .into_iter()
        .map(|v| match v {
            Value::Table(t) => {
                match t.get("name") {
                    Some(value) => value.to_string(),
                    None => t.to_string(),
                }
            },
            other => other.to_string(),
        })
        .collect();

    // How to abort on escape?
    let index_opt = FuzzySelect::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Select value for {}", key))
        .items(&display_names)
        .interact_opt()?;

    match index_opt {
        Some(index) => {
            match &values[index] {
                Value::Table(t) => match t.get("value") {
                    Some(value) => Ok(value.to_string()),
                    _ => bail!(SubstituteError::ReplacementNotFound),
                },
                other => Ok(other.to_string()),
            }
        },
        None => bail!(SubstituteError::UserCancelled),
    }
}

fn prompt_user(key: &str, fallback: Option<&str>) -> Result<String> {
    let fb = fallback.unwrap_or("");

    // Issue: Can't cancel here
    // https://github.com/console-rs/dialoguer/issues/160

    if key.ends_with("Date") {
        return prompt_for_date(key);
    }

    let input = Input::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Enter value for {}", key))
        .default(fb.to_string())
        .interact_text()?;

    Ok(input)
}

fn prompt_for_date(key: &str) -> Result<String> {
    let msg = format!("Select a date for {}", key);
    let date = DateSelect::new(&msg)
        .with_week_start(chrono::Weekday::Mon)
        .prompt()?;

    Ok(date.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_env() -> Table {
        let mut env = Table::new();

        env.insert("url".to_string(), Value::from("example.com"));
        env.insert("token".to_string(), Value::from("abc123"));
        env.insert("integer".to_string(), Value::from(42));
        env.insert("float".to_string(), Value::from(99.99));
        env.insert("boolean".to_string(), Value::from(true));

        env
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

