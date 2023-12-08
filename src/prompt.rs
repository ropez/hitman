use std::env;

fn set_boolean(name: &str, value: bool) {
    env::set_var(name, if value { "y" } else { "n" });
}

fn get_boolean(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => v == "y",
        _ => false,
    }
}

pub fn set_interactive_mode(enable: bool) {
    set_boolean("interactive", enable);
}

pub fn is_interactive_mode() -> bool {
    get_boolean("interactive")
}

pub fn fuzzy_match(filter: &str, value: &str) -> bool {
    let value_lower = value.to_lowercase();
    let filter_lower = filter.to_lowercase();

    let mut value_chars = value_lower.chars();
    let mut filter_chars = filter_lower.chars();

    'outer: while let Some(filter_char) = filter_chars.next() {
        while let Some(value_char) = value_chars.next() {
            if value_char == filter_char {
                continue 'outer;
            }
        }
        return false;
    }

    return true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_true_for_identical() {
        assert!(fuzzy_match("a", "a"));
    }

    #[test]
    fn returns_false_for_different() {
        assert!(!fuzzy_match("a", "b"));
    }

    #[test]
    fn returns_false_for_different_length() {
        assert!(!fuzzy_match("ab", "a"));
    }

    #[test]
    fn returns_true_for_different_case() {
        assert!(fuzzy_match("a", "A"));
    }

    #[test]
    fn returns_true_if_filter_is_empty() {
        assert!(fuzzy_match("", "a"));
    }

    #[test]
    fn returns_false_if_value_is_empty() {
        assert!(!fuzzy_match("a", ""));
    }

    #[test]
    fn returns_true_value_contains_filter() {
        assert!(fuzzy_match("a", "ab"));
    }

    #[test]
    fn returns_true_if_value_contains_all_letters_in_filter_in_the_same_order() {
        assert!(fuzzy_match("abc", "uaaxbycz"));
    }
}
