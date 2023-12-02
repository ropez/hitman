use eyre::{Result, eyre as err};
use toml::{Table, Value};
use colored::*;
use super::util::truncate;
use jsonpath::Selector;
use serde_json::Value as JsonValue;

pub fn extract_variables(data: &JsonValue, scope: &Table) -> Result<Table> {
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

                        // FIXME Structured logging, abbreiate long values
                        let msg = format!("# Got '{}' = '{}'", key, val);
                        eprintln!("{}", truncate(&msg).yellow());
                    }
                } else if let Value::Table(conf) = value {
                    let list = Selector::new(&get_string_or_panic(&conf, "list")).unwrap();
                    let value = Selector::new(&get_string_or_panic(&conf, "value")).unwrap();
                    let name = Selector::new(&get_string_or_panic(&conf, "name")).unwrap();

                    let res = list.find(&data).next().unwrap();
                    match res {
                        JsonValue::Array(items) => {
                            let x: Vec<Value> = items.iter()
                                .map(|k| {
                                    let v = value.find(&k).next().unwrap();
                                    let n = name.find(&k).next().unwrap();

                                    let mut out = Table::new();
                                    out.insert("value".to_string(), Value::try_from(v).unwrap());
                                    out.insert("name".to_string(), Value::try_from(n).unwrap());

                                    Value::Table(out)
                                })
                                .collect();

                            let msg = format!("# Got '{}' with {} elements", key, x.len());
                            eprintln!("{}", truncate(&msg).yellow());

                            let result = Value::Array(x);
                            out.insert(key.clone(), result);
                        }
                        _ => panic!("Oops"),
                    }
                }
            }
        },
        Some(_) => return Err(err!("Invalid _extract section")),
        None => {},
    }

    Ok(out)
}

fn get_string_or_panic(table: &Table, key: &str) -> String {
    match table.get(key) {
        Some(Value::String(s)) => s.clone(),
        _ => panic!("Oops"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_variables_from_json() {
        let env = toml::from_str(r#"
        url = "example.com"

        [_extract]
        token = "$.Data.Token"
        "#).unwrap();

        let data = serde_json::from_str(r#"{ 
            "Data": { "Token": "kokobaba1234" } 
        }"#).unwrap();

        let res = extract_variables(&data, &env).unwrap();

        assert!(res.get("token").is_some());
        assert_eq!(&Value::String(String::from("kokobaba1234")), res.get("token").unwrap());
    }

    #[test]
    fn extracts_multiple_values_into_array() {
        // workaround: Jsonpath crate doesn't support array
        let env = toml::from_str(r#"
        url = "example.com"

        [_extract]
        ToolId = { list = "$.Tools", value = "$.ToolId", name = "$.Name" }
        "#).unwrap();

        let data = serde_json::from_str(r#"{ 
            "Tools": [
                { "ToolId": 123, "Name": "First tool" },
                { "ToolId": 345, "Name": "Second tool" }
            ]
        }"#).unwrap();
        let res = extract_variables(&data, &env).unwrap();

        let expected: Table = toml::from_str(r#"
            ToolId = [
                { value = 123, name = "First tool" },
                { value = 345, name = "Second tool" },
            ]
        "#).unwrap();

        assert!(res.get("ToolId").is_some());
        assert_eq!(
            res.get("ToolId").unwrap(),
            expected.get("ToolId").unwrap(),
        );
    }
}

