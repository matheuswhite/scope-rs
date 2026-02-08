use crate::{
    graphics::{
        ansi::ANSI,
        buffer::{Buffer, BufferLine, BufferPosition, timestamp_fmt},
        graphics_task::SaveStats,
        palette::Palette,
        special_char::{SpecialCharItem, ToSpecialChar},
    },
    infra::{ByteFormat, LogLevel},
};
use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, block::Title},
};

pub struct Screen {
    position: BufferPosition,
    auto_scroll: bool,
    mode: ScreenMode,
    decoder: ScreenDecoder,
    size: Rect,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            position: Default::default(),
            auto_scroll: true,
            mode: ScreenMode::Normal,
            decoder: ScreenDecoder::Ascii,
            size: Rect {
                x: 0,
                y: 0,
                width: u16::MAX,
                height: u16::MAX,
            },
        }
    }
}

impl Screen {
    fn clamp_position(&mut self, max_main_axis: usize) {
        self.position.line = self.position.line.clamp(0, max_main_axis);
        if self.position.line == max_main_axis {
            self.auto_scroll = true;
        }
    }

    pub fn change_mode_to_normal(&mut self, max_main_axis: usize) {
        self.mode = ScreenMode::Normal;
        self.clamp_position(max_main_axis);
    }

    pub fn change_mode_to_search(&mut self, query: String, is_case_sensitive: bool) {
        self.mode = ScreenMode::Search {
            query,
            current: 0,
            entries: vec![],
            is_case_sensitive,
        };
    }

    pub fn search_indexes(&self) -> Option<(usize, usize)> {
        let ScreenMode::Search {
            entries, current, ..
        } = &self.mode
        else {
            return None;
        };

        Some((*current, entries.len()))
    }

    pub fn size(&self) -> Rect {
        self.size
    }

    pub fn set_size(&mut self, size: Rect) {
        self.size = size;
    }

    pub fn clear(&mut self) {
        self.auto_scroll = true;
        self.position = Default::default();
    }

    pub fn disable_auto_scroll(&mut self) {
        self.auto_scroll = false;
    }

    pub fn scroll_horizontal(&mut self, horizontal: isize, max_main_axis: usize) {
        if horizontal < 0 {
            self.position.column = self
                .position
                .column
                .saturating_sub(horizontal.wrapping_abs() as usize);
            if self.position.column == 0 && self.position.line == max_main_axis {
                self.auto_scroll = true;
                return;
            }
        } else {
            self.position.column = self.position.column.saturating_add(horizontal as usize);
        }

        self.auto_scroll = false;
    }

    pub fn update_after_new_lines(&mut self, buffer: &Buffer) {
        if self.auto_scroll {
            let visible_height = self.size.height.saturating_sub(2) as usize;
            let max_main_axis = buffer.len().saturating_sub(visible_height);
            self.position.line = max_main_axis;
        }
    }

    pub fn scroll_vertical(&mut self, vertical: isize, max_main_axis: usize) {
        if vertical < 0 {
            self.position.line = self
                .position
                .line
                .saturating_sub(vertical.wrapping_abs() as usize);
            if self.position.line < max_main_axis {
                self.auto_scroll = false;
            }
        } else {
            self.position.line = self.position.line.saturating_add(vertical as usize);
            self.clamp_position(max_main_axis);
        }
    }

    pub fn jump_to_start(&mut self) {
        self.position.line = 0;
    }

    pub fn jump_to_end(&mut self, max_main_axis: usize) {
        self.position.line = max_main_axis;
        self.auto_scroll = true;
    }

    fn build_block(&self, buffer: &Buffer, save_stats: &SaveStats) -> Block<'_> {
        let file_size = ByteFormat::from(save_stats.file_size());
        let record_indicator = if save_stats.is_recording() {
            " ◉"
        } else {
            ""
        };

        let border_color = if save_stats.is_recording() {
            Color::Red
        } else if save_stats.is_saving() {
            save_stats.save_color()
        } else {
            self.mode.color()
        };
        let border_style = Style::default().fg(border_color);
        let border_type = if self.auto_scroll {
            BorderType::Thick
        } else {
            BorderType::Double
        };

