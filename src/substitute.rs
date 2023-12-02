use std::str;
use toml::{Table, Value};
use derive_more::{Display, Error};

#[derive(Display, Error, Debug, Clone)]
pub struct SubstituteError;

pub fn substitute(input: &str, env: &Table) -> Result<String, SubstituteError> {
    let mut output = String::new();

    for line in input.lines() {
        let mut slice = line;
        loop {
            match slice.find("{{") {
                None => {
                    match slice.find("}}") {
                        Some(_) => return Err(SubstituteError),
                        None => {},
                    }
                    output.push_str(slice);
                    break;
                },
                Some(pos) => {
                    output.push_str(&slice[..pos]);
                    slice = &slice[pos..];

                    let Some(end) = slice.find("}}").map(|i| i + 2) else {
                        return Err(SubstituteError);
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

fn find_replacement(placeholder: &str, env: &Table) -> Result<String, SubstituteError> {
    let mut parts = placeholder.split("|");

    let key = parts.next().unwrap();
    match env.get(key.trim()) {
        Some(Value::String(v)) => Ok(v.to_string()),
        Some(Value::Integer(v)) => Ok(v.to_string()),
        Some(Value::Float(v)) => Ok(v.to_string()),
        Some(Value::Boolean(v)) => Ok(v.to_string()),
        Some(_) => Err(SubstituteError),
        None => {
            if let Some(fallback) = parts.next() {
                Ok(fallback.trim().to_string())
            } else {
                Err(SubstituteError)
            }
        },
    }
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

    #[test]
    fn fails_for_missing_variable() {
        let env = create_env();
        let res = substitute("foo {{koko}} bar\n", &env);
        assert!(res.is_err())
    }
}

