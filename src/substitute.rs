use anyhow::{bail, Context};
use httparse::Status;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Method, Url,
};
use std::{
    collections::HashMap,
    fs::read_to_string,
    str::{self, FromStr},
};

use crate::{
    request::{find_args, HitmanBody, HitmanRequest},
    resolve::Resolved,
};

#[derive(Debug, PartialEq, Eq)]
pub enum Substitution<T> {
    Complete(T),
    ValueMissing {
        key: String,
        fallback: Option<String>,
    },
}

pub use Substitution::{Complete, ValueMissing};

pub fn prepare_request(
    resolved: &Resolved,
    vars: &HashMap<String, String>,
) -> anyhow::Result<Substitution<HitmanRequest>> {
    // FIXME This is still doing too much:
    // - Substituting placeholders in the raw input text
    // - Parsing the result as HTTP, yielding method, url, headers and body
    // - Loading and parsing GraphQL
    // - Generating variables for GraphQL (quite different for raw text substitution)

    let input = read_to_string(resolved.http_file())?;
    let buf = match substitute(&input, vars)? {
        Complete(buf) => buf,
        ValueMissing { key, fallback } => {
            return Ok(ValueMissing { key, fallback })
        }
    };

    let mut headers_buf = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers_buf);

    let parse_result = req
        .parse(buf.as_bytes())
        .context("Invalid input: malformed request")?;

    let method = req.method.context("Invalid input: HTTP method not found")?;
    let url = req.path.context("Invalid input: URL not found")?;

    let method = Method::from_str(method)?;
    let url = Url::parse(url)?;

    let body = match &resolved {
        Resolved::GraphQL { graphql_path, .. } => {
            let body = read_to_string(graphql_path)?;
            let args = find_args(graphql_path)?;

            if args.is_empty() {
                Some(HitmanBody::GraphQL {
                    body,
                    variables: None,
                })
            } else {
                let mut map: HashMap<String, String> = HashMap::new();

                for key in args {
                    let Some(value) = vars.get(&key) else {
                        return Ok(ValueMissing {
                            key,
                            fallback: None,
                        });
                    };

                    map.insert(key, value.clone());
                }

                let variables = serde_json::to_value(map)?;

                Some(HitmanBody::GraphQL {
                    body,
                    variables: Some(variables),
                })
            }
        }
        Resolved::Simple { .. } => match parse_result {
            Status::Complete(offset) => Some(HitmanBody::Plain {
                body: buf[offset..].to_string(),
            }),
            Status::Partial => None,
        },
    };

    let mut headers = HeaderMap::new();

    for header in req.headers {
        // The parse_http crate is weird, it fills the array with empty headers
        // if a partial request is parsed.
        if header.name.is_empty() {
            break;
        }
        let value = str::from_utf8(header.value)?;
        let header_name = HeaderName::from_str(header.name)?;
        let header_value = HeaderValue::from_str(value)?;
        headers.insert(header_name, header_value);
    }

    Ok(Complete(HitmanRequest {
        headers,
        url,
        method,
        body,
    }))
}

pub fn substitute(
    input: &str,
    vars: &HashMap<String, String>,
) -> anyhow::Result<Substitution<String>> {
    let mut output = String::new();

    for line in input.lines() {
        let res = match substitute_line(line, vars)? {
            Complete(l) => l,
            m @ ValueMissing { .. } => return Ok(m),
        };
        output.push_str(&res);
        output.push('\n');
    }

    Ok(Complete(output))
}

fn substitute_line(
    line: &str,
    vars: &HashMap<String, String>,
) -> anyhow::Result<Substitution<String>> {
    let mut output = String::new();
    let mut slice = line;
    loop {
        match slice.find("{{") {
            None => {
                if slice.contains("}}") {
                    bail!("Syntax error");
                }
                output.push_str(slice);
                break;
            }
            Some(pos) => {
                output.push_str(&slice[..pos]);
                slice = &slice[pos..];

                let Some(end) = slice.find("}}").map(|i| i + 2) else {
                    bail!("Syntax error");
                };

                let rep = match substitute_inner(&slice[2..end - 2], vars) {
                    Complete(v) => v,
                    m @ ValueMissing { .. } => return Ok(m),
                };

                // Nested substitution
                let rep = match substitute_line(&rep, vars)? {
                    Complete(v) => v,
                    m @ ValueMissing { .. } => return Ok(m),
                };
                output.push_str(&rep);

                slice = &slice[end..];
            }
        }
    }

    Ok(Complete(output))
}

fn substitute_inner(
    inner: &str,
    vars: &HashMap<String, String>,
) -> Substitution<std::string::String> {
    let mut parts = inner.split('|');

    let key = parts.next().unwrap_or("").trim();
    let parsed_key = key.chars().filter(valid_character).collect::<String>();

    let fallback = parts.next().map(str::trim);

    vars.get(&parsed_key)
        .map(|v| key.replace(&parsed_key, v))
        .map_or_else(
            || ValueMissing {
                key: key.to_string(),
                fallback: fallback.map(ToString::to_string),
            },
            Complete,
        )
}

