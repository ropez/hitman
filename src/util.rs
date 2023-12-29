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

pub struct SplitWork {
    total_count: i32,
    workers: i32,
}

pub fn split_work(total_count: i32, workers: i32) -> SplitWork {
    SplitWork {
        total_count,
        workers,
    }
}

impl Iterator for SplitWork {
    type Item = i32;

    fn next(&mut self) -> Option<Self::Item> {
        if self.total_count > 0 {
            let c = (self.total_count + self.workers - 1) / self.workers;
            self.total_count -= c;
            self.workers -= 1;
            Some(c)
        } else {
            None
        }
    }
}

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

    #[test]
    fn splits_work_into_equal_chunks() {
        let chunks: Vec<_> = split_work(100, 4).collect();

        assert_eq!(chunks, vec![25, 25, 25, 25]);
    }

    #[test]
    fn splits_work_into_almost_equal_chunks() {
        let chunks: Vec<_> = split_work(15, 10).collect();

        assert_eq!(chunks, vec![2, 2, 2, 2, 2, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn splits_work_and_eliminates_zeros() {
        let chunks: Vec<_> = split_work(4, 10).collect();

        assert_eq!(chunks, vec![1, 1, 1, 1]);
    }
}
