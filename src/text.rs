use crate::messages::SerialRxData;
use crate::rich_string::RichText;
use crate::ConcreteBackend;
use chrono::{DateTime, Local};
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Paragraph};
use tui::Frame;

pub struct TextView {
    history: Vec<ViewData>,
    capacity: usize,
    auto_scroll: bool,
    scroll: (u16, u16),
    frame_height: u16,
}

impl TextView {
    pub fn new(capacity: usize) -> Self {
        Self {
            history: vec![],
            capacity,
            auto_scroll: true,
            scroll: (0, 0),
            frame_height: u16::MAX,
        }
    }

    fn max_main_axis(&self) -> u16 {
        let main_axis_length = self.frame_height - 5;
        let history_len = self.history.len() as u16;

        if history_len > main_axis_length {
            history_len - main_axis_length
        } else {
            0
        }
    }

    pub fn draw(&self, f: &mut Frame<ConcreteBackend>, rect: Rect) {
        let scroll = if self.auto_scroll {
            (self.max_main_axis(), self.scroll.1)
        } else {
            self.scroll
        };

        let (coll, max, coll_size) = (
            &self.history[(scroll.0 as usize)..],
            "".to_string(),
            self.history.len(),
        );

        let block = if self.auto_scroll {
            Block::default()
                .title(format!("[{:03}{}] Text UTF-8", coll_size, max))
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(Color::Reset))
        } else {
            Block::default()
                .title(format!("[{:03}{}] Text UTF-8", coll_size, max))
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(
                    Style::default()
                        .fg(Color::Reset)
                        .add_modifier(Modifier::RAPID_BLINK),
                )
        };

        let text = coll
            .iter()
            .map(|ViewData { data, timestamp }| {
                let timestamp_span = Span::styled(
                    format!("{} ", timestamp.format("%H:%M:%S.%3f")),
                    Style::default().fg(Color::DarkGray),
                );
                let content = vec![timestamp_span]
                    .into_iter()
                    .chain(data.iter().enumerate().map(|(i, rich_text)| {
                        if i == 0 {
                            rich_text.crop_prefix_len(scroll.1 as usize).to_span()
                        } else {
                            rich_text.to_span()
                        }
                    }))
                    .collect::<Vec<_>>();

                Spans::from(content)
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, rect);
    }

    pub fn add_data_out(&mut self, data: SerialRxData) {
        if self.history.len() >= self.capacity {
            self.history.remove(0);
        }

        self.history.push(data.into());
    }

    pub fn clear(&mut self) {
        self.scroll = (0, 0);
        self.auto_scroll = true;
        self.history.clear();
    }

    pub fn up_scroll(&mut self) {
        if self.max_main_axis() > 0 {
            self.auto_scroll = false;
        }

        if self.scroll.0 < 3 {
            self.scroll.0 = 0;
        } else {
            self.scroll.0 -= 3;
        }
    }

    pub fn down_scroll(&mut self) {
        let max_main_axis = self.max_main_axis();

        self.scroll.0 += 3;
        self.scroll.0 = self.scroll.0.clamp(0, max_main_axis);

        if self.scroll.0 == max_main_axis {
            self.auto_scroll = true;
        }
    }

    pub fn left_scroll(&mut self) {
        if self.scroll.1 < 3 {
            self.scroll.1 = 0;
        } else {
            self.scroll.1 -= 3;
        }
    }

    pub fn right_scroll(&mut self) {
        self.scroll.1 += 3;
    }

    pub fn set_frame_height(&mut self, frame_height: u16) {
        self.frame_height = frame_height;
    }

    pub fn update_scroll(&mut self) {
        self.scroll = if self.auto_scroll {
            (self.max_main_axis(), self.scroll.1)
        } else {
            self.scroll
        };
    }
}

pub struct ViewData {
    timestamp: DateTime<Local>,
    data: Vec<RichText>,
}

impl ViewData {
    pub fn new(timestamp: DateTime<Local>, data: Vec<RichText>) -> Self {
        Self { timestamp, data }
    }
}
