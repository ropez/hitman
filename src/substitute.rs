use std::cell::Cell;
use std::sync::{Arc, mpsc};
use std::thread;

use anyhow::Context;
use httparse::Status;
use minijinja::value::{Enumerator, ObjectRepr};
use minijinja::{Environment, UndefinedBehavior, Value, value::Object};
use reqwest::{
    Method, Url,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use std::{
    fs::read_to_string,
    str::{self, FromStr},
};

use crate::{
    request::{HitmanBody, HitmanRequest},
    resolve::{Resolved, ResolvedAs},
};

thread_local! {
    static FALLBACK: Cell<Option<String>> = const { Cell::new(None) };
}

#[derive(Debug, Clone)]
pub enum SubstituteValue {
    Single(Value),
    Multiple(Vec<toml::Value>),
}

#[derive(Debug)]
pub enum SubstituteEvent {
    LookupReplacement {
        key: String,
        reply_tx: mpsc::SyncSender<SubstituteValue>,
    },
    SelectSingle {
        key: String,
        values: Vec<toml::Value>,
        reply_tx: mpsc::SyncSender<Value>,
    },
    SelectMultiple {
        key: String,
        values: Vec<toml::Value>,
        reply_tx: mpsc::SyncSender<Vec<Value>>,
    },
}

#[derive(Debug)]
struct TrackingContext {
    tx: mpsc::SyncSender<SubstituteEvent>,
}

impl Object for TrackingContext {
    fn get_value(self: &Arc<Self>, key: &Value) -> Option<Value> {
        let key_str = key.as_str()?;
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(SubstituteEvent::LookupReplacement {
                key: key_str.to_string(),
                reply_tx,
            })
            .ok()?;

        let val = reply_rx.recv().ok()?;

        Some(match val {
            SubstituteValue::Single(value) => value,
            SubstituteValue::Multiple(values) => Value::from_object(
                SingleSelect::new(self.tx.clone(), key_str.to_string(), values),
            ),
        })
    }
}

#[derive(Debug, Clone)]
pub struct SingleSelect {
    tx: mpsc::SyncSender<SubstituteEvent>,
    key: String,
    values: Vec<toml::Value>,
}

impl SingleSelect {
    fn new(
        tx: mpsc::SyncSender<SubstituteEvent>,
        key: String,
        values: Vec<toml::Value>,
    ) -> Self {
        Self { tx, key, values }
    }

    fn to_multiple(&self) -> MultiSelect {
        MultiSelect::new(self.tx.clone(), self.key.clone(), self.values.clone())
    }
}

impl Object for SingleSelect {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Plain
    }

    fn render(
        self: &Arc<Self>,
        f: &mut std::fmt::Formatter<'_>,
    ) -> std::fmt::Result {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(SubstituteEvent::SelectSingle {
                key: self.key.clone(),
                values: self.values.clone(),
                reply_tx,
            })
            .unwrap();
        let value = reply_rx.recv().unwrap();

        write!(f, "{value}")
    }
}

#[derive(Debug, Clone)]
pub struct MultiSelect {
    tx: mpsc::SyncSender<SubstituteEvent>,
    key: String,
    values: Vec<toml::Value>,
}

impl MultiSelect {
    fn new(
        tx: mpsc::SyncSender<SubstituteEvent>,
        key: String,
        values: Vec<toml::Value>,
    ) -> Self {
        Self { tx, key, values }
    }
}

impl Object for MultiSelect {
    fn repr(self: &Arc<Self>) -> ObjectRepr {
        ObjectRepr::Iterable
    }

    fn enumerate(self: &Arc<Self>) -> Enumerator {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx
            .send(SubstituteEvent::SelectMultiple {
                key: self.key.clone(),
                values: self.values.clone(),
                reply_tx,
            })
            .unwrap();
        let values = reply_rx.recv().unwrap();

        Enumerator::Values(
            values.iter().map(|v| Value::from(v.clone())).collect(),
        )
    }
}

pub fn prepare_request(
    resolved: &Resolved,
    on_event: impl FnMut(SubstituteEvent) -> anyhow::Result<()>,
) -> anyhow::Result<HitmanRequest> {
    let input = read_to_string(resolved.http_file())?;
    let buf = substitute(input, on_event)?;

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
            todo!()
            // let body = read_to_string(graphql_path)?;
            // let args = find_args(graphql_path)?;
            //
            // if args.is_empty() {
            //     Some(HitmanBody::GraphQL {
            //         body,
            //         variables: None,
            //     })
            // } else {
            //     let mut map: HashMap<String, serde_json::Value> =
            //         HashMap::new();
            //
            //     for key in args {
            //         let Some(value) = vars.get(&key.name) else {
            //             return Ok(ValueMissing {
            //                 key: key.name,
            //                 fallback: None,
            //                 multiple: false,
            //             });
            //         };
            //
            //         map.insert(key.name, serde_json::to_value(value)?);
            //     }
            //
            //     let variables = serde_json::to_value(map)?;
            //
            //     Some(HitmanBody::GraphQL {
            //         body,
            //         variables: Some(variables),
            //     })
            // }
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

    Ok(HitmanRequest {
        headers,
        url,
        method,
        body,
    })
}

