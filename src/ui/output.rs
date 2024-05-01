use std::time::Duration;

use crossterm::event::Event;
use ratatui::{
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use super::{
    keymap::{mapkey, KeyMapping},
    Component, InteractiveComponent,
};

#[derive(Default, Clone)]
pub struct HttpRequestMessage(pub String);

#[derive(Default, Clone)]
pub struct HttpMessage {
    pub header: String,
    pub body: String,
}

pub struct HttpRequestInfo {
    request: HttpRequestMessage,
    status: RequestStatus,
}

impl HttpRequestInfo {
    pub fn new(request: HttpRequestMessage, status: RequestStatus) -> Self {
        Self { request, status }
    }
}

pub enum RequestStatus {
    Pending,
    Running,
    Complete {
        response: HttpMessage,
        elapsed: Duration,
    },
    Feiled {
        error: String,
    },
}

#[derive(Default)]
pub struct OutputView {
    request_info: Option<HttpRequestInfo>,
    scroll: (u16, u16),
}

impl OutputView {
    pub fn update(&mut self, info: HttpRequestInfo) {
        self.scroll = (0, 0);
        self.request_info = Some(info);
    }

    pub fn reset(&mut self) {
        self.scroll = (0, 0);
        self.request_info = None;
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

impl Component for OutputView {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        if let Some(info) = &self.request_info {
            let blue = Style::new().blue();
            let req_lines = info
                .request
                .0
                .lines()
                .map(|line| Line::styled(format!("> {line}"), blue));
            lines.extend(req_lines);

            lines.push(Line::default());

            match &info.status {
                RequestStatus::Pending => (),
                RequestStatus::Running => (),
                RequestStatus::Complete { response, .. } => {
                    let green = Style::new().green();
                    let res_lines = response
                        .header
                        .lines()
                        .map(|line| Line::styled(line, green));
                    lines.extend(res_lines);

                    let normal = Style::new();
                    let res_body_lines = response
                        .body
                        .lines()
                        .map(|line| Line::styled(line, normal));
                    lines.extend(res_body_lines);
                }
                RequestStatus::Feiled { error } => {
                    let yellow = Style::new().yellow();
                    let res_body_lines =
                        error.lines().map(|line| Line::styled(line, yellow));
                    lines.extend(res_body_lines);
                }
            }
        }

        let title_bottom = if let Some(info) = &self.request_info {
            if let RequestStatus::Complete { elapsed, .. } = &info.status {
                format!("Elapsed: {:.2?}", elapsed)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let para = Paragraph::new(Text::from(lines))
            .wrap(Wrap::default())
            .scroll(self.scroll)
            .block(
                Block::default()
                    .title("Output")
                    .title_bottom(title_bottom)
                    .borders(Borders::ALL)
                    .border_set(ratatui::symbols::border::ROUNDED)
            );

        frame.render_widget(para, area);
    }
}

impl InteractiveComponent for OutputView {
    type Intent = ();

    fn handle_event(&mut self, event: &Event) -> Option<()> {
        match mapkey(event) {
            KeyMapping::ScrollUp => {
                self.scroll_up();
            }
            KeyMapping::ScrollDown => {
                self.scroll_down();
            }
            _ => (),
        }

        None
    }
}
