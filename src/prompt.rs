use std::env;

pub fn set_interactive_mode(enable: bool) {
    env::set_var("interactive", if enable { "y" } else { "n" });
}

pub fn is_interactive_mode() -> bool {
    match env::var("interactive") {
        Ok(v) => v == "y",
        _ => false,
    }
}

