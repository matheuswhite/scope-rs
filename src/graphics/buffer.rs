use crate::{
    graphics::{
        Serialize,
        ansi::ANSI,
        screen::ScreenDecoder,
        selection::{Selection, SelectionPosition},
    },
    infra::LogLevel,
};
use chrono::{DateTime, Local};
use std::ops::AddAssign;
use std::sync::Arc;

/// The payload of a stored buffer line. It is reference-counted so the filtered
/// view (`Buffer`) can hold cheap clones of the lines in the full history
/// without duplicating the byte content: cloning a [`BufferLine`] only bumps
/// this `Arc`, it does not copy the message.
pub type LineBytes = Arc<[u8]>;

pub struct Buffer {
    lines: Vec<BufferLine<LineBytes>>,
    capacity: usize,
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Vec::new(),
            capacity: if capacity == 0 { 1 } else { capacity },
        }
    }

    pub fn get_selection_content(&self, selection: &Selection, decoder: ScreenDecoder) -> String {
        let (start, end) = selection.ordered_positions();
        let mut result = vec![];

        for line in self.get_range(start.line, end.line + 1) {
            // Strip ANSI escape codes before slicing: selection columns come from
            // the rendered screen, where `ANSI::decode` has already removed those
            // codes (they paint color, not glyphs). Slicing the still-encoded
            // string would misalign every column past an ANSI code and leak the
            // raw `\x1b[..m` text into the clipboard (issue #180).
            let content = ANSI::remove_encoding(decoder.decode(&line.message));
            let content = content.as_str().chars();

            match selection.selection_position(line.line) {
                SelectionPosition::OneLine {
                    start_column,
                    end_column,
                } => {
                    result.push(
                        content
                            .skip(start_column)
                            .take(end_column - start_column)
                            .collect::<String>(),
                    );
                }
                SelectionPosition::Top { column } => {
                    result.push(content.skip(column).collect::<String>());
                }
                SelectionPosition::Bottom { column } => {
                    let content_len = content.as_str().chars().count();
                    let column = column.clamp(0, content_len);
                    result.push(content.take(column).collect::<String>());
                }
                SelectionPosition::Middle => {
                    result.push(content.collect::<String>());
                }
                SelectionPosition::Outside => {}
            }
        }

        result.join("").replace("\\r", "\r").replace("\\n", "\n")
    }

    pub fn get_range(&self, start: usize, end: usize) -> &[BufferLine<LineBytes>] {
        let end = end.min(self.lines.len());
        let start = start.min(end);

        &self.lines[start..end]
    }

    pub fn iter(&self) -> impl Iterator<Item = &BufferLine<LineBytes>> {
        self.lines.iter()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    fn drop_oldest_if_needed(&mut self) {
        if self.lines.len() == self.capacity {
            self.lines.remove(0);

            for (index, line) in self.lines.iter_mut().enumerate() {
                line.line = index;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

impl AddAssign<BufferLine<LineBytes>> for Buffer {
    fn add_assign(&mut self, mut rhs: BufferLine<LineBytes>) {
        self.drop_oldest_if_needed();

        rhs.line = self.lines.len();
        self.lines.push(rhs);
    }
}

impl AddAssign<Vec<BufferLine<LineBytes>>> for Buffer {
    fn add_assign(&mut self, mut rhs: Vec<BufferLine<LineBytes>>) {
        for line in rhs.drain(..) {
            *self += line;
        }
    }
}

#[derive(Clone)]
pub struct BufferLine<T>
where
    T: AsRef<[u8]>,
{
    pub line: usize,
    pub timestamp: DateTime<Local>,
    pub level: Option<LogLevel>,
    pub message: T,
    pub is_tx: bool,
}

impl BufferLine<LineBytes> {
    pub fn decode(&self, decoder: ScreenDecoder) -> BufferLine<String> {
        BufferLine {
            line: self.line,
            timestamp: self.timestamp,
            level: self.level,
            message: decoder.decode(&self.message),
            is_tx: self.is_tx,
        }
    }

    pub fn new_rx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: None,
            message: message.into(),
            is_tx: false,
        }
    }

    pub fn new_tx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: None,
            message: message.into(),
            is_tx: true,
        }
    }

    pub fn new_log(timestamp: DateTime<Local>, level: LogLevel, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: Some(level),
            message: message.into(),
            is_tx: false,
        }
    }

    pub fn timestamp(&self) -> DateTime<Local> {
        self.timestamp
    }
}

