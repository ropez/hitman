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

use crate::{
    request::{find_args, HitmanBody, HitmanRequest},
    resolve::{resolve_path, Resolved},
};

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

pub type SubstituteResult<T> = std::result::Result<T, SubstituteError>;

pub trait Replacer {
    fn find_replacement(
        &self,
        key: &str,
        fallback: Option<&str>,
    ) -> SubstituteResult<String>;
}

pub fn prepare_request<R: Replacer>(
    path: &Path,
    replacer: &R,
) -> anyhow::Result<SubstituteResult<HitmanRequest>> {
    let resolved = resolve_path(path)?;

    let input = read_to_string(resolved.http_file())?;
    let buf = match substitute(&input, replacer) {
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

    let body = match &resolved {
        Resolved::GraphQL { graphql_path, .. } => {
            let body = read_to_string(graphql_path)?;
            let args = find_args(graphql_path)?;
            let vars = match substitute_graphql(&args, replacer) {
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
        }
        Resolved::Simple { .. } => match parse_result {
            Complete(offset) => Some(HitmanBody::Plain {
                body: buf[offset..].to_string(),
            }),
            Partial => None,
        },
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

fn substitute_graphql<R: Replacer>(
    vars: &[String],
    replacer: &R,
) -> SubstituteResult<Vec<String>> {
    let mut output = vec![];

    for key in vars {
        output.push(replacer.find_replacement(key, None)?);
    }

    Ok(output)
}

pub fn substitute<R: Replacer>(
    input: &str,
    replacer: &R,
) -> SubstituteResult<String> {
    let mut output = String::new();

    for line in input.lines() {
        output.push_str(&substitute_line(line, replacer)?);
        output.push('\n');
    }

    Ok(output)
}

fn substitute_line<R: Replacer>(
    line: &str,
    replacer: &R,
) -> SubstituteResult<String> {
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

                let rep = substitute_inner(&slice[2..end - 2], replacer)?;

                // Nested substitution
                let rep = substitute_line(&rep, replacer)?;
                output.push_str(&rep);

                slice = &slice[end..];
            }
        }
    }

    Ok(output)
}

fn substitute_inner<R: Replacer>(
    inner: &str,
    replacer: &R,
) -> SubstituteResult<String> {
    let mut parts = inner.split('|');

    let key = parts.next().unwrap_or("").trim();
    let parsed_key = key.chars().filter(valid_character).collect::<String>();

    let fallback = parts.next().map(str::trim);

    replacer
        .find_replacement(&parsed_key, fallback)
        .map(|v| key.replace(&parsed_key, &v))
}

// Only valid with ascii_alphabetic, ascii_digit or underscores in key name
fn valid_character(c: &char) -> bool {
    c.is_ascii_alphabetic() || c.is_ascii_digit() || *c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockReplacer;

    impl Replacer for MockReplacer {
        fn find_replacement(
            &self,
            key: &str,
            fallback: Option<&str>,
        ) -> SubstituteResult<String> {
            match key {
                "url" => Ok("example.com".to_string()),
                "token" => Ok("abc123".to_string()),
                "integer" => Ok("42".to_string()),
                "float" => Ok("99.99".to_string()),
                "boolean" => Ok("true".to_string()),
                "api_url1" => Ok("foo.com".to_string()),
                "nested" => Ok("the answer is {{integer}}".to_string()),
                _ => Err(SubstituteError::ValueNotFound {
                    key: key.to_string(),
                    fallback: fallback.map(ToString::to_string),
                }),
            }
        }
    }

    #[test]
    fn returns_the_input_unchanged() {
        let res = substitute("foo\nbar\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo\nbar\n");
    }

    #[test]
    fn substitutes_single_variable() {
        let res = substitute("foo {{url}}\nbar\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitutes_integer() {
        let res = substitute("foo={{integer}}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo=42\n");
    }

    #[test]
    fn substitutes_float() {
        let res = substitute("foo={{float}}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo=99.99\n");
    }

    #[test]
    fn substitutes_boolean() {
        let res = substitute("foo: {{boolean}}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: true\n");
    }

    #[test]
    fn substitutes_placeholder_with_default_value() {
        let res =
            substitute("foo: {{url | fallback.com}}\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: example.com\n");
    }

    #[test]
    fn substitutes_default_value() {
        let err = substitute("foo: {{href | fallback.com }}\n", &MockReplacer)
            .unwrap_err();

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
        let res = substitute("foo {{ url  }}\nbar\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitutes_one_variable_per_line() {
        let res =
            substitute("foo {{url}}\nbar {{token}}\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo example.com\nbar abc123\n");
    }

    #[test]
    fn substitutes_variable_on_the_same_line() {
        let res =
            substitute("foo {{url}}, bar {{token}}\n", &MockReplacer).unwrap();

        assert_eq!(&res, "foo example.com, bar abc123\n");
    }

    #[test]
    fn substitutes_nested_variable() {
        let res = substitute("# {{nested}}!\n", &MockReplacer).unwrap();

        assert_eq!(&res, "# the answer is 42!\n");
    }

    #[test]
    fn substitutes_only_characters_inside_quotes() {
        let res = substitute("foo: {{ \"boolean\" }}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: \"true\"\n");
    }

    #[test]
    fn substitutes_only_characters_inside_list() {
        let res = substitute("foo: {{ [url] }}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: [example.com]\n");
    }

    #[test]
    fn substitutes_only_characters_inside_list_inside_quotes() {
        let res = substitute("foo: {{ [\"url\"] }}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: [\"example.com\"]\n");
    }

    #[test]
    fn substitutes_variable_on_the_same_line_in_list() {
        let res = substitute(
            "foo: [{{ \"url\" }}, {{ \"integer\" }}]",
            &MockReplacer,
        )
        .unwrap();

        assert_eq!(&res, "foo: [\"example.com\", \"42\"]\n");
    }

    #[test]
    fn substitutes_only_numbers_inside_quote() {
        let res = substitute("foo: {{ \"integer\" }}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: \"42\"\n");
    }

    #[test]
    fn substitutes_variable_with_underscore_and_number_in_name() {
        let res = substitute("foo: {{ api_url1 }}", &MockReplacer).unwrap();

        assert_eq!(&res, "foo: foo.com\n");
    }

    #[test]
    fn fails_for_unmatched_open() {
        let res = substitute("foo {{url\n", &MockReplacer);

        assert!(res.is_err());
    }

    #[test]
    fn fails_for_unmatched_close() {
        let res = substitute("foo url}} bar\n", &MockReplacer);

        assert!(res.is_err());
    }
}
