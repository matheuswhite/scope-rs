use crate::messages::SerialRxData;
use crate::rich_string::RichText;
use crate::storage::Storage;
use chrono::{DateTime, Local};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::block::Title;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

pub struct TextView {
    history: Vec<ViewData>,
    save_file: Storage,
    capacity: usize,
    auto_scroll: bool,
    scroll: (u16, u16),
    frame_height: u16,
}

impl TextView {
    pub fn new(capacity: usize, filename: String) -> Self {
        Self {
            history: vec![],
            save_file: Storage::new(filename),
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

    pub fn get_save_file_storage(&self) -> &Storage {
        &self.save_file
    }

    pub fn draw(&self, f: &mut Frame, rect: Rect, blink_color: Option<Color>) {
        let scroll = if self.auto_scroll {
            (self.max_main_axis(), self.scroll.1)
        } else {
            self.scroll
        };

        let (coll, coll_size) = (&self.history[(scroll.0 as usize)..], self.history.len());

        let save_file = self.save_file.get_filename();
        let border_type = if self.auto_scroll {
            BorderType::Thick
        } else {
            BorderType::Double
        };
        let block = Block::default()
            .title(format!("[{:03}][ASCII] {}", coll_size, save_file))
            .title(
                Title::from(format!("[{}]", self.save_file.get_size())).alignment(Alignment::Right),
            )
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(Style::default().fg(blink_color.unwrap_or(Color::Reset)));

        let text = coll
            .iter()
            .map(|ViewData { data, timestamp }| {
                let timestamp_span = Span::styled(
                    format!("{} ", timestamp.format("%H:%M:%S.%3f")),
                    Style::default().fg(Color::DarkGray),
                );
                let content = vec![timestamp_span]
                    .into_iter()
                    .chain(RichText::crop_rich_texts(data, scroll.1 as usize))
                    .collect::<Vec<_>>();

                Line::from(content)
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(paragraph, rect);
    }

    pub fn add_data_out(&mut self, data: SerialRxData) {
        if self.history.len() >= self.capacity {
            self.history.remove(0);
        }

        self.save_file += data.serialize();
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

    pub fn save_history(&mut self) -> Result<(), String> {
        self.save_file.flush()
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
