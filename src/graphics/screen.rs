use crate::{
    graphics::{
        ansi::ANSI,
        buffer::{Buffer, BufferLine, BufferPosition, timestamp_fmt},
        graphics_task::SaveStats,
        palette::Palette,
        selection::{Selection, SelectionPosition},
        special_char::{SpecialCharItem, ToSpecialChar},
    },
    infra::{ByteFormat, LogLevel},
};
use chrono::{DateTime, Local};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, block::Title},
};

pub struct Screen {
    position: BufferPosition,
    auto_scroll: bool,
    mode: ScreenMode,
    decoder: ScreenDecoder,
    size: Rect,
    selection: Option<Selection>,
}

pub struct ScreenPosition {
    pub x: u16,
    pub y: u16,
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
            selection: None,
        }
    }
}

impl Screen {
    const CONTENT_OFFSET_X: usize = 2 + 12; /* border space + timestamp space */
    const CONTENT_OFFSET_Y: usize = 1;

    fn clamp_position(&mut self, max_main_axis: usize) {
        self.position.line = self.position.line.clamp(0, max_main_axis);
        if self.position.line == max_main_axis {
            self.auto_scroll = true;
        }
    }

    pub fn selection(&self) -> Option<&Selection> {
        self.selection.as_ref()
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

    pub fn set_selection(&mut self, start_point: ScreenPosition) {
        let line =
            (start_point.y as usize + self.position.line).saturating_sub(Self::CONTENT_OFFSET_Y);
        let column =
            (start_point.x as usize + self.position.column).saturating_sub(Self::CONTENT_OFFSET_X);

        let start_point = BufferPosition { line, column };

        self.selection = Some(Selection::new(start_point, start_point));
    }

    pub fn set_selection_end(&mut self, end_point: ScreenPosition) {
        if let Some(selection) = &mut self.selection {
            let line =
                (end_point.y as usize + self.position.line).saturating_sub(Self::CONTENT_OFFSET_Y);
            let column = (end_point.x as usize + self.position.column)
                .saturating_sub(Self::CONTENT_OFFSET_X);

            let end_point = BufferPosition { line, column };

            selection.update(end_point);
        }
    }

    pub fn clear(&mut self) {
        self.auto_scroll = true;
        self.position = Default::default();
        self.selection = None;
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

    pub fn draw(
        &self,
        buffer: &Buffer,
        save_stats: &SaveStats,
        frame: &mut Frame,
        system_log_level: LogLevel,
    ) {
        let block = self.build_block(buffer, save_stats);

        let start = self.position.line;
        let visible_height = self.size.height.saturating_sub(2) as usize;
        let end = start + visible_height;
        let max_width = self.size.width as usize;

        let decoded_lines = buffer
            .get_range(start, end)
            .iter()
            .map(|buffer_line| buffer_line.decode(self.decoder))
            .filter(|line| {
                let Some(level) = line.level else {
                    return true;
                };

                level as u32 <= system_log_level as u32
            })
            .collect::<Vec<_>>();

        let lines = self
            .mode
            .to_lines(decoded_lines, self.selection.as_ref())
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

    fn to_lines(
        &self,
        cropped_lines: Vec<BufferLine<String>>,
        selection: Option<&Selection>,
    ) -> Vec<Line<'static>> {
        match self {
            Self::Normal => cropped_lines
                .into_iter()
                .map(|line| self.to_normal_line(line, selection))
                .collect::<Vec<_>>(),
            Self::Search { .. } => cropped_lines
                .into_iter()
                .map(|line| self.to_search_line(line, selection))
                .collect::<Vec<_>>(),
        }
    }

    fn reverse_forward_span(span: Span, span_column: usize, column_split: usize) -> Vec<Span> {
        let span_width = span.content.chars().count();

        if column_split <= span_column {
            vec![span.reversed()]
        } else if column_split >= span_column + span_width {
            vec![span]
        } else {
            let split_point = column_split.saturating_sub(span_column);

            let left_part = span.content.chars().take(split_point).collect::<String>();
            let left_part = Span::styled(left_part, span.style);

            let right_part = span.content.chars().skip(split_point).collect::<String>();
            let right_part = Span::styled(right_part, span.style.reversed());

            vec![left_part, right_part]
        }
    }

    fn reverse_backward_span(span: Span, span_column: usize, column_split: usize) -> Vec<Span> {
        let span_width = span.content.chars().count();

        if column_split <= span_column {
            vec![span]
        } else if column_split >= span_column + span_width {
            vec![span.reversed()]
        } else {
            let split_point = column_split.saturating_sub(span_column);

            let left_part = span.content.chars().take(split_point).collect::<String>();
            let left_part = Span::styled(left_part, span.style.reversed());

            let right_part = span.content.chars().skip(split_point).collect::<String>();
            let right_part = Span::styled(right_part, span.style);

            vec![left_part, right_part]
        }
    }

    fn reversed_middle_span(
        span: Span,
        span_column: usize,
        start_column: usize,
        end_column: usize,
    ) -> Vec<Span> {
        let span_width = span.content.chars().count();

        if end_column <= span_column || start_column >= span_column + span_width {
            vec![span]
        } else {
            let split_start = start_column.saturating_sub(span_column);
            let split_end = end_column.saturating_sub(span_column);

            let left_part = span.content.chars().take(split_start).collect::<String>();
            let left_part = Span::styled(left_part, span.style);

            let middle_part = span
                .content
                .chars()
                .skip(split_start)
                .take(split_end.saturating_sub(split_start))
                .collect::<String>();
            let middle_part = Span::styled(middle_part, span.style.reversed());

            let right_part = span.content.chars().skip(split_end).collect::<String>();
            let right_part = Span::styled(right_part, span.style);

            vec![left_part, middle_part, right_part]
        }
    }

    fn reverse_content<'a>(
        content: Vec<Span<'a>>,
        selection: Option<&Selection>,
        line_number: usize,
    ) -> Vec<Span<'a>> {
        if let Some(selection) = selection {
            let selection = selection.selection_position(line_number);

            match selection {
                super::selection::SelectionPosition::OneLine {
                    start_column,
                    end_column,
                } => {
                    let mut span_column = 0;
                    let mut result = vec![];

                    for span in content {
                        let span_width = span.content.chars().count();

                        let reversed_spans =
                            Self::reversed_middle_span(span, span_column, start_column, end_column);
                        result.extend(reversed_spans);

                        span_column += span_width;
                    }

                    result
                }
                SelectionPosition::Top { column } => {
                    let mut span_column = 0;
                    let mut result = vec![];

                    for span in content {
                        let span_width = span.content.chars().count();

                        let reversed_spans = Self::reverse_forward_span(span, span_column, column);
                        result.extend(reversed_spans);

                        span_column += span_width;
                    }

                    result
                }
                SelectionPosition::Middle => content
                    .into_iter()
                    .map(|span| Span::styled(span.content, span.style.reversed()))
                    .collect::<Vec<_>>(),
                SelectionPosition::Bottom { column } => {
                    let mut span_column = 0;
                    let mut result = vec![];

                    for span in content {
                        let span_width = span.content.chars().count();

                        let reversed_spans = Self::reverse_backward_span(span, span_column, column);
                        result.extend(reversed_spans);

                        span_column += span_width;
                    }

                    result
                }
                SelectionPosition::Outside => content,
            }
        } else {
            content
        }
    }

