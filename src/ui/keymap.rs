use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub enum KeyMapping {
    None,

    Up,
    Down,
    Left,
    Right,
    Abort,
    Accept,
    ScrollUp,
    ScrollDown,
    SelectTarget,
    Reload,
    Editor,
    New,
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

    match (key.modifiers, key.code) {
        (KeyModifiers::NONE, Up) => KeyMapping::Up,
        (KeyModifiers::NONE, Down) => KeyMapping::Down,
        (KeyModifiers::NONE, Left) => KeyMapping::Left,
        (KeyModifiers::NONE, Right) => KeyMapping::Right,
        (KeyModifiers::NONE, Esc) => KeyMapping::Abort,
        (KeyModifiers::NONE, Enter) => KeyMapping::Accept,
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

        _ => KeyMapping::None,
    }
}