        Block::default()
            .title(format!(
                "[{:03}][{}]{} {}",
                buffer.len(),
                self.decoder.name(),
                record_indicator,
                save_stats.filename()
            ))
            .title(Title::from(format!("[{}]", file_size.0)).alignment(Alignment::Right))
            .borders(Borders::ALL)
            .border_type(border_type)
            .border_style(border_style)
    }

    pub fn draw(&self, buffer: &Buffer, save_stats: &SaveStats, frame: &mut Frame) {
        let block = self.build_block(buffer, save_stats);

        let start = self.position.line;
        let visible_height = self.size.height.saturating_sub(2) as usize;
        let end = start + visible_height;
        let max_width = self.size.width as usize;

        let cropped_lines = buffer
            .get_range(start, end)
            .iter()
            .map(|buffer_line| buffer_line.decode(self.decoder))
            .collect::<Vec<_>>();

        let lines = self
            .mode
            .to_lines(cropped_lines)
            .into_iter()
            .map(|line| Self::crop(line, self.position.column, max_width))
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, self.size);
    }

    fn crop(line: Line, start_x: usize, max_width: usize) -> Line {
        let mut index = 0;
        let mut line_iter = line.iter();
        let timestamp = line_iter.next().cloned().unwrap_or(Span::raw(""));
        let mut line_final: Vec<Span<'_>> = vec![timestamp.clone()];
        /* timestamp + 2 space + border space */
        let max_width = max_width.saturating_sub(timestamp.content.chars().count() + 4);

        line_final.push(line_iter.next().cloned().unwrap());

        for span in line_iter {
            let span_width = span.content.chars().count();

            if index + span_width > start_x {
                let crop_start = start_x.saturating_sub(index);
                let crop_end = (crop_start + max_width).min(span_width);

                let cropped_content = span
                    .content
                    .chars()
                    .skip(crop_start)
                    .take(crop_end.saturating_sub(crop_start))
                    .collect::<String>();
                line_final.push(Span::styled(cropped_content, span.style));
            }

            index += span_width;

            if index >= start_x + max_width {
                break;
            }
        }

        Line::from(line_final)
    }

    pub fn mode_mut(&mut self) -> &mut ScreenMode {
        &mut self.mode
    }

    pub fn decoder(&self) -> ScreenDecoder {
        self.decoder
    }

    fn screen_center_y(&self) -> u16 {
        self.size.height.saturating_sub(2) / 2
    }

    fn jump_to_centered_position(
        &mut self,
        BufferPosition { line, column }: BufferPosition,
        max_main_axis: usize,
    ) {
        self.position.line = line;
        self.position.line = self
            .position
            .line
            .saturating_sub(self.screen_center_y() as usize);
        self.position.column = column;
        self.position.column = self
            .position
            .column
            .saturating_sub(self.size.width as usize / 2);

        self.clamp_position(max_main_axis);
    }

    pub fn jump_to_current_search(&mut self, max_main_axis: usize) {
        let ScreenMode::Search {
            entries, current, ..
        } = &self.mode
        else {
            return;
        };

        let Some(position) = entries.get(*current) else {
            return;
        };

        self.jump_to_centered_position(*position, max_main_axis);
        self.auto_scroll = false;
    }

    pub fn jump_to_next_search(&mut self, max_main_axis: usize) {
        let pos = {
            let ScreenMode::Search {
                current, entries, ..
            } = &mut self.mode
            else {
                return;
            };

            if entries.len() <= 1 {
                return;
            }

            *current = (*current + 1) % entries.len();
            entries[*current]
        };

        self.jump_to_centered_position(pos, max_main_axis);
    }

    pub fn jump_to_previous_search(&mut self, max_main_axis: usize) {
        let pos = {
            let ScreenMode::Search {
                current, entries, ..
            } = &mut self.mode
            else {
                return;
            };

            if entries.len() <= 1 {
                return;
            }

            if *current == 0 {
                *current = entries.len() - 1;
            } else {
                *current -= 1;
            }

            entries[*current]
        };

        self.jump_to_centered_position(pos, max_main_axis);
    }
}

