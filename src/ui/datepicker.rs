use chrono::{Datelike, Days, Local, Months, NaiveDate, Weekday};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

use super::{
    keymap::{mapkey, KeyMapping},
    Component, PromptComponent, PromptIntent,
};

pub struct DatePicker {
    title: String,
    selected: NaiveDate,
}

impl DatePicker {
    pub fn new(title: String) -> Self {
        DatePicker {
            title,
            selected: Local::now().date_naive(),
        }
    }

    pub fn with_fallback(self, fallback: Option<String>) -> Self {
        self
    }
}

impl Component for DatePicker {
    fn render_ui(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
    ) {
        let chunks = Layout::new(
            Direction::Vertical,
            [
                Constraint::Percentage(50),
                Constraint::Length(12),
                Constraint::Percentage(50),
            ],
        )
        .split(area);

        let area = chunks[1];

        let block = Block::bordered().title(self.title.clone());

        let today = Local::now().date_naive();
        let date = self.selected;

        let start = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
            .expect("start of month to be valid");

        let end = start.checked_add_months(Months::new(1)).expect("add month");

        let start = start.week(Weekday::Mon).first_day();

        let mut lines: Vec<_> = start
            .iter_weeks()
            .take_while(|d| *d < end)
            .map(|d| {
                let days: Vec<_> = d
                    .iter_days()
                    .take(7)
                    .map(|d| {
                        let style = if d == date {
                            Style::new().reversed()
                        } else if d == today {
                            Style::new().cyan()
                        } else if d.month() != date.month() {
                            Style::new().dark_gray()
                        } else {
                            Style::new()
                        };
                        Span::raw(format!("{:>3} ", d.day())).style(style)
                    })
                    .collect();

                Line::from(days)
            })
            .collect();

        lines.insert(0, Line::raw(""));
        lines.insert(0, Line::raw(format!("{}", date.format("%B %Y"))));
        lines.insert(0, Line::raw(""));
        lines.push(Line::raw(""));
        lines.push(Line::from(format!("Date: {}", date)));

        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

impl PromptComponent for DatePicker {
    fn handle_prompt(
        &mut self,
        event: &crossterm::event::Event,
    ) -> Option<super::PromptIntent> {
        match mapkey(event) {
            KeyMapping::Up => {
                self.selected =
                    self.selected.checked_sub_days(Days::new(7)).unwrap();
            }
            KeyMapping::Down => {
                self.selected =
                    self.selected.checked_add_days(Days::new(7)).unwrap();
            }
            KeyMapping::Left => {
                self.selected =
                    self.selected.checked_sub_days(Days::new(1)).unwrap();
            }
            KeyMapping::Right => {
                self.selected =
                    self.selected.checked_add_days(Days::new(1)).unwrap();
            }
            KeyMapping::Accept => {
                return Some(PromptIntent::Accept(format!(
                    "{}",
                    self.selected
                )));
            }
            KeyMapping::Abort => {
                return Some(PromptIntent::Abort);
            }
            _ => (),
        }

        None
    }
}
