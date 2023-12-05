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

pub fn set_verbose(enable: bool) {
    set_boolean("verbose", enable);
}

pub fn is_verbose() -> bool {
    get_boolean("verbose")
}

pub fn set_quiet(enable: bool) {
    set_boolean("quiet", enable);
}

pub fn is_quiet() -> bool {
    get_boolean("quiet")
}
