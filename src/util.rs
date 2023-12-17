use std::collections::HashMap;
use std::hash::Hash;

const TRUNC_COLUMN: usize = 92;

pub fn truncate(s: &str) -> String {
    match s.char_indices().nth(TRUNC_COLUMN - 3) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}...", &s[..idx]),
    }
}

pub trait IterExt
where
    Self: Iterator + Sized,
{
    fn counted(self) -> HashMap<Self::Item, u32>
    where
        <Self as Iterator>::Item: Hash;
}

impl<T> IterExt for T
where
    T: Iterator,
    T::Item: Hash + Eq,
{
    fn counted(self) -> HashMap<T::Item, u32> {
        let mut map: HashMap<T::Item, u32> = HashMap::new();

        for item in self {
            map.entry(item).and_modify(|count| *count += 1).or_insert(1);
        }

        map
    }
}

// iter().counted()

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("foo"), "foo");
    }

    #[test]
    fn test_truncate_long() {
        let long = "x".repeat(200);
        let expected = "x".repeat(89) + "...";
        assert_eq!(truncate(&long), expected);
    }

    #[test]
    fn test_truncate_unicode() {
        let long = "√".repeat(200);
        let expected = "√".repeat(89) + "...";
        assert_eq!(truncate(&long), expected);
    }

    #[test]
    fn counted_numbers() {
        let values: Vec<i32> = vec![100, 200, 200, 300, 200];

        let result = values.into_iter().counted();
        assert_eq!(1, result[&100]);
        assert_eq!(3, result[&200]);
        assert_eq!(1, result[&300]);
    }

    #[test]
    fn counted_strings() {
        let values: Vec<_> = vec!["100", "200", "200", "300", "200"];

        let result = values.into_iter().counted();
        assert_eq!(1, result["100"]);
        assert_eq!(3, result["200"]);
        assert_eq!(1, result["300"]);
    }
}