// Only valid with ascii_alphabetic, ascii_digit or underscores in key name
fn valid_character(c: &char) -> bool {
    c.is_ascii_alphabetic() || c.is_ascii_digit() || *c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_vars() -> HashMap<String, String> {
        let mut vars = HashMap::new();

        vars.insert("url".to_string(), "example.com".to_string());
        vars.insert("token".to_string(), "abc123".to_string());
        vars.insert("integer".to_string(), "42".to_string());
        vars.insert("api_url1".to_string(), "foo.com".to_string());
        vars.insert(
            "nested".to_string(),
            "the answer is {{integer}}".to_string(),
        );

        vars
    }

    #[test]
    fn returns_the_input_unchanged() {
        let vars = create_vars();
        let res = substitute("foo\nbar\n", &vars).unwrap();

        assert_eq!(res, Complete("foo\nbar\n".to_string()));
    }

    #[test]
    fn substitutes_single_variable() {
        let vars = create_vars();
        let res = substitute("foo {{url}}\nbar\n", &vars).unwrap();

        assert_eq!(res, Complete("foo example.com\nbar\n".to_string()));
    }

    #[test]
    fn substitutes_integer() {
        let vars = create_vars();
        let res = substitute("foo={{integer}}", &vars).unwrap();

        assert_eq!(res, Complete("foo=42\n".to_string()));
    }

    #[test]
    fn substitutes_placeholder_with_default_value() {
        let vars = create_vars();
        let res = substitute("foo: {{url | fallback.com}}\n", &vars).unwrap();

        assert_eq!(res, Complete("foo: example.com\n".to_string()));
    }

    #[test]
    fn substitutes_default_value() {
        let vars = create_vars();
        let res = substitute("foo: {{href | fallback.com }}\n", &vars).unwrap();

        assert_eq!(res, ValueMissing { key: "href".to_string(), fallback: Some("fallback.com".to_string()) });
    }

    #[test]
    fn substitutes_single_variable_with_speces() {
        let vars = create_vars();
        let res = substitute("foo {{ url  }}\nbar\n", &vars).unwrap();

        assert_eq!(res, Complete("foo example.com\nbar\n".to_string()));
    }

    #[test]
    fn substitutes_one_variable_per_line() {
        let vars = create_vars();
        let res = substitute("foo {{url}}\nbar {{token}}\n", &vars).unwrap();

        assert_eq!(res, Complete("foo example.com\nbar abc123\n".to_string()));
    }

    #[test]
    fn substitutes_variable_on_the_same_line() {
        let vars = create_vars();
        let res = substitute("foo {{url}}, bar {{token}}\n", &vars).unwrap();

        assert_eq!(res, Complete("foo example.com, bar abc123\n".to_string()));
    }

    #[test]
    fn substitutes_nested_variable() {
        let vars = create_vars();
        let res = substitute("# {{nested}}!\n", &vars).unwrap();

        assert_eq!(res, Complete("# the answer is 42!\n".to_string()));
    }

    #[test]
    fn substitutes_only_characters_inside_quotes() {
        let vars = create_vars();
        let res = substitute("foo: {{ \"integer\" }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: \"42\"\n".to_string()));
    }

    #[test]
    fn substitutes_only_characters_inside_list() {
        let vars = create_vars();
        let res = substitute("foo: {{ [url] }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: [example.com]\n".to_string()));
    }

    #[test]
    fn substitutes_only_characters_inside_list_inside_quotes() {
        let vars = create_vars();
        let res = substitute("foo: {{ [\"url\"] }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: [\"example.com\"]\n".to_string()));
    }

    #[test]
    fn substitutes_variable_on_the_same_line_in_list() {
        let vars = create_vars();
        let res = substitute("foo: [{{ \"url\" }}, {{ \"integer\" }}]", &vars)
            .unwrap();

        assert_eq!(res, Complete("foo: [\"example.com\", \"42\"]\n".to_string()));
    }

    #[test]
    fn substitutes_only_numbers_inside_quote() {
        let vars = create_vars();
        let res = substitute("foo: {{ \"integer\" }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: \"42\"\n".to_string()));
    }

    #[test]
    fn substitutes_variable_with_underscore_and_number_in_name() {
        let vars = create_vars();
        let res = substitute("foo: {{ api_url1 }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: foo.com\n".to_string()));
    }

    #[test]
    fn fails_for_unmatched_open() {
        let vars = create_vars();
        let res = substitute("foo {{url\n", &vars);

        assert!(res.is_err());
    }

    #[test]
    fn fails_for_unmatched_close() {
        let vars = create_vars();
        let res = substitute("foo url}} bar\n", &vars);

        assert!(res.is_err());
    }
}
