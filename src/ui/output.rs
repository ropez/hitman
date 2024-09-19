use std::time::Duration;

use crossterm::event::Event;
use hitman::request::HitmanRequest;
use ratatui::{
    layout::Rect,
    style::{Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Color, Style as HlStyle, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};
use syntect_tui::into_span;

use super::{
    keymap::{mapkey, KeyMapping},
    Component, InteractiveComponent,
};

#[derive(Clone)]
pub struct HttpRequestMessage(pub HitmanRequest);

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

pub struct OutputView {
    content: Content,
    scroll: (u16, u16),
    noheaders: bool,
    nowrap: bool,
    highlighter: SyntaxHighlighter,
}

impl OutputView {
    pub fn new() -> Self {
        Self {
            content: Content::Empty,
            scroll: (0, 0),
            noheaders: false,
            nowrap: false,
            highlighter: SyntaxHighlighter::new(),
        }
    }

    pub fn show_preview(&mut self, text: String) {
        self.scroll = (0, 0);
        self.content = Content::Preview(text);
    }

    pub fn show_request(&mut self, info: HttpRequestInfo) {
        if let RequestStatus::Complete { response, .. } = &info.status {
            self.highlighter.update("json", &response.body);
        }

        self.scroll = (0, 0);
        self.content = Content::Request(info);
    }

    pub fn reset(&mut self) {
        self.scroll = (0, 0);
        self.content = Content::Empty;
    }

    pub fn scroll_up(&mut self) {
        if self.scroll.0 <= 15 {
            self.scroll.0 = 0;
        } else {
            self.scroll.0 -= 15;
        }
    }

    pub fn scroll_down(&mut self) {
        self.scroll.0 += 15;
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

    fn make_lines(&self) -> Vec<Line> {
        let mut lines: Vec<Line> = Vec::new();

        match &self.content {
            Content::Empty => {}
            Content::Request(info) => {
                let blue = Style::new().blue();

                for line in info.request.0.to_string().lines() {
                    lines.push(Line::styled(format!("> {}", line), blue));
                }

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

                        if let Some(highlighted_lines) =
                            self.highlighter.lines()
                        {
                            lines.extend(highlighted_lines);
                        } else {
                            lines.extend(
                                response
                                    .body
                                    .lines()
                                    .map(|line| Line::from(line)),
                            );
                        }
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
        lines
    }
}

impl Component for OutputView {
    fn render_ui(&mut self, frame: &mut Frame, area: Rect) {
        let title_bottom = if let Content::Request(info) = &self.content {
            if let RequestStatus::Complete { elapsed, .. } = &info.status {
                format!("Elapsed: {:.2?}", elapsed)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        let lines = self.make_lines();
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

struct SyntaxHighlighter {
    syntax_set: SyntaxSet,
    theme: Theme,

    cache: Option<Vec<Vec<(HlStyle, String)>>>,
}

impl SyntaxHighlighter {
    pub fn new() -> Self {
        let ps = SyntaxSet::load_defaults_newlines();

        // Load built-in theme
        let ts = ThemeSet::load_defaults();
        let mut theme = ts.themes["Solarized (dark)"].clone();

        // Set theme background to transparent
        let mut bg = theme.settings.background.unwrap_or(Color::BLACK);
        bg.a = 0;
        theme.settings.background = Some(bg);

        Self {
            syntax_set: ps,
            theme,
            cache: None,
        }
    }

    fn lines(&self) -> Option<Vec<Line>> {
        match &self.cache {
            Some(lines) => Some(
                lines
                    .iter()
                    .map(|line| {
                        let line_spans: Vec<Span> = line
                            .iter()
                            .filter_map(|seg| {
                                into_span((seg.0, seg.1.as_str())).ok()
                            })
                            .collect();

                        Line::from(line_spans)
                    })
                    .collect(),
            ),
            None => None,
        }
    }

    fn update(&mut self, extension: &str, text: &str) {
        let Some(syntax) = self.syntax_set.find_syntax_by_extension(extension)
        else {
            return;
        };

        let mut ctx = HighlightLines::new(syntax, &self.theme);

        let lines: Vec<_> = LinesWithEndings::from(text)
            .map(|line| {
                Ok(ctx
                    .highlight_line(line, &self.syntax_set)?
                    .into_iter()
                    // Map from &str to String, so that we can store it
                    .map(|(style, s)| (style, s.to_string()))
                    .collect())
            })
            .filter_map(|o: Result<Vec<_>, syntect::Error>| o.ok())
            .collect();

        self.cache.replace(lines);
    }
}
