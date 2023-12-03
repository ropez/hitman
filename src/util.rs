const TRUNC_COLUMN: usize = 92;

macro_rules! err {
    ($msg:literal $(,)?) => { Err(eyre::eyre!($msg)) };
    ($err:expr $(,)?) => { Err(eyre::eyre!($err)) };
    ($fmt:expr, $($arg:tt)*) => { Err(eyre::eyre!($fmt, $($arg)*)) };
}

pub fn truncate(s: &str) -> String {
    match s.char_indices().nth(TRUNC_COLUMN - 3) {
        None => s.to_string(),
        Some((idx, _)) => format!("{}...", &s[..idx]),
    }
}