pub fn substitute(
    input: String,
    mut on_event: impl FnMut(SubstituteEvent) -> anyhow::Result<()>,
) -> anyhow::Result<String> {
    let (tx, rx) = mpsc::sync_channel(1);

    FALLBACK.set(None);
    let ctx = TrackingContext { tx };

    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_keep_trailing_newline(true);
    env.add_filter("select_multiple", |v: Value| {
        if let Some(obj) = v.downcast_object_ref::<SingleSelect>() {
            Value::from_object(obj.to_multiple())
        } else if v.downcast_object_ref::<MultiSelect>().is_some() {
            v
        } else {
            eprintln!("WARNING: Not multiple choice");
            v
        }
    });
    env.add_filter("select_one", |v: Value| v);

    env.add_filter("fallback", move |v: Value, fallback: String| {
        if v.is_undefined() {
            FALLBACK.set(Some(fallback));
        }
        v
    });

    let ctx_val = Value::from_object(ctx);

    let handle = thread::spawn(move || env.render_str(&input, ctx_val));

    while !handle.is_finished() {
        if let Ok(ev) = rx.recv() {
            on_event(ev)?;
        } else {
            break;
        }
    }

    Ok(handle.join().unwrap()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_vars() -> HashMap<String, Value> {
        let mut vars = HashMap::new();

        vars.insert("url".to_string(), Value::from("example.com"));
        vars.insert("token".to_string(), Value::from("abc123"));
        vars.insert("integer".to_string(), Value::from(42i64));
        vars.insert("api_url1".to_string(), Value::from("foo.com"));
        vars.insert(
            "list".to_string(),
            Value::from(vec![
                Value::from("1"),
                Value::from("2"),
                Value::from("3"),
            ]),
        );
        vars.insert(
            "label".to_string(),
            Value::from_serialize(vec![
                serde_json::json!({"value": "bug", "name": "bug"}),
                serde_json::json!({"value": "docs", "name": "documentation"}),
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

        assert_eq!(res, Complete("foo=42".to_string()));
    }

    #[test]
    fn substitutes_placeholder_with_default_value() {
        let vars = create_vars();
        let res =
            substitute("foo: {{ url | fallback('fallback.com') }}\n", &vars)
                .unwrap();

        assert_eq!(res, Complete("foo: example.com\n".to_string()));
    }

    #[test]
    fn substitutes_default_value() {
        let vars = create_vars();
        let res =
            substitute("foo: {{ href | fallback('fallback.com') }}\n", &vars)
                .unwrap();

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
    fn returns_value_missing_for_missing_variable() {
        let vars = create_vars();
        let res = substitute("foo: {{ href }}\n", &vars).unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "href".to_string(),
                fallback: None,
                multiple: false,
            }
        );
    }

    #[test]
    fn substitutes_single_variable_with_spaces() {
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
    fn substitutes_variable_with_underscore_and_number_in_name() {
        let vars = create_vars();
        let res = substitute("foo: {{ api_url1 }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: foo.com".to_string()));
    }

    #[test]
    fn substitutes_list_joined() {
        let vars = create_vars();
        let res = substitute("foo: {{ list | join('') }}", &vars).unwrap();

        assert_eq!(res, Complete("foo: 123".to_string()));
    }

    #[test]
    fn substitutes_comma_separated_list() {
        let vars = create_vars();
        let res =
            substitute("foo: [ {{ list | join(', ') }} ]", &vars).unwrap();

        assert_eq!(res, Complete("foo: [ 1, 2, 3 ]".to_string()));
    }

    #[test]
    fn substitutes_list_quoted_join() {
        let vars = create_vars();
        let res =
            substitute(r#"foo: ["{{ list | join('", "') }}"]"#, &vars).unwrap();

        assert_eq!(res, Complete(r#"foo: ["1", "2", "3"]"#.to_string()));
    }

    #[test]
    fn substitutes_list_of_objects() {
        let vars = create_vars();
        let res = substitute(
            r#"{% for l in label %}"{{ l.value }}"{% if not loop.last %}, {% endif %}{% endfor %}"#,
            &vars,
        )
        .unwrap();

        assert_eq!(res, Complete(r#""bug", "docs""#.to_string()));
    }

    #[test]
    fn returns_value_missing_when_var_missing_but_other_has_default() {
        let vars = create_vars();
        let res = substitute("{{ url | default('x') }} {{ missing }}", &vars)
            .unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "missing".to_string(),
                multiple: false,
                fallback: None,
            }
        );
    }

    #[test]
    fn returns_multiple_true_when_select_multiple_filter_used() {
        let vars = create_vars();
        let res = substitute("{{ missing | select_multiple }}", &vars).unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "missing".to_string(),
                multiple: true,
                fallback: None,
            }
        );
    }

    #[test]
    fn returns_fallback_when_fallback_filter_used() {
        let vars = create_vars();
        let res =
            substitute("{{ href | fallback('fallback.com') }}", &vars).unwrap();

        assert_eq!(
            res,
            ValueMissing {
                key: "href".to_string(),
                multiple: false,
                fallback: Some("fallback.com".to_string()),
            }
        );
    }

    #[test]
    fn fallback_filter_is_noop_when_value_present() {
        let vars = create_vars();
        let res =
            substitute("{{ url | fallback('fallback.com') }}", &vars).unwrap();

        assert_eq!(res, Complete("example.com".to_string()));
    }

    #[test]
    fn fails_for_template_syntax_error() {
        let vars = create_vars();
        let res = substitute("{% if %}", &vars);

        assert!(res.is_err());
    }
}
