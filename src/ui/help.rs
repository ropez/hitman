use ratatui::{
    layout::{Alignment, Rect},
    style::Stylize,
    text::{Line, Text},
    widgets::{Block, BorderType, Clear, Padding, Paragraph},
    Frame,
};

use super::{centered, keymap::keymap_list, Component};

pub struct Help;

impl Component for Help {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let lines: Vec<_> = keymap_list()
            .into_iter()
            .map(|(a, b)| Line::from(format!("{a:-14} {b}")))
            .collect();

        let longest = lines.iter().map(|l| l.width()).max().unwrap_or(0);

        let inner_area =
            centered(area, 4 + longest as u16, 2 + lines.len() as u16);

        let content = Paragraph::new(Text::from(lines))
            .block(
                Block::bordered()
                    .title("Keyboard shortcuts")
                    .title_alignment(Alignment::Center)
                    .border_type(BorderType::Rounded)
                    .padding(Padding::horizontal(1))
                    .cyan(),
            )
            .white();

        frame.render_widget(Clear, inner_area);
        frame.render_widget(content, inner_area);
    }
}
