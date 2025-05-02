use anyhow::bail;
use toml::{Table, Value};

#[derive(Clone)]
pub struct Scope(Table);

#[derive(Debug, Clone, PartialEq)]
pub enum Replacement {
    Value(String),

    ValueNotFound {
        key: String,
    },

    MultipleValuesFound {
        key: String,
        values: Vec<toml::Value>,
    },
}

impl From<Table> for Scope {
    fn from(env: Table) -> Self {
        Self(env)
    }
}

impl Scope {
    pub fn lookup(&self, key: &str) -> anyhow::Result<Replacement> {
        let rep = match self.0.get(key) {
            None => Replacement::ValueNotFound { key: key.into() },
            Some(Value::String(v)) => Replacement::Value(v.clone()),
            Some(Value::Integer(v)) => Replacement::Value(v.to_string()),
            Some(Value::Float(v)) => Replacement::Value(v.to_string()),
            Some(Value::Boolean(v)) => Replacement::Value(v.to_string()),
            Some(Value::Array(arr)) => Replacement::MultipleValuesFound {
                key: key.into(),
                values: arr.clone(),
            },
            Some(_) => bail!("Type not supported"),
        };

        Ok(rep)
    }

    pub fn extract(&self) -> Option<&Value> {
        self.0.get("_extract")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_env() -> Scope {
        toml::from_str::<Table>(
            r#"
            url = "example.com"
            token = "abc123"
            integer = 42
            float = 99.99
            boolean = true
            api_url1 = "foo.com"

            nested = "the answer is {{integer}}"
            multiple = ["a", "b", "c"]
            "#,
        )
        .unwrap()
        .into()
    }

    #[test]
    fn finds_variable() {
        let rep = create_env();
        let res = rep.lookup("url").unwrap();

        assert_eq!(res, Replacement::Value("example.com".into()));
    }

    #[test]
    fn finds_integer() {
        let rep = create_env();
        let res = rep.lookup("integer").unwrap();

        assert_eq!(res, Replacement::Value("42".into()));
    }

    #[test]
    fn finds_float() {
        let rep = create_env();
        let res = rep.lookup("float").unwrap();

        assert_eq!(res, Replacement::Value("99.99".into()));
    }

    #[test]
    fn finds_boolean() {
        let rep = create_env();
        let res = rep.lookup("boolean").unwrap();

        assert_eq!(res, Replacement::Value("true".into()));
    }

    #[test]
    fn fails_for_multiple() {
        let rep = create_env();
        let res = rep.lookup("multiple").unwrap();

        assert_eq!(
            res,
            Replacement::MultipleValuesFound {
                key: "multiple".into(),
                values: vec!["a".into(), "b".into(), "c".into()]
            }
        );
    }

    #[test]
    fn fails_for_missing() {
        let rep = create_env();
        let res = rep.lookup("missing").unwrap();

        assert_eq!(
            res,
            Replacement::ValueNotFound {
                key: "missing".into()
            }
        );
    }
}
