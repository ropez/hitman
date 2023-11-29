use std::fs;
use std::str;
use std::fmt;
use std::error::Error;
use toml::{Table, Value};
use serde_json::{Value as JsonValue};
use jsonpath::Selector;
use substitute::substitute;

mod substitute;

pub fn load(path: &str, section: &Table) -> Result<String, Box<dyn Error>> {
    let buf = fs::read_to_string(path)?;

    Ok(substitute(&buf, section)?)
}

pub fn extract_variables(data: &JsonValue, scope: &Table) -> Result<Table, ()> {
    let mut out = Table::new();

    let extract = scope.get("_extract");
    match extract {
        Some(Value::Table(table)) => {
            for (key, value) in table {
                if let Value::String(jsonpath) = value {
                    let selector = Selector::new(&jsonpath).unwrap();
                    let result = selector.find(&data).next();

                    if let Some(JsonValue::String(val)) = result {
                        out.insert(key.clone(), Value::String(String::from(val)));
                    }
                }
            }
        },
        Some(_) => return Err(()),
        None => {},
    }

    Ok(out)
}

#[derive(Debug, Clone)]
pub struct SubstituteError;

impl fmt::Display for SubstituteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Substitution error")
    }
}

impl std::error::Error for SubstituteError {}

#[cfg(test)]
mod export_variables_tests {
    use super::*;

    #[test]
    fn extracts_variables_from_json() {
        // Scope should contain an _extract attribute with a jsonpath that reference
        // an element in data
        let scope = create_scope();

        let data = create_data();
        let res = extract_variables(&data, &scope).unwrap();

        assert!(res.get("token").is_some());
        assert_eq!(&Value::String(String::from("kokobaba1234")), res.get("token").unwrap());
    }

    fn create_data() -> JsonValue {
        let json = r#"{ "Data": { "Token": "kokobaba1234" } }"#;

        return serde_json::from_str(json).unwrap();
    }

    fn create_scope() -> Table {
        let mut table = Table::new();

        table.insert(String::from("url"), Value::from("example.com"));

        let mut extract = Table::new();

        extract.insert(String::from("token"), Value::from("$.Data.Token"));

        table.insert(String::from("_extract"), Value::from(extract));

        table
    }

}

