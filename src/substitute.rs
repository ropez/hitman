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
    resolve::{Resolved, ResolvedAs},
};

#[derive(Debug, PartialEq, Eq)]
pub enum Substitution<T> {
    Complete(T),
    ValueMissing {
        key: String,
        fallback: Option<String>,
        multiple: bool,
    },
}

pub use Substitution::{Complete, ValueMissing};

pub fn prepare_request(
    resolved: &Resolved,
    vars: &HashMap<String, SubstitutionValue<String>>,
) -> anyhow::Result<Substitution<HitmanRequest>> {
    // FIXME This is still doing too much:
    // - Substituting placeholders in the raw input text
    // - Parsing the result as HTTP, yielding method, url, headers and body
    // - Loading and parsing GraphQL
    // - Generating variables for GraphQL (quite different for raw text
    //   substitution)

    let input = read_to_string(resolved.http_file())?;
    let buf = match substitute(&input, vars)? {
        Complete(buf) => buf,
        ValueMissing {
            key,
            fallback,
            multiple,
        } => {
            return Ok(ValueMissing {
                key,
                fallback,
                multiple,
            })
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

    let body = match &resolved.resolved_as {
        ResolvedAs::GraphQL { graphql_path, .. } => {
            let body = read_to_string(graphql_path)?;
            let args = find_args(graphql_path)?;

            if args.is_empty() {
                Some(HitmanBody::GraphQL {
                    body,
                    variables: None,
                })
            } else {
                let mut map: HashMap<String, serde_json::Value> =
                    HashMap::new();

                for key in args {
                    let Some(value) = vars.get(&key.name) else {
                        return Ok(ValueMissing {
                            key: key.name,
                            fallback: None,
                            multiple: key.list,
                        });
                    };

                    match value {
                        SubstitutionValue::Single(item) => {
                            map.insert(key.name, serde_json::to_value(item)?);
                        }
                        SubstitutionValue::Multiple(items) => {
                            map.insert(key.name, serde_json::to_value(items)?);
                        }
                    };
                }

                let variables = serde_json::to_value(map)?;

                Some(HitmanBody::GraphQL {
                    body,
                    variables: Some(variables),
                })
            }
        }
        ResolvedAs::Simple { .. } => match parse_result {
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

#[derive(Debug, Clone)]
pub enum SubstitutionValue<T> {
    Single(T),
    Multiple(Vec<T>),
}

pub fn substitute(
    input: &str,
    vars: &HashMap<String, SubstitutionValue<String>>,
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
    vars: &HashMap<String, SubstitutionValue<String>>,
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

#[derive(Debug)]
struct Pair {
    open: String,
    close: String,
}

#[derive(Debug)]
struct ListSyntax {
    separator: String,
    pair: Option<Pair>,
}

fn parse_list_syntax(s: &str) -> anyhow::Result<ListSyntax> {
    let first_open = s.find('[').context("Invalid list syntax")?;
    let first_close = s[first_open..]
        .find(']')
        .map(|i| i + first_open)
        .context("Invalid list syntax")?;

    let separator = &s[first_open + 1..first_close];

    let Some(second_open) = s[first_close..].find('[').map(|i| i + first_close)
    else {
        return Ok(ListSyntax {
            separator: separator.to_string(),
            pair: None,
        });
    };

    let second_close = s[second_open..]
        .find(']')
        .map(|i| i + second_open)
        .context("Invalid list syntax")?;

    let Some(third_open) =
        s[second_close..].find('[').map(|i| i + second_close)
    else {
        return Ok(ListSyntax {
            separator: separator.to_string(),
            pair: Some(Pair {
                open: s[second_open + 1..second_close].to_string(),
                close: s[second_open + 1..second_close].to_string(),
            }),
        });
    };

    let third_close = s[third_open..]
        .find(']')
        .map(|i| i + third_open)
        .context("Invalid list syntax")?;

    Ok(ListSyntax {
        separator: separator.to_string(),
        pair: Some(Pair {
            open: s[second_open + 1..second_close].to_string(),
            close: s[third_open + 1..third_close].to_string(),
        }),
    })
}

fn substitute_inner(
    inner: &str,
    vars: &HashMap<String, SubstitutionValue<String>>,
) -> Substitution<std::string::String> {
    let mut parts = inner.split('|');

    // Only valid with ascii_alphabetic, ascii_digit or underscores in key name
    let valid_character = |c: &char| -> bool {
        c.is_ascii_alphabetic() || c.is_ascii_digit() || *c == '_'
    };

    let key = parts.next().unwrap_or("").trim();
    let parsed_key = key
        .chars()
        .skip_while(|c| !valid_character(c))
        .take_while(valid_character)
        .collect::<String>();

    let fallback = parts.next().map(str::trim);

    let list_syntax = parse_list_syntax(key);

    let substitution = vars.get(&parsed_key).map(|v| match v {
        SubstitutionValue::Single(s) => key.replace(&parsed_key, s),
        SubstitutionValue::Multiple(list) => {
            let syntax = list_syntax.as_ref().unwrap();

            let start = inner.find(parsed_key.as_str()).unwrap();
            let end = inner.rfind(']').unwrap();
            let joined = list
                .iter()
                .map(|s| {
                    if let Some(pair) = &syntax.pair {
                        format!("{}{}{}", pair.open, s, pair.close)
                    } else {
                        s.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(&syntax.separator);

            key.replace(&inner[start..=end], &joined)
        }
    });

    match substitution {
        Some(s) => Complete(s),
        None => ValueMissing {
            key: parsed_key,
            fallback: fallback.map(ToString::to_string),
            multiple: list_syntax.is_ok(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_vars() -> HashMap<String, SubstitutionValue<String>> {
        let mut vars = HashMap::new();

        vars.insert(
            "url".to_string(),
            SubstitutionValue::Single("example.com".to_string()),
        );
        vars.insert(
            "token".to_string(),
            SubstitutionValue::Single("abc123".to_string()),
        );
        vars.insert(
            "integer".to_string(),
            SubstitutionValue::Single("42".to_string()),
        );
        vars.insert(
            "api_url1".to_string(),
            SubstitutionValue::Single("foo.com".to_string()),
        );
        vars.insert(
            "nested".to_string(),
            SubstitutionValue::Single("the answer is {{integer}}".to_string()),
        );
        vars.insert(
            "list".to_string(),
            SubstitutionValue::Multiple(vec![
                "1".to_string(),
                "2".to_string(),
                "3".to_string(),
            ]),
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

        assert_eq!(
            res,
            ValueMissing {
                key: "href".to_string(),
                fallback: Some("fallback.com".to_string()),
                multiple: false,
            }
        );
    }

    #[test]
    fn substitutes_default_value_multiple() {
        let vars = create_vars();
        let res = substitute(
            r#"foo: {{href | "fallback.com", "foobar.com" }}\n"#,
            &vars,
        )
        .unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "href".to_string(),
                fallback: Some("\"fallback.com\", \"foobar.com\"".to_string()),
                multiple: false,
            }
        );
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

        assert_eq!(
            res,
            Complete("foo: [\"example.com\", \"42\"]\n".to_string())
        );
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

    #[test]
    fn substitutes_list() {
        let vars = create_vars();
        let res = substitute("foo: {{ list[] }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: 123\n".to_string()));
    }

    #[test]
    fn substitutes_comma_separated_list() {
        let vars = create_vars();
        let res = substitute("foo: [ {{ list [, ] }} ]", &vars).unwrap();

        assert_eq!(res, Complete("foo: [ 1, 2, 3 ]\n".to_string()));
    }

    #[test]
    fn substitutes_list_multi_char_separator() {
        let vars = create_vars();
        let res = substitute("foo: {{ list [>>, <<]}}", &vars).unwrap();

        assert_eq!(res, Complete("foo: 1>>, <<2>>, <<3\n".to_string()));
    }

    #[test]
    fn substitutes_list_custom_open_pair() {
        let vars = create_vars();
        let res = substitute("foo: {{ list [:] ['] }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: '1':'2':'3'\n".to_string()));
    }

    #[test]
    fn substitutes_list_custom_open_and_close_pair() {
        let vars = create_vars();
        let res =
            substitute("foo: {{ list   [ - ] [<<][>>] }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: <<1>> - <<2>> - <<3>>\n".to_string()));
    }

    #[test]
    fn substitutes_list_default_value_multiple() {
        let vars = create_vars();
        let res = substitute(
            "foo: {{ missing_list   [ - ] [<<][>>] | 9 8 7 }}",
            &vars,
        )
        .unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "missing_list".to_string(),
                fallback: Some("9 8 7".to_string()),
                multiple: true,
            }
        );
    }

    #[test]
    fn substitutes_list_default_value_multiple_with_separator() {
        let vars = create_vars();
        let res = substitute(
            "foo: [ {{ missing_list   [ - ] [<<][>>] | \"9\", \"8\", \"7\" }} ]",
            &vars,
        )
        .unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "missing_list".to_string(),
                fallback: Some("\"9\", \"8\", \"7\"".to_string()),
                multiple: true,
            }
        );
    }

    #[test]
    fn substitutes_list_creates_object() {
        let vars = create_vars();
        let res =
            substitute("foo: {{ list [, ] [{ \"Id\": \"] [\" }] }}", &vars)
                .unwrap();

        assert_eq!(
            res,
            Complete(
                "foo: { \"Id\": \"1\" }, { \"Id\": \"2\" }, { \"Id\": \"3\" }\n".to_string()
            )
        );
    }
}
