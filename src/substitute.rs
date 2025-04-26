use anyhow::Context;
use httparse::Status::{Complete, Partial};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Method, Url,
};
use std::{
    collections::HashMap,
    fs::read_to_string,
    path::Path,
    str::{self, FromStr},
};
use thiserror::Error;
use toml::{Table, Value};

use crate::request::{find_args, resolve_http_file, HitmanBody, HitmanRequest};

#[derive(Error, Debug, Clone)]
pub enum SubstituteError {
    #[error("Missing substitution value for {key}")]
    ValueNotFound {
        key: String,
        fallback: Option<String>,
    },

    #[error("Found multiple possible substitutions for {key}")]
    MultipleValuesFound {
        key: String,
        values: Vec<toml::Value>,
    },

    #[error("Syntax error")]
    SyntaxError,

    #[error("Type not supported")]
    TypeNotSupported,
}

type SubstituteResult<T> = std::result::Result<T, SubstituteError>;

pub fn prepare_request(
    path: &Path,
    env: &Table,
) -> anyhow::Result<SubstituteResult<HitmanRequest>> {
    let extension = path.extension().context("Couldn't get ext")?;
    let http_file = resolve_http_file(path)?;
    let input = read_to_string(&http_file)?;
    let buf = match substitute(&input, env) {
        Ok(buf) => buf,
        Err(e) => return Ok(Err(e)),
    };

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);

    let parse_result = req
        .parse(buf.as_bytes())
        .context("Invalid input: malformed request")?;

    let method = req.method.context("Invalid input: HTTP method not found")?;
    let url = req.path.context("Invalid input: URL not found")?;

    let method = Method::from_str(method)?;
    let url = Url::parse(url)?;

    let mut map = HeaderMap::new();

    let body = if extension == "gql" || extension == "graphql" {
        let body = read_to_string(path)?;
        let args = find_args(path)?;
        let vars = match substitute_graphql(&args, env) {
            Ok(vars) => vars,
            Err(e) => return Ok(Err(e)),
        };

        if vars.is_empty() {
            Some(HitmanBody::GraphQL {
                body,
                variables: None,
            })
        } else {
            let mut map: HashMap<String, String> = HashMap::new();

            for (key, value) in args.into_iter().zip(vars.into_iter()) {
                map.insert(key, value);
            }

            let variables = serde_json::to_value(map)?;

            Some(HitmanBody::GraphQL {
                body,
                variables: Some(variables),
            })
        }
    } else {
        match parse_result {
            Complete(offset) => Some(HitmanBody::REST {
                body: buf[offset..].to_string(),
            }),
            Partial => None,
        }
    };

    for header in req.headers {
        // The parse_http crate is weird, it fills the array with empty headers
        // if a partial request is parsed.
        if header.name.is_empty() {
            break;
        }
        let value = str::from_utf8(header.value)?;
        let header_name = HeaderName::from_str(header.name)?;
        let header_value = HeaderValue::from_str(value)?;
        map.insert(header_name, header_value);
    }

    Ok(Ok(HitmanRequest {
        headers: map,
        url,
        method,
        body,
    }))
}

fn substitute_graphql(
    vars: &[String],
    env: &Table,
) -> SubstituteResult<Vec<String>> {
    let mut output = vec![];

    for line in vars {
        output.push(find_replacement(line, env)?);
    }

    Ok(output)
}

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
        Some(Value::Array(arr)) => Err(SubstituteError::MultipleValuesFound {
            key: parsed_key,
            values: arr.clone(),
        }),
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
        let err = substitute("foo: {{href | fallback.com }}\n", &env).unwrap_err();

        assert!(matches!(
            err,
            SubstituteError::ValueNotFound {
                key,
                fallback: Some(fb),
            } if key == "href" && fb == "fallback.com"
        ));
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

        assert!(res.is_err());
    }

    #[test]
    fn fails_for_unmatched_close() {
        let env = create_env();
        let res = substitute("foo url}} bar\n", &env);

        assert!(res.is_err());
    }
}
