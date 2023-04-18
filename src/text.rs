use crate::interface::DataOut;
use crate::view::View;
use chrono::{DateTime, Local};
use std::borrow::Cow;
use std::fmt::Write;
use std::marker::PhantomData;
use tui::backend::Backend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use tui::Frame;

pub struct TextView<'a, B: Backend> {
    history: Vec<ViewData<'a>>,
    capacity: usize,
    _marker: PhantomData<B>,
    auto_scroll: bool,
    snapshot_mode_en: bool,
    snapshot: Vec<ViewData<'a>>,
}

impl<'a, B: Backend> TextView<'a, B> {
    pub fn new(capacity: usize) -> Self {
        Self {
            history: vec![],
            capacity,
            _marker: PhantomData,
            auto_scroll: true,
            snapshot_mode_en: false,
            snapshot: vec![],
        }
    }
}

impl<'a, B: Backend> View for TextView<'a, B> {
    type Backend = B;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect, scroll: (u16, u16)) {
        let height = (rect.height - 2) as usize;
        let scroll = if self.auto_scroll {
            let max_size = self.max_main_axis();

            if max_size > height {
                ((max_size - height) as u16, scroll.1)
            } else {
                (0, scroll.1)
            }
        } else {
            scroll
        };

        let (coll, title, max, coll_size) = if self.snapshot_mode_en {
            (
                &self.snapshot[(scroll.0 as usize)..],
                "Snapshot",
                format!("/{}", self.snapshot.len()),
                self.snapshot.len(),
            )
        } else {
            (
                &self.history[(scroll.0 as usize)..],
                "Normal",
                "".to_string(),
                self.history.len(),
            )
        };

        let block = if self.auto_scroll {
            Block::default()
                .title(format!("[{:03}{}] Text UTF-8 <{}>", coll_size, max, title))
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::White))
        } else {
            Block::default()
                .title(format!("[{:03}{}] Text UTF-8 <{}>", coll_size, max, title))
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::RAPID_BLINK),
                )
        };

        let text = coll
            .iter()
            .map(|x| {
                let scroll = scroll.1 as usize;
                let content = if scroll >= x.data.len() {
                    ""
                } else {
                    &x.data[scroll..]
                };

                Spans::from(vec![
                    x.timestamp.clone(),
                    Span::styled(
                        format!(
                            "{}{} ",
                            if x.bg != Color::Reset { " " } else { "" },
                            content
                        ),
                        Style::default().bg(x.bg).fg(x.fg),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
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
            DataOut::ConfirmHexString(timestamp, bytes) => self
                .history
                .push(ViewData::user_hex_string(timestamp, bytes)),
            DataOut::FailData(timestamp, data) => {
                self.history.push(ViewData::fail_data(timestamp, data))
            }
            DataOut::FailCommand(timestamp, cmd_name, _data) => self
                .history
                .push(ViewData::fail_command(timestamp, cmd_name)),
            DataOut::FailHexString(timestamp, bytes) => self
                .history
                .push(ViewData::fail_hex_string(timestamp, bytes)),
        };
    }

    fn clear(&mut self) {
        self.history.clear();
    }

    fn toggle_auto_scroll(&mut self) {
        self.auto_scroll = !self.auto_scroll;
    }

    fn max_main_axis(&self) -> usize {
        self.history.len()
    }

    fn save_snapshot(&mut self) {
        let snapshot_capacity = self.capacity / 4;
        let last_index = self.history.len();
        let start = if snapshot_capacity > last_index {
            0
        } else {
            last_index - snapshot_capacity
        };

        self.snapshot = self.history[start..].to_vec();
    }

    fn toggle_snapshot_mode(&mut self) {
        self.snapshot_mode_en = !self.snapshot_mode_en;

        self.auto_scroll = !self.snapshot_mode_en;
    }
}

#[derive(Clone)]
struct ViewData<'a> {
    timestamp: Span<'a>,
    data: String,
    fg: Color,
    bg: Color,
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

    fn bytes_to_hex_string(bytes: &[u8]) -> String {
        let mut hex_string = String::new();

        for byte in bytes {
            write!(&mut hex_string, "{:02X}", byte).unwrap();
        }

        hex_string
    }

    fn build_timestmap_span(timestamp: DateTime<Local>, fg: Color, bg: Color) -> Span<'a> {
        let tm_fg = if bg != Color::Reset { bg } else { fg };

        Span::styled(
            format!("[{}] ", timestamp.format("%d/%m/%Y %H:%M:%S")),
            Style::default().fg(tm_fg),
        )
    }

    fn compute_content_length(timestamp: DateTime<Local>, content: &str) -> usize {
        let timestamp_str = format!("[{}] ", timestamp.format("%d/%m/%Y %H:%M:%S"));

        timestamp_str.chars().count() + content.chars().count() + 2
    }

    fn if_data(timestamp: DateTime<Local>, content: String, color: Color) -> Self {
        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, color, Color::Reset),
            data: content,
            fg: color,
            bg: Color::Reset,
        }
    }

    fn user_data(timestamp: DateTime<Local>, content: String) -> Self {
        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::Black, Color::LightCyan),
            data: content,
            fg: Color::Black,
            bg: Color::LightCyan,
        }
    }

    fn user_command(timestamp: DateTime<Local>, cmd_name: String, content: String) -> Self {
        let content = format!("</{cmd_name}> {content}");

        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::Black, Color::LightGreen),
            data: content,
            fg: Color::Black,
            bg: Color::LightGreen,
        }
    }

    fn user_hex_string(timestamp: DateTime<Local>, bytes: Vec<u8>) -> Self {
        let content = format!("<${}> {:?}", ViewData::bytes_to_hex_string(&bytes), &bytes);

        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::Black, Color::Yellow),
            data: content,
            fg: Color::Black,
            bg: Color::Yellow,
        }
    }

    fn fail_data(timestamp: DateTime<Local>, content: String) -> Self {
        let content = format!("Cannot send \"{content}\"");

        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::White, Color::LightRed),
            data: content,
            fg: Color::White,
            bg: Color::LightRed,
        }
    }

    fn fail_command(timestamp: DateTime<Local>, cmd_name: String) -> Self {
        let content = format!("Cannot send </{cmd_name}>");

        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::White, Color::LightRed),
            data: content,
            fg: Color::White,
            bg: Color::LightRed,
        }
    }

    fn fail_hex_string(timestamp: DateTime<Local>, bytes: Vec<u8>) -> Self {
        let content = format!("Cannot send <${}>", ViewData::bytes_to_hex_string(&bytes));

        Self {
            timestamp: ViewData::build_timestmap_span(timestamp, Color::White, Color::LightRed),
            data: content,
            fg: Color::White,
            bg: Color::LightRed,
        }
    }
}
