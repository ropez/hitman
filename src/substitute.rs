use std::str;
use toml::{Table, Value};
use crate::SubstituteError;

pub fn substitute(input: &str, section: &Table) -> Result<String, SubstituteError> {
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

                    match slice.find("}}").map(|i| i + 2) {
                        Some(end) => {
                            let key = &slice[2 .. end - 2];
                            match section.get(key.trim()) {
                                Some(Value::String(v)) => {
                                    output.push_str(v);
                                },
                                Some(_) => return Err(SubstituteError),
                                None => return Err(SubstituteError),
                            }

                            slice = &slice[end..];
                        },
                        None => return Err(SubstituteError),
                    }
                }
            }
        }

        output.push_str("\n");
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_table() -> Table {
        let mut table = Table::new();

        table.insert("url".to_string(), Value::from("example.com"));
        table.insert("token".to_string(), Value::from("abc123"));

        table
    }

    #[test]
    fn returns_the_input_unchanged() {
        let tab = create_table();
        let res = substitute("foo\nbar\n", &tab).unwrap();

        assert_eq!(&res, "foo\nbar\n");
    }

    #[test]
    fn substitues_single_variable() {
        let tab = create_table();
        let res = substitute("foo {{url}}\nbar\n", &tab).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitues_single_variable_with_speces() {
        let tab = create_table();
        let res = substitute("foo {{ url  }}\nbar\n", &tab).unwrap();

        assert_eq!(&res, "foo example.com\nbar\n");
    }

    #[test]
    fn substitues_one_variable_per_line() {
        let tab = create_table();
        let res = substitute("foo {{url}}\nbar {{token}}\n", &tab).unwrap();

        assert_eq!(&res, "foo example.com\nbar abc123\n");
    }

    #[test]
    fn substitues_variable_on_the_same_line() {
        let tab = create_table();
        let res = substitute("foo {{url}}, bar {{token}}\n", &tab).unwrap();

        assert_eq!(&res, "foo example.com, bar abc123\n");
    }

    #[test]
    fn fails_for_unmatched_open() {
        let tab = create_table();
        let res = substitute("foo {{url\n", &tab);
        assert!(res.is_err())
    }

    #[test]
    fn fails_for_unmatched_close() {
        let tab = create_table();
        let res = substitute("foo url}} bar\n", &tab);
        assert!(res.is_err())
    }

    #[test]
    fn fails_for_missing_variable() {
        let tab = create_table();
        let res = substitute("foo {{koko}} bar\n", &tab);
        assert!(res.is_err())
    }
}