pub enum ScreenMode {
    Normal,
    Search {
        query: String,
        current: usize,
        entries: Vec<BufferPosition>,
        is_case_sensitive: bool,
    },
}

impl ScreenMode {
    pub fn set_query(&mut self, query: String, is_case_sensitive: bool) {
        if let Self::Search {
            query: current_query,
            entries,
            is_case_sensitive: current_is_case_sensitive,
            current,
            ..
        } = self
        {
            *current_query = query;
            *current_is_case_sensitive = is_case_sensitive;
            *current = 0;
            entries.clear();
        }
    }

    pub fn add_entry(&mut self, entry: BufferPosition) {
        if let Self::Search { entries, .. } = self {
            entries.push(entry);
        }
    }

    pub fn update_current(&mut self) {
        if let Self::Search {
            entries, current, ..
        } = self
        {
            if entries.is_empty() {
                *current = 0;
            } else if *current > entries.len() - 1 {
                *current = entries.len() - 1;
            }
        }
    }

    fn color(&self) -> Color {
        match self {
            Self::Normal => Color::White,
            Self::Search { .. } => Color::Yellow,
        }
    }

    fn to_lines(&self, cropped_lines: Vec<BufferLine<String>>) -> Vec<Line<'static>> {
        match self {
            Self::Normal => cropped_lines
                .into_iter()
                .map(|line| self.to_normal_line(line))
                .collect::<Vec<_>>(),
            Self::Search { .. } => cropped_lines
                .into_iter()
                .map(|line| self.to_search_line(line))
                .collect::<Vec<_>>(),
        }
    }

    fn to_normal_line(&self, line: BufferLine<String>) -> Line<'static> {
        let timestamp = Self::timestamp_line(line.timestamp);

        let content = if line.level.is_some() {
            self.log_line(line)
        } else if line.is_tx {
            self.tx_line(line)
        } else {
            self.rx_line(line)
        };

        let content = ANSI::decode(content)
            .into_iter()
            .flat_map(|span| Self::highlight_special_characters(span))
            .collect::<Vec<_>>();

        Line::from(timestamp.into_iter().chain(content).collect::<Vec<_>>())
    }

    fn to_search_line(&self, line: BufferLine<String>) -> Line<'static> {
        let timestamp = Self::timestamp_line(line.timestamp);
        let content = self.search_line(line);

        Line::from(timestamp.into_iter().chain(content).collect::<Vec<_>>())
    }

    fn search_line(&self, line: BufferLine<String>) -> Vec<Span<'static>> {
        let Self::Search {
            query,
            current,
            entries,
            is_case_sensitive,
            ..
        } = self
        else {
            unreachable!();
        };

        let disable_style = Style::default().bg(Color::Black).fg(Color::DarkGray);

        if query.is_empty() {
            let message = ANSI::remove_encoding(line.message);
            return vec![Span::styled(message, disable_style)];
        }

        let highlighted_style = Style::default().bg(Color::Black).fg(Color::Yellow);
        let chosen_style = Style::default().bg(Color::Yellow).fg(Color::Black);
        let query = if *is_case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };

        let message_splitted = line.message.to_special_char(|string| {
            let string = if *is_case_sensitive {
                string.to_string()
            } else {
                string.to_lowercase()
            };

            if string.contains(&query) {
                Some(query.len())
            } else {
                None
            }
        });

        let mut output = vec![];

        for submsg in message_splitted {
            let vec_span = match submsg {
                SpecialCharItem::Plain(submsg) => {
                    let submsg = ANSI::remove_encoding(submsg);
                    vec![Span::styled(submsg, disable_style)]
                }
                SpecialCharItem::Special(query, column) => {
                    let query_pos = BufferPosition {
                        line: line.line,
                        column,
                    };
                    let index = entries.iter().position(|&pos| pos == query_pos);

                    if let Some(i) = index
                        && i == *current
                    {
                        let chosen = Span::styled(query.to_string(), chosen_style);
                        Self::highlight_special_characters(chosen)
                    } else {
                        vec![Span::styled(query.to_string(), highlighted_style)]
                    }
                }
            };

            output.extend(vec_span);
        }

        output
    }

    fn rx_line(&self, line: BufferLine<String>) -> Span<'static> {
        let style = Style::default().bg(Color::Black).fg(Color::White);
        Span::styled(line.message, style)
    }

    fn tx_line(&self, line: BufferLine<String>) -> Span<'static> {
        let style = Style::default().bg(Color::White).fg(Color::Black);
        Span::styled(line.message, style)
    }

    fn log_line(&self, line: BufferLine<String>) -> Span<'static> {
        let level = line.level.unwrap();
        let bg = match level {
            LogLevel::Error => Color::Red,
            LogLevel::Warning => Color::Yellow,
            LogLevel::Success => Color::LightGreen,
            LogLevel::Info => Color::Cyan,
            LogLevel::Debug => Color::DarkGray,
        };
        let fg = Palette::fg(bg);
        let style = Style::default().bg(bg).fg(fg);

        Span::styled(line.message, style)
    }

    fn timestamp_line(timestamp: DateTime<Local>) -> Vec<Span<'static>> {
        let timestamp = timestamp_fmt(timestamp);
        let style = Style::default().fg(Color::DarkGray);

        vec![Span::styled(timestamp, style), Span::raw(" ")]
    }

    fn highlight_special_characters(span: Span) -> Vec<Span> {
        let mut result = vec![];

        let iter = span.content.to_special_char(|string| {
            if let Some(pos) = string.find("\\x")
                && let Some(hex) = string.get(pos + 2..pos + 4)
                && u8::from_str_radix(hex, 16).is_ok()
            {
                return Some(4);
            }

            if string.contains("\\n") || string.contains("\\r") {
                Some(2)
            } else {
                None
            }
        });

        for item in iter {
            match item {
                SpecialCharItem::Plain(plain) => {
                    result.push(Span::styled(plain, span.style));
                }
                SpecialCharItem::Special(special, _) => {
                    result.push(Span::styled(
                        special,
                        span.style.fg(Palette::ascent_fg(
                            span.style.bg.unwrap_or(Color::Reset),
                            span.style.fg.unwrap_or(Color::Reset),
                        )),
                    ));
                }
            }
        }

        result
    }
}

