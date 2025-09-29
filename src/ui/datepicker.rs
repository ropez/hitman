use chrono::{Datelike, Days, Local, Months, NaiveDate, Weekday};
use hitman::substitute::SubstitutionValue;
use ratatui::{
    layout::{Constraint, Layout},
    prelude::{Alignment::Center, Margin},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

use super::{
    centered,
    keymap::{mapkey, KeyMapping},
    Component, PromptComponent, PromptIntent,
};

pub struct DatePicker {
    title: String,
    selected: NaiveDate,
}

impl DatePicker {
    pub fn new(title: String) -> Self {
        Self {
            title,
            selected: Local::now().date_naive(),
        }
    }

    pub fn with_fallback(self, fallback: Option<String>) -> Self {
        Self {
            selected: fallback
                .and_then(|f| f.parse::<NaiveDate>().ok())
                .unwrap_or(self.selected),
            ..self
        }
    }
}

impl Component for DatePicker {
    fn render_ui(
        &mut self,
        frame: &mut ratatui::Frame,
        area: ratatui::prelude::Rect,
    ) {
        let area = centered(area, 32, 14);

        let today = Local::now().date_naive();
        let date = self.selected;

        let start = NaiveDate::from_ymd_opt(date.year(), date.month(), 1)
            .expect("start of month to be valid");

        let end = start.checked_add_months(Months::new(1)).expect("add month");

        let start = start.week(Weekday::Mon).first_day();

        let weekdays = Line::from(
            start
                .iter_days()
                .take(7)
                .map(|d| {
                    Span::raw(format!(" {}", d.format("%a")))
                        .style(Style::new().white())
                })
                .collect::<Vec<_>>(),
        );

        let calendar: Vec<_> = start
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
                            Style::new().white()
                        };
                        Span::raw(format!(" {} ", d.format("%_d"))).style(style)
                    })
                    .collect();

                Line::from(days)
            })
            .collect();

        let block = Block::bordered().cyan().title(self.title.clone());

        let inner_layout = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(block.inner(area));

        frame.render_widget(Clear, area);
        frame.render_widget(block, area);

        let heading =
            Line::raw(format!("{}", date.format("%B %Y"))).alignment(Center);
        frame.render_widget(Paragraph::new(vec![heading]), inner_layout[1]);

        frame.render_widget(Paragraph::new(weekdays), inner_layout[2]);
        frame.render_widget(
            Paragraph::new(calendar),
            inner_layout[3].inner(Margin::new(1, 0)),
        );

        let current = Line::from(format!("{date}"))
            .alignment(Center)
            .style(Style::new().white());
        frame.render_widget(Paragraph::new(vec![current]), inner_layout[4]);
    }
}

impl PromptComponent for DatePicker {
    fn handle_prompt(
        &mut self,
        event: &crossterm::event::Event,
    ) -> Option<super::PromptIntent> {
        match mapkey(event) {
            KeyMapping::ScrollUp => {
                self.selected =
                    self.selected.checked_sub_months(Months::new(1)).unwrap();
            }
            KeyMapping::ScrollDown => {
                self.selected =
                    self.selected.checked_add_months(Months::new(1)).unwrap();
            }
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
                return Some(PromptIntent::Accept(SubstitutionValue::Single(
                    format!("{}", self.selected),
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
