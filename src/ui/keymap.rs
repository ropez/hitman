use crossterm::event::{Event, KeyCode, KeyModifiers};

pub enum KeyMapping {
    None,

    Up,
    Down,
    Abort,
    Accept,
    ScrollUp,
    ScrollDown,
    SelectTarget,
}

pub fn mapkey(event: &Event) -> KeyMapping {
    if let Event::Key(key) = event {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('j') | KeyCode::Char('n') => return KeyMapping::Down,
                KeyCode::Char('k') | KeyCode::Char('p') => return KeyMapping::Up,
                KeyCode::Char('c') => return KeyMapping::Abort,
                KeyCode::Char('u') => return KeyMapping::ScrollUp,
                KeyCode::Char('d') => return KeyMapping::ScrollDown,
                KeyCode::Char('s') => return KeyMapping::SelectTarget,
                _ => (),
            }
        }

        return match key.code {
            KeyCode::Esc => KeyMapping::Abort,
            KeyCode::Enter => KeyMapping::Accept,
            KeyCode::Down => KeyMapping::Down,
            KeyCode::Up => KeyMapping::Up,
            _ => KeyMapping::None,
        };
    }

    KeyMapping::None
}
