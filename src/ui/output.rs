use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, StatefulWidget, Widget},
};

#[derive(Default)]
pub struct OutputView;

impl StatefulWidget for OutputView {
    type State = OutputViewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let blue = Style::new().blue();
        let req_lines = state.request.lines().map(|line| Line::styled(line, blue));

        let yellow = Style::new().yellow();
        let res_lines = state.response.lines().map(|line| Line::styled(line, yellow));

        let lines: Vec<Line> = req_lines.chain(res_lines).collect();

        let para = Paragraph::new(Text::from(lines))
            .scroll(state.scroll)
            .block(Block::default().title("Output").borders(Borders::ALL));

        para.render(area, buf);
    }
}

pub struct OutputViewState {
    request: String,
    response: String,
    scroll: (u16, u16),
}

impl Default for OutputViewState {
    fn default() -> Self {
        Self {
            request: String::default(),
            response: String::default(),
            scroll: (0, 0),
        }
    }
}

impl OutputViewState {
    pub fn update(&mut self, request: String, response: String) {
        self.scroll = (0, 0);
        self.request = request;
        self.response = response;
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
