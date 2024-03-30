use ratatui::{
    buffer::Buffer,
    layout::Rect,
    widgets::{Block, Borders, Paragraph, StatefulWidget, Widget},
};

#[derive(Default)]
pub struct OutputView;

impl StatefulWidget for OutputView {
    type State = OutputViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let para = Paragraph::new(state.output.clone())
            .scroll(state.scroll)
            .block(Block::default().title("Output").borders(Borders::ALL));

        para.render(area, buf);
    }
}

pub struct OutputViewState {
    output: String,
    scroll: (u16, u16),
}

impl Default for OutputViewState {
    fn default() -> Self {
        Self {
            output: String::new(),
            scroll: (0, 0),
        }
    }
}

impl OutputViewState {
    pub fn update(&mut self, output: String) {
        self.scroll = (0, 0);
        self.output = output;
    }

    pub fn scroll_up(&mut self) {
        if self.scroll.0 <= 5 {
            self.scroll.0 = 0;
        } else {
            self.scroll.0 -= 5;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll.0 += 5;
    }
}
