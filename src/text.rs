use crate::interface::DataOut;
use crate::view::View;
use chrono::{DateTime, Local};
use std::marker::PhantomData;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use tui::Frame;

pub struct TextView<'a, B: Backend> {
    history: Vec<ViewData<'a>>,
    capacity: usize,
    _marker: PhantomData<B>,
    auto_scroll: bool,
}

impl<'a, B: Backend> TextView<'a, B> {
    pub fn new(capacity: usize) -> Self {
        Self {
            history: vec![],
            capacity,
            _marker: PhantomData,
            auto_scroll: true,
        }
    }
}

impl<'a, B: Backend> View for TextView<'a, B> {
    type Backend = B;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect, scroll: (u16, u16)) {
        let height = (rect.height - 2) as usize;
        let scroll = if self.auto_scroll {
            let max_size = self.max_main_axis((f.size().width, f.size().height));

            if max_size > height {
                ((max_size - height) as u16, 0)
            } else {
                (0, 0)
            }
        } else {
            scroll
        };

        let block = if self.auto_scroll {
            Block::default()
                .title(format!("[{:03}] Text UTF-8", self.history.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::White))
        } else {
            Block::default()
                .title(format!("[{:03}] Text UTF-8", self.history.len()))
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::RAPID_BLINK),
                )
        };

        let text = self
            .history
            .iter()
            .map(|x| x.spans.clone())
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text)
            .block(block)
            .wrap(Wrap { trim: false })
            .scroll(scroll);
        f.render_widget(paragraph, rect);
    }

    fn add_data_out(&mut self, data: DataOut) {
        if self.history.len() >= self.capacity {
            self.history.remove(0);
        }

        match data {
            DataOut::Data(timestamp, data) => {
                let contents = ViewData::decode_ansi_color(&data);
                for (content, color) in contents {
                    self.history
                        .push(ViewData::if_data(timestamp, content, color));
                }
            }
            DataOut::ConfirmData(timestamp, data) => {
                self.history.push(ViewData::user_data(timestamp, data))
            }
            DataOut::ConfirmCommand(timestamp, cmd_name, data) => self
                .history
                .push(ViewData::user_command(timestamp, cmd_name, data)),
            DataOut::FailData(timestamp, data) => {
                self.history.push(ViewData::fail_data(timestamp, data))
            }
            DataOut::FailCommand(timestamp, cmd_name, _data) => self
                .history
                .push(ViewData::fail_command(timestamp, cmd_name)),
        };
    }

    fn clear(&mut self) {
        self.history.clear();
    }

    fn toggle_auto_scroll(&mut self) {
        self.auto_scroll = !self.auto_scroll;
    }

    fn max_main_axis(&self, frame_size: (u16, u16)) -> usize {
        self.history.iter().fold(0usize, |cnt, x| {
            let lines = ((x.length - 1) / frame_size.0 as usize) + 1;
            cnt + lines
        })
    }
}

struct ViewData<'a> {
    length: usize,
    spans: Spans<'a>,
}

impl<'a> ViewData<'a> {
    fn decode_ansi_color(text: &str) -> Vec<(String, Color)> {
        if text.is_empty() {
            return vec![];
        }

        let splitted = text.split("\x1B[").collect::<Vec<_>>();
        let mut res = vec![];

        let pattern_n_color = [
            ("0m", Color::White),
            ("30m", Color::Black),
            ("0;30m", Color::Black),
            ("31m", Color::Red),
            ("0;31m", Color::Red),
            ("32m", Color::Green),
            ("0;32m", Color::Green),
            ("33m", Color::Yellow),
            ("0;33m", Color::Yellow),
            ("34m", Color::Blue),
            ("0;34m", Color::Blue),
            ("35m", Color::Magenta),
            ("0;35m", Color::Magenta),
            ("36m", Color::Cyan),
            ("0;36m", Color::Cyan),
            ("37m", Color::Gray),
            ("0;37m", Color::Gray),
        ];

        for splitted_str in splitted.iter() {
            if splitted_str.is_empty() {
                continue;
            }

            if pattern_n_color.iter().all(|(pattern, color)| {
                if splitted_str.starts_with(pattern) {
                    let final_str = splitted_str
                        .to_string()
                        .replace(pattern, "")
                        .trim()
                        .to_string();
                    if final_str.is_empty() {
                        return true;
                    }

                    res.push((final_str, *color));
                    return false;
                }

                true
            }) && !splitted_str.starts_with("0m")
            {
                res.push((splitted_str.to_string(), Color::White));
            }
        }

        res
    }

    fn build_spans(timestamp: DateTime<Local>, content: String, fg: Color, bg: Color) -> Spans<'a> {
        let tm_fg = if bg != Color::Reset { bg } else { fg };

        Spans::from(vec![
            Span::styled(
                format!("[{}] ", timestamp.format("%d/%m/%Y %H:%M:%S")),
                Style::default().fg(tm_fg),
            ),
            Span::styled(
                format!("{}{} ", if bg != Color::Reset { " " } else { "" }, content),
                Style::default().bg(bg).fg(fg),
            ),
        ])
    }

    fn compute_content_length(timestamp: DateTime<Local>, content: &str) -> usize {
        let timestamp_str = format!("[{}] ", timestamp.format("%d/%m/%Y %H:%M:%S"));

        timestamp_str.chars().count() + content.chars().count() + 2
    }

    fn if_data(timestamp: DateTime<Local>, content: String, color: Color) -> Self {
        Self {
            length: ViewData::compute_content_length(timestamp, &content),
            spans: ViewData::build_spans(timestamp, content, color, Color::Reset),
        }
    }

    fn user_data(timestamp: DateTime<Local>, content: String) -> Self {
        Self {
            length: ViewData::compute_content_length(timestamp, &content),
            spans: ViewData::build_spans(timestamp, content, Color::Black, Color::LightCyan),
        }
    }

    fn user_command(timestamp: DateTime<Local>, cmd_name: String, content: String) -> Self {
        let content = format!("</{cmd_name}> {content}");

        Self {
            length: ViewData::compute_content_length(timestamp, &content),
            spans: ViewData::build_spans(timestamp, content, Color::Black, Color::LightGreen),
        }
    }

    fn fail_data(timestamp: DateTime<Local>, content: String) -> Self {
        let content = format!("Cannot send \"{content}\"");

        Self {
            length: ViewData::compute_content_length(timestamp, &content),
            spans: ViewData::build_spans(timestamp, content, Color::White, Color::LightRed),
        }
    }

    fn fail_command(timestamp: DateTime<Local>, cmd_name: String) -> Self {
        let content = format!("Cannot send </{cmd_name}>");

        Self {
            length: ViewData::compute_content_length(timestamp, &content),
            spans: ViewData::build_spans(timestamp, content, Color::White, Color::LightRed),
        }
    }
}
