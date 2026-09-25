use crate::{
    graphics::{
        Serialize,
        screen::ScreenDecoder,
        selection::{Selection, SelectionPosition},
    },
    infra::LogLevel,
};
use chrono::{DateTime, Local};
use std::collections::VecDeque;
use std::ops::AddAssign;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic source of per-line identifiers. A line keeps its `id` for its whole
/// lifetime, independent of its position in the buffer, so features that pin to a
/// specific line (bookmarks) survive re-indexing on capacity drop and the
/// re-derivation of the displayed view when the filter changes. Every
/// `BufferLine` is created on the graphics thread, so a `Relaxed` counter is
/// enough — we only need uniqueness, not cross-thread ordering.
static NEXT_LINE_ID: AtomicU64 = AtomicU64::new(0);

fn next_line_id() -> u64 {
    NEXT_LINE_ID.fetch_add(1, Ordering::Relaxed)
}

/// The payload of a stored buffer line. It is reference-counted so the filtered
/// view (`Buffer`) can hold cheap clones of the lines in the full history
/// without duplicating the byte content: cloning a [`BufferLine`] only bumps
/// this `Arc`, it does not copy the message.
pub type LineBytes = Arc<[u8]>;

pub struct Buffer {
    /// A deque, not a `Vec`: dropping the oldest line happens on every single
    /// line once the buffer is at capacity, and `Vec::remove(0)` would memmove
    /// the whole history each time (~1.3MB per line at 20k lines).
    lines: VecDeque<BufferLine<LineBytes>>,
    capacity: usize,
    /// How many lines have been evicted from the front over this buffer's
    /// lifetime. Anything that points at a line *by index* — the scroll offset
    /// of a frozen viewport, an active selection — has to slide by the growth
    /// of this counter to keep pointing at the same content once the capacity
    /// starts rotating (issue #218). Bookmarks pin to the stable
    /// [`BufferLine::id`] instead and need no such fixup.
    evicted: u64,
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity: if capacity == 0 { 1 } else { capacity },
            evicted: 0,
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
            let content = decoder.plain_text(&line.message);
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

    pub fn get_range(
        &self,
        start: usize,
        end: usize,
    ) -> impl Iterator<Item = &BufferLine<LineBytes>> {
        let end = end.min(self.lines.len());
        let start = start.min(end);

        self.lines.range(start..end)
    }

    pub fn iter(&self) -> impl Iterator<Item = &BufferLine<LineBytes>> {
        self.lines.iter()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        // Reset in lockstep with `Screen::clear`, which zeroes the viewport's
        // copy of this counter; the two are always cleared together.
        self.evicted = 0;
    }

    /// Lifetime count of lines dropped from the front — see [`Buffer::evicted`].
    pub fn evicted(&self) -> u64 {
        self.evicted
    }

    fn drop_oldest_if_needed(&mut self) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
            self.evicted += 1;

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
        self.lines.push_back(rhs);
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
    /// Stable identity assigned at creation and preserved across cloning,
    /// re-indexing and filter changes (see [`next_line_id`]). Unlike `line`
    /// (the current index, which shifts as lines drop off the top), this never
    /// changes for a given logical line, so it is what bookmarks pin to.
    pub id: u64,
    pub timestamp: DateTime<Local>,
    pub level: Option<LogLevel>,
    pub message: T,
    pub is_tx: bool,
}

impl BufferLine<LineBytes> {
    pub fn decode(&self, decoder: ScreenDecoder) -> BufferLine<String> {
        BufferLine {
            line: self.line,
            id: self.id,
            timestamp: self.timestamp,
            level: self.level,
            message: decoder.decode(&self.message),
            is_tx: self.is_tx,
        }
    }

    pub fn new_rx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            id: next_line_id(),
            timestamp,
            level: None,
            message: message.into(),
            is_tx: false,
        }
    }

    pub fn new_tx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            id: next_line_id(),
            timestamp,
            level: None,
            message: message.into(),
            is_tx: true,
        }
    }

    pub fn new_log(timestamp: DateTime<Local>, level: LogLevel, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            id: next_line_id(),
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
        let message = ScreenDecoder::default().decode(&self.message);

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

        let content = buffer.get_selection_content(&selection, ScreenDecoder::default());

        assert_eq!(content, "Red");
    }

    // Issue #239: the copy is what the screen shows, so the selection columns
    // are counted over the formatted bytes.
    #[test]
    fn selection_copies_bytes_in_the_display_format() {
        use crate::graphics::screen::HexFormat;

        // Rendered as "idA5A6"; "A5A6" starts at visible column 2.
        let buffer = buffer_from(&[b"id\xa5\xa6"]);
        let selection = Selection::new(pos(0, 2), pos(0, 6));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::new(HexFormat::Bare));

        assert_eq!(content, "A5A6");
    }

    #[test]
    fn selection_from_line_start_skips_leading_ansi() {
        // A leading color code must not shift the visible columns.
        let buffer = buffer_from(&[b"\x1b[32mgreen\x1b[0m"]);
        let selection = Selection::new(pos(0, 0), pos(0, 5));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::default());

        assert_eq!(content, "green");
    }

    // Issue #218: the eviction count is what lets a frozen viewport (and an
    // active selection) slide with the content when the capacity rotates.
    #[test]
    fn nothing_is_evicted_below_capacity() {
        let mut buffer = Buffer::new(4);
        for _ in 0..4 {
            buffer += BufferLine::new_rx(Local::now(), b"x".to_vec());
        }

        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer.evicted(), 0);
    }

    #[test]
    fn one_line_is_evicted_per_line_past_capacity() {
        let mut buffer = Buffer::new(4);
        for _ in 0..7 {
            buffer += BufferLine::new_rx(Local::now(), b"x".to_vec());
        }

        assert_eq!(buffer.len(), 4);
        assert_eq!(buffer.evicted(), 3);
    }

    #[test]
    fn the_oldest_line_is_the_one_dropped_and_the_rest_are_reindexed() {
        let mut buffer = Buffer::new(3);
        for i in 0..4 {
            buffer += BufferLine::new_rx(Local::now(), format!("L{i}").into_bytes());
        }

        let lines = buffer
            .iter()
            .map(|line| {
                (
                    line.line,
                    String::from_utf8_lossy(&line.message).to_string(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            lines,
            vec![
                (0, "L1".to_string()),
                (1, "L2".to_string()),
                (2, "L3".to_string())
            ]
        );
    }

    #[test]
    fn clear_resets_the_eviction_count() {
        let mut buffer = Buffer::new(2);
        for _ in 0..5 {
            buffer += BufferLine::new_rx(Local::now(), b"x".to_vec());
        }
        assert_eq!(buffer.evicted(), 3);

        buffer.clear();

        assert_eq!(buffer.evicted(), 0);
    }

    #[test]
    fn multi_line_selection_strips_ansi_on_every_line() {
        let buffer = buffer_from(&[b"\x1b[31mfoo", b"bar\x1b[0m"]);
        // Top line from column 1, bottom line up to column 2.
        let selection = Selection::new(pos(0, 1), pos(1, 2));

        let content = buffer.get_selection_content(&selection, ScreenDecoder::default());

        assert_eq!(content, "ooba");
    }
}