    fn to_normal_line(
        &self,
        line: BufferLine<String>,
        selection: Option<&Selection>,
    ) -> Line<'static> {
        let is_reversed = selection.is_some_and(|sel| sel.is_inside(line.line));
        let timestamp = Self::timestamp_line(line.timestamp, is_reversed);

        let line_number = line.line;
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
        let content = Self::reverse_content(content, selection, line_number);

        Line::from(timestamp.into_iter().chain(content).collect::<Vec<_>>())
    }

    fn to_search_line(
        &self,
        line: BufferLine<String>,
        selection: Option<&Selection>,
    ) -> Line<'static> {
        let is_reversed = selection.is_some_and(|sel| sel.is_inside(line.line));
        let timestamp = Self::timestamp_line(line.timestamp, is_reversed);
        let line_number = line.line;
        let content = self.search_line(line);
        let content = Self::reverse_content(content, selection, line_number);

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

        let disable_style = Style::default().bg(Color::Reset).fg(Color::DarkGray);
        let message = ANSI::remove_encoding(line.message);

        if query.is_empty() {
            return vec![Span::styled(message, disable_style)];
        }

        let highlighted_style = Style::default().bg(Color::Reset).fg(Color::Yellow);
        let chosen_style = Style::default().bg(Color::Yellow).fg(Color::Black);
        let query = if *is_case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };

        let message_splitted = message.to_special_char(|string| {
            let string = if *is_case_sensitive {
                string.to_string()
            } else {
                string.to_ascii_lowercase()
            };

            string.find(&query).map(|start| {
                let start = string[..start].chars().count();
                (start, query.chars().count()).into()
            })
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

                    if entries.get(*current) == Some(&query_pos) {
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
        let style = Style::default().bg(Color::Reset).fg(Color::Reset);
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

    fn timestamp_line(timestamp: DateTime<Local>, is_reversed: bool) -> Vec<Span<'static>> {
        let timestamp = timestamp_fmt(timestamp);
        let style = if !is_reversed {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().bg(Color::White).fg(Color::DarkGray)
        };

        vec![Span::styled(timestamp, style), Span::raw(" ")]
    }

    fn highlight_special_characters(span: Span) -> Vec<Span> {
        let mut result = vec![];

        let iter = span.content.to_special_char(|string| {
            let mut least_pos = usize::MAX;
            let mut found_pattern = None;

            if let Some(pos) = string.find("\\x")
                && let Some(hex) = string.get(pos + 2..pos + 4)
                && u8::from_str_radix(hex, 16).is_ok()
                && pos < least_pos
            {
                least_pos = pos;
                let pos = string[..pos].chars().count();
                found_pattern = Some((pos, "\\x00".chars().count()).into());
            }

            if let Some(start) = string.find("\\n")
                && start < least_pos
            {
                least_pos = start;
                let start = string[..start].chars().count();
                found_pattern = Some((start, "\\n".chars().count()).into());
            }

            if let Some(start) = string.find("\\r")
                && start < least_pos
            {
                let start = string[..start].chars().count();
                found_pattern = Some((start, "\\r".chars().count()).into());
            }

            found_pattern
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
                    b'\x09' => "    ".to_string(),
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

                result
                    .replace("\n", "\\n")
                    .replace("\r", "\\r")
                    .replace("\t", "    ")
            }
        }
    }
}
