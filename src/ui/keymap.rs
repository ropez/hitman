use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub enum KeyMapping {
    None,

    Up,
    Down,
    Left,
    Right,
    Abort,
    Accept,
    Tab,
    ScrollUp,
    ScrollDown,
    SelectTarget,
    ToggleHelp,
    ToggleWrap,
    ToggleHeaders,
    Reload,
    Editor,
    New,
    IncreaseWidth,
    DecreaseWitdh,
}

pub fn mapkey(event: &Event) -> KeyMapping {
    if let Event::Key(key) = event {
        if key.kind == KeyEventKind::Press {
            return mapkey_keypress(key);
        }
    }

    KeyMapping::None
}

fn mapkey_keypress(key: &KeyEvent) -> KeyMapping {
    use KeyCode::*;

    #[allow(clippy::match_same_arms)]
    match (key.modifiers, key.code) {
        (KeyModifiers::NONE, Up) => KeyMapping::Up,
        (KeyModifiers::NONE, Down) => KeyMapping::Down,
        (KeyModifiers::NONE, Left) => KeyMapping::Left,
        (KeyModifiers::NONE, Right) => KeyMapping::Right,
        (KeyModifiers::NONE, Esc) => KeyMapping::Abort,
        (KeyModifiers::NONE, Enter) => KeyMapping::Accept,
        (KeyModifiers::NONE, Tab) => KeyMapping::Tab,
        (KeyModifiers::CONTROL, Char('k')) => KeyMapping::Up,
        (KeyModifiers::CONTROL, Char('j')) => KeyMapping::Down,
        (KeyModifiers::CONTROL, Char('h')) => KeyMapping::Left,
        (KeyModifiers::CONTROL, Char('l')) => KeyMapping::Right,
        (KeyModifiers::CONTROL, Char('p')) => KeyMapping::Up,
        (KeyModifiers::CONTROL, Char('n')) => KeyMapping::Down,
        (KeyModifiers::CONTROL, Char('c')) => KeyMapping::Abort,
        (KeyModifiers::CONTROL, Char('u')) => KeyMapping::ScrollUp,
        (KeyModifiers::CONTROL, Char('d')) => KeyMapping::ScrollDown,
        (KeyModifiers::CONTROL, Char('s')) => KeyMapping::SelectTarget,
        (KeyModifiers::CONTROL, Char('r')) => KeyMapping::Reload,
        (KeyModifiers::CONTROL, Char('e')) => KeyMapping::Editor,
        (KeyModifiers::CONTROL, Char('a')) => KeyMapping::New,
        (KeyModifiers::CONTROL, Char(' ')) => KeyMapping::ToggleHeaders,
        (KeyModifiers::NONE, Char('<')) => KeyMapping::DecreaseWitdh,
        (KeyModifiers::NONE, Char('>')) => KeyMapping::IncreaseWidth,
        (KeyModifiers::NONE, Char(';')) => KeyMapping::ToggleWrap,
        (KeyModifiers::NONE, Char(',')) => KeyMapping::ToggleWrap,
        (KeyModifiers::NONE, Char('?')) => KeyMapping::ToggleHelp,

        _ => KeyMapping::None,
    }
}

pub fn keymap_list() -> Vec<(&'static str, &'static str)> {
    vec![
        ("<C-j> or <C-n>", "Select next"),
        ("<C-k> or <C-p>", "Select previous"),
        ("<C-u>", "Scroll up"),
        ("<C-d>", "Scroll down"),
        ("<C-s>", "Select target"),
        ("<C-r>", "Re-scan folder"),
        ("<C-e>", "Edit selected request"),
        ("<C-a>", "New request"),
        ("<Esc> or <C-c>", "Abort"),
        ("<C-space>", "Toggle request headers"),
        (",", "Toggle output wrapping"),
        ("?", "Toggle this help message"),
        ("<", "Increase output width"),
        (">", "Decrease output width"),
    ]
}