#[derive(Clone, Copy)]
pub enum ScreenDecoder {
    Ascii,
    #[allow(unused)]
    Utf8,
}

impl ScreenDecoder {
    fn name(&self) -> &str {
        match self {
            Self::Ascii => "ASCII",
            Self::Utf8 => "UTF-8",
        }
    }

    pub fn decode(&self, data: &[u8]) -> String {
        match self {
            Self::Ascii => data
                .iter()
                .map(|&b| match b {
                    b'\n' => "\\n".to_string(),
                    b'\r' => "\\r".to_string(),
                    b if (0x20..=0x7e).contains(&b) => (b as char).to_string(),
                    _ => format!("\\x{:02x}", b),
                })
                .collect::<Vec<_>>()
                .join(""),
            Self::Utf8 => {
                let mut result = String::new();
                let mut i = 0;

                while i < data.len() {
                    match str::from_utf8(&data[i..]) {
                        Ok(valid_string) => {
                            result.push_str(valid_string);
                            break;
                        }
                        Err(e) => {
                            let valid_up_to = e.valid_up_to();

                            if valid_up_to > 0 {
                                let valid = str::from_utf8(&data[i..i + valid_up_to]).unwrap();
                                result.push_str(valid);
                                i += valid_up_to;
                            }

                            match e.error_len() {
                                Some(len) => {
                                    let end = (i + len).min(data.len());
                                    for b in &data[i..end] {
                                        result.push_str(&format!("\\x{:02x}", b))
                                    }
                                    i = end;
                                }
                                None => {
                                    for b in &data[i..] {
                                        result.push_str(&format!("\\x{:02x}", b))
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }

                result.replace("\n", "\\n").replace("\r", "\\r")
            }
        }
    }
}
