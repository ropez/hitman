use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::Event;
use ratatui::{
    layout::Alignment,
    prelude::{Frame, Rect},
    style::{Style, Stylize},
    widgets::{Block, BorderType, Clear, Paragraph},
};

use super::{centered, Component};

pub struct Progress;

impl Component for Progress {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let pos = (t as usize / 50) % PATTERN.len();
        let loading = Paragraph::new(PATTERN[pos])
            .centered()
            .block(
                Block::bordered()
                    .title("Running")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded),
            )
            .style(Style::new().yellow());

        let inner_area = centered(area, 3, 18);
        frame.render_widget(Clear, inner_area);
        frame.render_widget(loading, inner_area);
    }

    fn handle_event(&mut self, _event: &Event) -> bool {
        false
    }
}

const PATTERN: &[&str] = &[
    "===             ",
    " ===            ",
    "  ===           ",
    "   ===          ",
    "    ===         ",
    "     ===        ",
    "      ===       ",
    "       ===      ",
    "        ===     ",
    "         ===    ",
    "          ===   ",
    "           ===  ",
    "            === ",
    "             ===",
    "            === ",
    "           ===  ",
    "          ===   ",
    "         ===    ",
    "        ===     ",
    "       ===      ",
    "      ===       ",
    "     ===        ",
    "    ===         ",
    "   ===          ",
    "  ===           ",
    " ===            ",
];
