use eyre::{bail, eyre, Result};
use log::info;
use toml::{Table, Value};

use super::util::truncate;
use jsonpath::Selector;
use serde_json::Value as JsonValue;

pub fn extract_variables(data: &JsonValue, scope: &Table) -> Result<Table> {
    let mut out = Table::new();

    let extract = scope.get("_extract");
    match extract {
        Some(Value::Table(table)) => {
            for (key, value) in table {
                match value {
                    Value::String(jsonpath) => {
                        let selector = make_selector(jsonpath)?;

                        if let Some(JsonValue::String(val)) = selector.find(data).next() {
                            let msg = format!("# Got '{}' = '{}'", key, val);
                            info!("{}", truncate(&msg));

                            out.insert(key.clone(), Value::String(String::from(val)));
                        }
                    }
                    Value::Table(conf) => {
                        let items_selector = make_selector(&get_string(conf, "_")?)?;
                        let value_selectors = make_item_selectors(conf)?;

                        // jsonpath returns an iterator that contains one element,
                        // which is the JSON array.

                        if let Some(JsonValue::Array(items)) = items_selector.find(data).next() {
                            let mut toml_items: Vec<Value> = Vec::new();

                            for item_json in items {
                                let mut toml_item = Table::new();
                                for (name, selector) in value_selectors.iter() {
                                    if let Some(v) = selector.find(item_json).next() {
                                        toml_item.insert(name.clone(), Value::try_from(v)?);
                                    }
                                }

                                let raw_json = Value::try_from(item_json.to_string())?;
                                toml_item.insert(String::from("_raw"), raw_json);

                                toml_items.push(Value::Table(toml_item));
                            }

                            let msg = format!("# Got '{}' with {} elements", key, toml_items.len());
                            info!("{}", truncate(&msg));

                            out.insert(key.clone(), Value::Array(toml_items));
                        }
                    }
                    _ => bail!("Invalid _extract rule: {}", value),
                }
            }
        }
        Some(_) => bail!("Invalid _extract section"),
        None => {}
    }

    Ok(out)
}

fn make_item_selectors(conf: &Table) -> Result<Vec<(String, Selector)>> {
    conf.iter()
        .filter_map(|(k, v)| {
            if k == "_" {
                None
            } else if let Value::String(s) = v {
                Some(make_selector(s).map(|r| (k.to_string(), r)))
            } else {
                None
            }
        })
        .collect::<Result<Vec<_>>>()
}

fn make_selector(path: &str) -> Result<Selector> {
    Selector::new(path).map_err(|err| eyre!("Invalid jsonpath: {}", err))
}

fn get_string(table: &Table, key: &str) -> Result<String> {
    match table.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        _ => bail!("Required key not found: {}", key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_variables_from_json() {
        let env = toml::from_str(
            r#"
        url = "example.com"

        [_extract]
        token = "$.Data.Token"
        "#,
        )
        .unwrap();

        let data = serde_json::from_str(
            r#"{ 
            "Data": { "Token": "kokobaba1234" } 
        }"#,
        )
        .unwrap();

        let res = extract_variables(&data, &env).unwrap();

        assert!(res.get("token").is_some());
        assert_eq!(
            &Value::String(String::from("kokobaba1234")),
            res.get("token").unwrap()
        );
    }

    #[test]
    fn extracts_multiple_values_into_array() {
        // workaround: Jsonpath crate doesn't support array
        let env = toml::from_str(
            r#"
        url = "example.com"

        [_extract]
        ToolId = { _ = "$.Tools", value = "$.ToolId", name = "$.Name" }
        "#,
        )
        .unwrap();

        let data = serde_json::from_str(
            r#"{ 
            "Tools": [
                { "Name": "First tool", "ToolId": 123 },
                { "Name": "Second tool", "ToolId": 345 }
            ]
        }"#,
        )
        .unwrap();
        let res = extract_variables(&data, &env).unwrap();

        let expected: Table = toml::from_str(
            r#"
            [[ToolId]]
            value = 123
            name = "First tool"
            _raw = "{\"Name\":\"First tool\",\"ToolId\":123}"

            [[ToolId]]
            value = 345
            name = "Second tool"
            _raw = "{\"Name\":\"Second tool\",\"ToolId\":345}"
        "#,
        )
        .unwrap();

        assert!(res.get("ToolId").is_some());
        assert_eq!(res.get("ToolId").unwrap(), expected.get("ToolId").unwrap(),);
    }
}
