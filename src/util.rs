const TRUNC_COLUMN: usize = 92;

pub fn truncate(s: &str) -> String {
    match s.char_indices().nth(TRUNC_COLUMN - 3) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}...", &s[..idx]),
    }
}
