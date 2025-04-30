use toml::{Table, Value};

use crate::substitute::{Replacer, SubstituteError, SubstituteResult};

pub struct Env(Table);

// TODO: Incorporate "UserInteraction" into this
//
// For UI, we need to drive the whole substitution process in a async task,
// and use a channel to request input from the main thread.

impl Env {
    pub fn new(env: Table) -> Self {
        Self(env)
    }
}

impl Replacer for Env {
    fn find_replacement(
        &self,
        key: &str,
        fallback: Option<&str>,
    ) -> SubstituteResult<String> {
        match self.0.get(key) {
            Some(Value::String(v)) => Ok(v.clone()),
            Some(Value::Integer(v)) => Ok(v.to_string()),
            Some(Value::Float(v)) => Ok(v.to_string()),
            Some(Value::Boolean(v)) => Ok(v.to_string()),
            Some(Value::Array(arr)) => {
                Err(SubstituteError::MultipleValuesFound {
                    key: key.into(),
                    values: arr.clone(),
                })
            }
            Some(_) => Err(SubstituteError::TypeNotSupported),
            None => Err(SubstituteError::ValueNotFound {
                key: key.into(),
                fallback: fallback.map(ToString::to_string),
            }),
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
    fn finds_variable() {
        let rep = Env::new(create_env());
        let res = rep.find_replacement("url", None).unwrap();

        assert_eq!(&res, "example.com");
    }

    #[test]
    fn finds_integer() {
        let rep = Env::new(create_env());
        let res = rep.find_replacement("integer", None).unwrap();

        assert_eq!(&res, "42");
    }

    #[test]
    fn finds_float() {
        let rep = Env::new(create_env());
        let res = rep.find_replacement("float", None).unwrap();

        assert_eq!(&res, "99.99");
    }

    #[test]
    fn finds_boolean() {
        let rep = Env::new(create_env());
        let res = rep.find_replacement("boolean", None).unwrap();

        assert_eq!(&res, "true");
    }

    #[test]
    fn returns_fallback_for_missing() {
        let rep = Env::new(create_env());
        let err = rep.find_replacement("missing", Some("foobar")).unwrap_err();

        assert!(matches!(err, SubstituteError::ValueNotFound {
            key,
            fallback: Some(fb)
        } if key == "missing" && fb == "foobar" ));
    }

    #[test]
    fn fails_for_missing() {
        let rep = Env::new(create_env());
        let res = rep.find_replacement("missing", None);

        assert!(res.is_err());
    }
}