impl Serialize for BufferLine<LineBytes> {
    fn serialize(&self) -> String {
        let message = ScreenDecoder::Ascii.decode(&self.message);

        if let Some(level) = self.level {
            let log_level = match level {
                LogLevel::Error => "ERR",
                LogLevel::Warning => "WRN",
                LogLevel::Success => " OK",
                LogLevel::Info => "INF",
                LogLevel::Debug => "DBG",
            };

            return format!(
                "[{}][{}] {}",
                timestamp_fmt(self.timestamp),
                log_level,
                message
            );
        }

        if self.is_tx {
            format!("[{}][ =>] {}", timestamp_fmt(self.timestamp), message)
        } else {
            format!("[{}][ <=] {}", timestamp_fmt(self.timestamp), message)
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct BufferPosition {
    pub line: usize,
    pub column: usize,
}

impl PartialOrd for BufferPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.line != other.line {
            self.line.partial_cmp(&other.line)
        } else {
            self.column.partial_cmp(&other.column)
        }
    }
}

pub fn timestamp_fmt(timestamp: DateTime<Local>) -> String {
    timestamp.format("%H:%M:%S.%3f").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer_from(lines: &[&[u8]]) -> Buffer {
        let mut buffer = Buffer::new(lines.len().max(1));
        for bytes in lines {
            buffer += BufferLine::new_rx(Local::now(), bytes.to_vec());
        }
        buffer
    }

    fn pos(line: usize, column: usize) -> BufferPosition {
        BufferPosition { line, column }
    }

    // The filtered display buffer holds clones of the full-history lines. Those
    // clones must share the byte payload (the message is an Arc), not copy it —
    // otherwise every shown line would be stored twice. Guard that here so the
    // payload type can't silently regress to an owned buffer.
    #[test]
    fn cloning_a_line_shares_the_payload_without_copying() {
        let line = BufferLine::new_rx(Local::now(), b"a shared payload".to_vec());
        assert_eq!(Arc::strong_count(&line.message), 1);

        let clone = line.clone();
        // Both handles point at the same allocation.
        assert_eq!(Arc::strong_count(&line.message), 2);
        assert!(Arc::ptr_eq(&line.message, &clone.message));
    }

    // Issue #180: copying a region that contains ANSI color codes used to leak
    // the raw `\x1b[..m` text and misalign every column past the code, because
    // selection columns come from the rendered screen (where ANSI codes paint
    // color and occupy no glyphs) but the copy sliced the still-encoded string.
    #[test]
    fn one_line_selection_strips_ansi_and_aligns_columns() {
        // Rendered as "Hello Red World"; "Red" starts at visible column 6.
        let buffer = buffer_from(&[b"Hello \x1b[31mRed\x1b[0m World"]);
        let selection = Selection::new(pos(0, 6), pos(0, 9));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::Ascii);

        assert_eq!(content, "Red");
    }

    #[test]
    fn selection_from_line_start_skips_leading_ansi() {
        // A leading color code must not shift the visible columns.
        let buffer = buffer_from(&[b"\x1b[32mgreen\x1b[0m"]);
        let selection = Selection::new(pos(0, 0), pos(0, 5));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::Ascii);

        assert_eq!(content, "green");
    }

    #[test]
    fn multi_line_selection_strips_ansi_on_every_line() {
        let buffer = buffer_from(&[b"\x1b[31mfoo", b"bar\x1b[0m"]);
        // Top line from column 1, bottom line up to column 2.
        let selection = Selection::new(pos(0, 1), pos(1, 2));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::Ascii);

        assert_eq!(content, "ooba");
    }
}
