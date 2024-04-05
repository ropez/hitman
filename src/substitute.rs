use std::str;
use thiserror::Error;
use toml::{Table, Value};

#[derive(Error, Debug, Clone)]
pub enum SubstituteError {
    #[error("Missing substitution value for {key}")]
    ValueNotFound { key: String, fallback: Option<String> },

    #[error("Found multiple possible substitutions for {key}")]
    MultipleValuesFound { key: String, values: Vec<toml::Value> },

    #[error("Syntax error")]
    SyntaxError,

    #[error("Type not supported")]
    TypeNotSupported,
}

type SubstituteResult<T> = std::result::Result<T, SubstituteError>;

pub fn substitute(input: &str, env: &Table) -> SubstituteResult<String> {
    let mut output = String::new();

    for line in input.lines() {
        output.push_str(&substitute_line(line, env)?);
        output.push('\n');
    }

    Ok(output)
}

fn substitute_line(line: &str, env: &Table) -> SubstituteResult<String> {
    let mut output = String::new();
    let mut slice = line;
    loop {
        match slice.find("{{") {
            None => {
                if slice.contains("}}") {
                    return Err(SubstituteError::SyntaxError);
                }
                output.push_str(slice);
                break;
            }
            Some(pos) => {
                output.push_str(&slice[..pos]);
                slice = &slice[pos..];

                let Some(end) = slice.find("}}").map(|i| i + 2) else {
                    return Err(SubstituteError::SyntaxError);
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

fn find_replacement(
    placeholder: &str,
    env: &Table,
) -> SubstituteResult<String> {
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
            Err(SubstituteError::MultipleValuesFound {
                key: parsed_key,
                values: arr.clone(),
            })
        }
        Some(_) => Err(SubstituteError::TypeNotSupported),
        None => {
            let fallback = parts.next().map(|fb| fb.trim().to_string());

            Err(SubstituteError::ValueNotFound {
                key: parsed_key,
                fallback,
            })
        }
    }
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
        let res = substitute("foo: [{{ \"url\" }}, {{ \"integer\" }}]", &env)
            .unwrap();

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
