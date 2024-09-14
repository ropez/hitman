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
    Running,
    Complete {
        response: HttpMessage,
        elapsed: Duration,
    },
    Failed {
        error: String,
    },
}

#[derive(Default)]
pub enum Content {
    #[default]
    Empty,
    Preview(String),
    Request(HttpRequestInfo),
}

#[derive(Default)]
pub struct OutputView {
    content: Content,
    scroll: (u16, u16),
    noheaders: bool,
    nowrap: bool,
}

impl OutputView {
    pub fn show_preview(&mut self, text: String) {
        self.scroll = (0, 0);
        self.content = Content::Preview(text);
    }

    pub fn show_request(&mut self, info: HttpRequestInfo) {
        self.scroll = (0, 0);
        self.content = Content::Request(info);
    }

    pub fn reset(&mut self) {
        self.scroll = (0, 0);
        self.content = Content::Empty;
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

    fn title(&self) -> &'static str {
        let title = match &self.content {
            Content::Empty => "",
            Content::Preview(_) => "Preview",
            Content::Request(_) => "Output",
        };
        title
    }

    fn mode_string(&self) -> String {
        let mut s = String::new();
        if !self.noheaders {
            s.push('H');
        }
        if !self.nowrap {
            s.push('W');
        }

        s
    }
}

impl Component for OutputView {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let mut lines: Vec<Line> = Vec::new();

        match &self.content {
            Content::Empty => {}
            Content::Request(info) => {
                let blue = Style::new().blue();
                let req_lines = info
                    .request
                    .0
                    .lines()
                    .take(if self.noheaders { 1 } else { usize::MAX })
                    .map(|line| Line::styled(format!("> {line}"), blue));
                lines.extend(req_lines);

                if !self.noheaders {
                    lines.push(Line::default());
                }

                match &info.status {
                    RequestStatus::Running => (),
                    RequestStatus::Complete { response, .. } => {
                        let green = Style::new().green();
                        let res_lines = response
                            .header
                            .lines()
                            .take(if self.noheaders { 1 } else { usize::MAX })
                            .map(|line| Line::styled(line, green));
                        lines.extend(res_lines);

                        let normal = Style::new();
                        let res_body_lines = response
                            .body
                            .lines()
                            .map(|line| Line::styled(line, normal));
                        lines.extend(res_body_lines);
                    }
                    RequestStatus::Failed { error } => {
                        let yellow = Style::new().yellow();
                        let res_body_lines = error
                            .lines()
                            .map(|line| Line::styled(line, yellow));
                        lines.extend(res_body_lines);
                    }
                }
            }
            Content::Preview(text) => {
                let blue = Style::new().blue();
                let req_lines = text
                    .lines()
                    .map(|line| Line::styled(format!("> {line}"), blue));
                lines.extend(req_lines);
            }
        }

        let title_bottom = if let Content::Request(info) = &self.content {
            if let RequestStatus::Complete { elapsed, .. } = &info.status {
                format!("Elapsed: {:.2?}", elapsed)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let para = Paragraph::new(Text::from(lines));

        let para = if self.nowrap {
            para
        } else {
            para.wrap(Wrap::default())
        };

        let para = para.scroll(self.scroll).block(
            Block::default()
                .title(self.title())
                .title_bottom(title_bottom)
                .title_bottom(Line::from(self.mode_string()).right_aligned())
                .borders(Borders::ALL)
                .border_set(ratatui::symbols::border::ROUNDED),
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
            KeyMapping::ToggleWrap => {
                self.nowrap = !self.nowrap;
            }
            KeyMapping::ToggleHeaders => {
                self.noheaders = !self.noheaders;
            }
            _ => (),
        }

        None
    }
}
