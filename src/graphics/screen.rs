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
    layout::{Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};
use regex::{Regex, RegexBuilder};
use std::collections::BTreeSet;

pub struct Screen {
    position: BufferPosition,
    auto_scroll: bool,
    mode: ScreenMode,
    decoder: ScreenDecoder,
    size: Rect,
    selection: Option<Selection>,
    /// Bookmarked lines, tracked by their stable [`BufferLine::id`](crate::graphics::buffer::BufferLine)
    /// so they follow a line through capacity drops and filter changes rather
    /// than pinning to a positional index that shifts underneath them.
    bookmarks: BTreeSet<u64>,
    /// Id of the bookmark the last `Tab`/`Shift+Tab` jump landed on, so repeated
    /// presses advance from where the user is instead of re-deriving an anchor
    /// from the (possibly clamped) scroll offset. Cleared when that bookmark is
    /// removed or the screen is cleared.
    current_bookmark: Option<u64>,
    /// Value of [`Buffer::evicted`](crate::graphics::buffer::Buffer::evicted)
    /// the last time the viewport was reconciled with the buffer. Its growth is
    /// how many lines have dropped off the front since, which is exactly how
    /// far a frozen viewport (and any selection) has to slide to stay on the
    /// same content — see [`Screen::update_after_new_lines`].
    evicted_seen: u64,
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
            bookmarks: BTreeSet::new(),
            current_bookmark: None,
            evicted_seen: 0,
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

    pub fn change_mode_to_search(
        &mut self,
        query: String,
        is_case_sensitive: bool,
        is_regex: bool,
    ) {
        self.mode = ScreenMode::Search {
            current: 0,
            entries: vec![],
            matcher: SearchMatcher::build(&query, is_case_sensitive, is_regex),
        };
    }

    /// True while a non-empty search query is active — see
    /// [`ScreenMode::is_searching`].
    pub fn is_searching(&self) -> bool {
        self.mode.is_searching()
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

    pub fn set_size(&mut self, size: Rect, buffer_len: usize) {
        self.size = size;
        /* A resize that grows the viewport shrinks the max scroll offset. Re-clamp
         * position.line so it never sits past the new range; otherwise a stale
         * offset would desync selection mapping and small-step scroll inputs (and
         * the render path / scrollbar, which derive from it) until the next event
         * happens to re-clamp it. clamp_position also re-enables auto-scroll when
         * the clamp lands on the bottom, matching the rest of the scroll logic. */
        let visible_height = size.height.saturating_sub(2) as usize;
        let max_main_axis = buffer_len.saturating_sub(visible_height);
        self.clamp_position(max_main_axis);
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
        self.bookmarks.clear();
        self.current_bookmark = None;
        // The buffer is cleared together with the screen, resetting its own
        // counter, so the two stay in step.
        self.evicted_seen = 0;
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

    /// Reconciles the viewport with the buffer after new lines have landed.
    ///
    /// With auto-scroll on the viewport simply follows the bottom. With it off —
    /// the user scrolled up to read — the viewport has to stay on the *lines* it
    /// is showing, not on their indices: once the buffer is at capacity every
    /// new line evicts the oldest one and re-indexes the rest, so a fixed offset
    /// would let the content slide upwards under a frozen viewport, which is
    /// issue #218 (and why it only showed up after the first `--capacity` lines).
    /// Sliding the offset by the number of evictions keeps the same lines on
    /// screen.
    ///
    /// Offset 0 is the floor: from there the lines being read are themselves the
    /// ones being dropped, and no offset can hold content the buffer no longer
    /// has. That is the limit `--capacity` buys.
    pub fn update_after_new_lines(&mut self, buffer: &Buffer) {
        let evicted = buffer.evicted();
        // Saturating, so a buffer cleared without the screen (which resets the
        // counter) re-syncs instead of underflowing.
        let dropped = evicted.saturating_sub(self.evicted_seen) as usize;
        self.evicted_seen = evicted;

        // A selection is positional too, so it follows the same slide whether or
        // not the viewport is frozen; one whose every line is gone is dropped.
        if dropped > 0
            && let Some(selection) = &mut self.selection
            && !selection.shift_up(dropped)
        {
            self.selection = None;
        }

        if self.auto_scroll {
            let visible_height = self.size.height.saturating_sub(2) as usize;
            let max_main_axis = buffer.len().saturating_sub(visible_height);
            self.position.line = max_main_axis;
        } else {
            self.position.line = self.position.line.saturating_sub(dropped);
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

    fn border_color(&self, save_stats: &SaveStats) -> Color {
        if save_stats.is_recording() {
            Color::Red
        } else if save_stats.is_saving() {
            save_stats.save_color()
        } else {
            self.mode.color()
        }
    }

    fn build_block(&self, buffer: &Buffer, save_stats: &SaveStats) -> Block<'_> {
        let file_size = ByteFormat::from(save_stats.file_size());
        let record_indicator = if save_stats.is_recording() {
            " ◉"
        } else {
            ""
        };

        let border_color = self.border_color(save_stats);
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
            .title(Line::from(format!("[{}]", file_size.0)).right_aligned())
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

        /* position.line is kept within the scroll range by scroll/new-line events
         * and re-clamped on resize in set_size, so it is always a valid top line. */
        let start = self.position.line;
        let visible_height = self.size.height.saturating_sub(2) as usize;
        let end = start + visible_height;
        let max_width = self.size.width as usize;

        let decoded_lines = buffer
            .get_range(start, end)
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
            .to_lines(
                decoded_lines,
                self.selection.as_ref(),
                &self.bookmarks,
                self.current_bookmark,
            )
            .into_iter()
            .map(|line| Self::crop(line, self.position.column, max_width))
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(lines).block(block);
        frame.render_widget(paragraph, self.size);

        self.draw_scrollbar(buffer, save_stats, frame, start, visible_height);
    }

    /// Draws a vertical scrollbar over the right border, indicating the current
    /// scroll position within the buffer. It is only shown when the buffer has
    /// more lines than fit in the viewport.
    fn draw_scrollbar(
        &self,
        buffer: &Buffer,
        save_stats: &SaveStats,
        frame: &mut Frame,
        top: usize,
        visible_height: usize,
    ) {
        let total = buffer.len();
        if visible_height == 0 || total <= visible_height {
            return;
        }

        /* ScrollbarState's content_length is the number of distinct scroll
         * positions, not the line count: the thumb only reaches the bottom when
         * `position` equals `content_length - 1`. Our top line maxes out at
         * `total - visible_height` (the last full screen), so content_length must
         * be `max_offset + 1`. Passing `total` here would cap the thumb partway
         * down the track. viewport_content_length keeps the thumb sized to the
         * visible fraction. `top` is the clamped top line shared with the rendered
         * content, so the thumb stays consistent with what is on screen. */
        let max_offset = total - visible_height;
        let mut state = ScrollbarState::new(max_offset + 1)
            .position(top)
            .viewport_content_length(visible_height);

        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("▲"))
            .end_symbol(Some("▼"))
            .thumb_style(Style::default().fg(self.border_color(save_stats)))
            .track_style(Style::default().fg(Color::DarkGray));

        /* inset vertically so the arrows fall inside the block's borders */
        let area = self.size.inner(Margin {
            vertical: 1,
            horizontal: 0,
        });

        frame.render_stateful_widget(scrollbar, area, &mut state);
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

        let Some(hit) = entries.get(*current) else {
            return;
        };

        self.jump_to_centered_position(hit.position, max_main_axis);
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
            entries[*current].position
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

            entries[*current].position
        };

        self.jump_to_centered_position(pos, max_main_axis);
    }

    /// Toggle the bookmark on the line under `point` (a right-click). A click on
    /// the borders or the empty area past the last line is a no-op. The bookmark
    /// pins to the line's stable id, so it stays on the same line even after the
    /// buffer rotates or the filter is changed.
    pub fn toggle_bookmark(&mut self, buffer: &Buffer, point: ScreenPosition) {
        // Content starts one row below the top border; clicks on it (or above)
        // do not map to a line.
        let row = point.y as usize;
        if row < Self::CONTENT_OFFSET_Y {
            return;
        }

        let line = (row + self.position.line) - Self::CONTENT_OFFSET_Y;
        let Some(buffer_line) = buffer.get_range(line, line + 1).next() else {
            return;
        };
        let id = buffer_line.id;

        if self.bookmarks.remove(&id) {
            if self.current_bookmark == Some(id) {
                self.current_bookmark = None;
            }
        } else {
            self.bookmarks.insert(id);
        }
    }

    /// The bookmarked lines currently present in the displayed buffer, as
    /// `(line index, id)` pairs ordered top-to-bottom. Derived on demand: a
    /// bookmark whose line is filtered out or has scrolled off simply does not
    /// appear here, and comes back if the line does.
    fn bookmark_positions(&self, buffer: &Buffer) -> Vec<(usize, u64)> {
        buffer
            .iter()
            .filter(|line| self.bookmarks.contains(&line.id))
            .map(|line| (line.line, line.id))
            .collect()
    }

    pub fn jump_to_next_bookmark(&mut self, buffer: &Buffer, max_main_axis: usize) {
        let positions = self.bookmark_positions(buffer);
        let anchor = self.position.line + self.screen_center_y() as usize;
        let Some(index) =
            Self::next_bookmark_index(&positions, self.current_bookmark, anchor, true)
        else {
            return;
        };
        self.focus_bookmark(positions[index], max_main_axis);
    }

    pub fn jump_to_previous_bookmark(&mut self, buffer: &Buffer, max_main_axis: usize) {
        let positions = self.bookmark_positions(buffer);
        let anchor = self.position.line + self.screen_center_y() as usize;
        let Some(index) =
            Self::next_bookmark_index(&positions, self.current_bookmark, anchor, false)
        else {
            return;
        };
        self.focus_bookmark(positions[index], max_main_axis);
    }

    /// Which bookmark to jump to next. When `current` is still in view, step one
    /// entry from it (wrapping around the ends); otherwise start from where the
    /// user is looking — the first bookmark past the `anchor` line when going
    /// forward, the last before it when going backward — wrapping if there is
    /// none on that side. Returns `None` only when there are no bookmarks.
    fn next_bookmark_index(
        positions: &[(usize, u64)],
        current: Option<u64>,
        anchor: usize,
        forward: bool,
    ) -> Option<usize> {
        let len = positions.len();
        if len == 0 {
            return None;
        }

        if let Some(current) = current
            && let Some(i) = positions.iter().position(|(_, id)| *id == current)
        {
            return Some(if forward {
                (i + 1) % len
            } else {
                (i + len - 1) % len
            });
        }

        Some(if forward {
            positions
                .iter()
                .position(|(line, _)| *line > anchor)
                .unwrap_or(0)
        } else {
            positions
                .iter()
                .rposition(|(line, _)| *line < anchor)
                .unwrap_or(len - 1)
        })
    }

    fn focus_bookmark(&mut self, (line, id): (usize, u64), max_main_axis: usize) {
        self.current_bookmark = Some(id);
        self.jump_to_centered_position(BufferPosition { line, column: 0 }, max_main_axis);
        // Parking on a bookmark should hold that view; keep new lines from
        // yanking it back to the bottom even when the bookmark is the last line.
        self.auto_scroll = false;
    }
}

/// Turns a search query into the positions it matches on a line. Built once
/// per search change (not per rendered line) so a regex compiles only once.
pub enum SearchMatcher {
    /// Empty query, or a regex that failed to compile: matches nothing.
    Empty,
    /// Literal substring search, with case folding when not case-sensitive.
    Plain {
        needle: String,
        is_case_sensitive: bool,
    },
    /// The query compiled as a regular expression (case folding is baked into
    /// the compiled regex).
    Regex(Regex),
}

impl SearchMatcher {
    fn build(query: &str, is_case_sensitive: bool, is_regex: bool) -> Self {
        if query.is_empty() {
            return Self::Empty;
        }

        if is_regex {
            match RegexBuilder::new(query)
                .case_insensitive(!is_case_sensitive)
                .build()
            {
                Ok(regex) => Self::Regex(regex),
                // An in-progress or otherwise invalid pattern matches nothing;
                // the search bar turning red (0 matches) signals it to the user.
                Err(_) => Self::Empty,
            }
        } else {
            Self::Plain {
                needle: query.to_string(),
                is_case_sensitive,
            }
        }
    }

    fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    /// Non-overlapping matches within `line`, each as `(char_start, char_len)`.
    /// The offsets are in characters (not bytes) so they line up with the
    /// screen columns used for highlighting and navigation.
    fn matches(&self, line: &str) -> Vec<(usize, usize)> {
        match self {
            Self::Empty => vec![],
            Self::Plain {
                needle,
                is_case_sensitive,
            } => {
                let (haystack, needle) = if *is_case_sensitive {
                    (line.to_string(), needle.clone())
                } else {
                    (line.to_ascii_lowercase(), needle.to_ascii_lowercase())
                };
                let needle_chars = needle.chars().count();

                let mut result = vec![];
                let mut start_byte = 0;
                while let Some(rel_byte) = haystack[start_byte..].find(&needle) {
                    let abs_byte = start_byte + rel_byte;
                    let column = haystack[..abs_byte].chars().count();
                    result.push((column, needle_chars));
                    start_byte = abs_byte + needle.len();
                }
                result
            }
            Self::Regex(regex) => {
                let mut result = vec![];
                for m in regex.find_iter(line) {
                    // Skip zero-width matches (e.g. a trailing `.*` or `a*`):
                    // there is nothing to highlight and they only inflate the
                    // match count.
                    if m.start() == m.end() {
                        continue;
                    }
                    let column = line[..m.start()].chars().count();
                    let len = line[m.start()..m.end()].chars().count();
                    result.push((column, len));
                }
                result
            }
        }
    }
}

/// One hit of the active search: where it sits in the buffer right now
/// (`position`) and the stable id of the line it is on (`line_id`).
///
/// The hit list is re-scanned from the buffer whenever new lines land, and the
/// buffer re-indexes its lines as it rotates, so `position` alone cannot
/// identify the hit the user navigated to across a re-scan. The id can, because
/// it never changes for a given line (issue #218).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SearchHit {
    pub position: BufferPosition,
    pub line_id: u64,
}

pub enum ScreenMode {
    Normal,
    Search {
        current: usize,
        entries: Vec<SearchHit>,
        matcher: SearchMatcher,
    },
}

impl ScreenMode {
    pub fn set_query(&mut self, query: String, is_case_sensitive: bool, is_regex: bool) {
        if let Self::Search {
            entries,
            matcher,
            current,
            ..
        } = self
        {
            *matcher = SearchMatcher::build(&query, is_case_sensitive, is_regex);
            *current = 0;
            entries.clear();
        }
    }

    pub fn add_entry(&mut self, entry: SearchHit) {
        if let Self::Search { entries, .. } = self {
            entries.push(entry);
        }
    }

    /// True while a non-empty query is active, i.e. the hit list means
    /// something and is worth re-scanning for.
    pub fn is_searching(&self) -> bool {
        matches!(self, Self::Search { matcher, .. } if !matcher.is_empty())
    }

    /// The hit the user is currently on.
    fn current_hit(&self) -> Option<SearchHit> {
        let Self::Search {
            entries, current, ..
        } = self
        else {
            return None;
        };

        entries.get(*current).copied()
    }

    /// Empties the hit list ahead of a re-scan, handing back the hit the user
    /// was on so [`ScreenMode::restore_current`] can find it again.
    pub fn take_hits_for_rescan(&mut self) -> Option<SearchHit> {
        let previous = self.current_hit();

        if let Self::Search { entries, .. } = self {
            entries.clear();
        }

        previous
    }

    /// Puts the navigation position back where the user left it after a
    /// re-scan: on the very same hit when it is still there, otherwise on the
    /// first hit at or after its line (ids grow monotonically, so that is the
    /// next hit in time), and on the first hit when neither applies.
    pub fn restore_current(&mut self, previous: Option<SearchHit>) {
        let Self::Search {
            entries, current, ..
        } = self
        else {
            return;
        };

        *current = previous
            .and_then(|prev| {
                entries
                    .iter()
                    .position(|hit| *hit == prev)
                    .or_else(|| entries.iter().position(|hit| hit.line_id >= prev.line_id))
            })
            .unwrap_or(0);
    }

    /// Match positions of the active search query within `line`, as
    /// `(char_start, char_len)`. Empty outside Search mode.
    pub fn search_matches(&self, line: &str) -> Vec<(usize, usize)> {
        match self {
            Self::Search { matcher, .. } => matcher.matches(line),
            Self::Normal => vec![],
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
        bookmarks: &BTreeSet<u64>,
        current_bookmark: Option<u64>,
    ) -> Vec<Line<'static>> {
        match self {
            Self::Normal => cropped_lines
                .into_iter()
                .map(|line| self.to_normal_line(line, selection, bookmarks, current_bookmark))
                .collect::<Vec<_>>(),
            Self::Search { .. } => cropped_lines
                .into_iter()
                .map(|line| self.to_search_line(line, selection, bookmarks, current_bookmark))
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
        bookmarks: &BTreeSet<u64>,
        current_bookmark: Option<u64>,
    ) -> Line<'static> {
        let is_reversed = selection.is_some_and(|sel| sel.is_inside(line.line));
        let is_bookmarked = bookmarks.contains(&line.id);
        let is_current_bookmark = current_bookmark == Some(line.id);
        let timestamp = Self::timestamp_line(
            line.timestamp,
            is_reversed,
            is_bookmarked,
            is_current_bookmark,
        );

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
        bookmarks: &BTreeSet<u64>,
        current_bookmark: Option<u64>,
    ) -> Line<'static> {
        let is_reversed = selection.is_some_and(|sel| sel.is_inside(line.line));
        let is_bookmarked = bookmarks.contains(&line.id);
        let is_current_bookmark = current_bookmark == Some(line.id);
        let timestamp = Self::timestamp_line(
            line.timestamp,
            is_reversed,
            is_bookmarked,
            is_current_bookmark,
        );
        let line_number = line.line;
        let content = self.search_line(line);
        let content = Self::reverse_content(content, selection, line_number);

        Line::from(timestamp.into_iter().chain(content).collect::<Vec<_>>())
    }

    fn search_line(&self, line: BufferLine<String>) -> Vec<Span<'static>> {
        let Self::Search {
            current,
            entries,
            matcher,
            ..
        } = self
        else {
            unreachable!(
                "search_line should only be called in Search mode. This is a bug. Please, report it."
            );
        };

        let disable_style = Style::default().bg(Color::Reset).fg(Color::DarkGray);
        let message = ANSI::remove_encoding(line.message);

        if matcher.is_empty() {
            return vec![Span::styled(message, disable_style)];
        }

        // Match on the whole (decoded, ANSI-stripped) line so regex anchors like
        // `^`, `$` and `\b` behave against the real line boundaries. The same
        // `message` and matcher feed `update_search_state`, so the char columns
        // computed here line up with the navigation entries below.
        let matches = matcher.matches(&message);
        if matches.is_empty() {
            return vec![Span::styled(message, disable_style)];
        }

        let highlighted_style = Style::default().bg(Color::Reset).fg(Color::Yellow);
        let chosen_style = Style::default().bg(Color::Yellow).fg(Color::Black);

        let chars = message.chars().collect::<Vec<_>>();
        let mut output = vec![];
        let mut cursor = 0;

        for (start, len) in matches {
            if start > cursor {
                let plain = chars[cursor..start].iter().collect::<String>();
                output.push(Span::styled(plain, disable_style));
            }

            let matched = chars[start..start + len].iter().collect::<String>();
            // By line id, not index: the hit list is re-scanned as lines land
            // and the buffer re-indexes as it rotates.
            let is_chosen = entries
                .get(*current)
                .is_some_and(|hit| hit.line_id == line.id && hit.position.column == start);

            if is_chosen {
                let chosen = Span::styled(matched, chosen_style);
                output.extend(Self::highlight_special_characters(chosen));
            } else {
                output.push(Span::styled(matched, highlighted_style));
            }

            cursor = start + len;
        }

        if cursor < chars.len() {
            let plain = chars[cursor..].iter().collect::<String>();
            output.push(Span::styled(plain, disable_style));
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

    fn timestamp_line(
        timestamp: DateTime<Local>,
        is_reversed: bool,
        is_bookmarked: bool,
        is_current_bookmark: bool,
    ) -> Vec<Span<'static>> {
        let timestamp = timestamp_fmt(timestamp);
        // Two-tier bookmark styling so `Tab` navigation is legible: the bookmark
        // the cursor is parked on gets the full yellow-background highlight,
        // while every other bookmark keeps a subtler yellow foreground on the
        // normal background. Both win over the transient selection highlight, so
        // a bookmark stays marked even while a selection is dragged across it.
        let style = if is_current_bookmark {
            Style::default().bg(Color::Yellow).fg(Color::Black)
        } else if is_bookmarked {
            Style::default().fg(Color::Yellow)
        } else if is_reversed {
            Style::default().bg(Color::White).fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::DarkGray)
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

#[cfg(test)]
mod tests {
    use super::SearchMatcher;

    // Regex search on each line (issue #209).

    #[test]
    fn plain_case_sensitive_finds_all_occurrences() {
        let matcher = SearchMatcher::build("ab", true, false);
        assert_eq!(matcher.matches("ab_ab_AB"), vec![(0, 2), (3, 2)]);
    }

    #[test]
    fn plain_case_insensitive_matches_regardless_of_case() {
        let matcher = SearchMatcher::build("ab", false, false);
        assert_eq!(matcher.matches("ab_ab_AB"), vec![(0, 2), (3, 2), (6, 2)]);
    }

    #[test]
    fn columns_are_character_offsets_not_bytes() {
        // "á" is two bytes but one column; the two "X" matches must land on
        // char columns 1 and 3, not byte offsets 2 and 5.
        let matcher = SearchMatcher::build("X", true, false);
        assert_eq!(matcher.matches("áXbX"), vec![(1, 1), (3, 1)]);
    }

    #[test]
    fn regex_matches_pattern_with_char_columns_and_lengths() {
        let matcher = SearchMatcher::build(r"\d+", true, true);
        assert_eq!(matcher.matches("ab12cde345"), vec![(2, 2), (7, 3)]);
    }

    #[test]
    fn regex_case_insensitive_flag_is_honored() {
        let sensitive = SearchMatcher::build("ERR", true, true);
        assert!(sensitive.matches("an err happened").is_empty());

        let insensitive = SearchMatcher::build("ERR", false, true);
        assert_eq!(insensitive.matches("an err happened"), vec![(3, 3)]);
    }

    #[test]
    fn regex_anchor_matches_only_at_line_start() {
        let matcher = SearchMatcher::build("^ab", true, true);
        assert_eq!(matcher.matches("abcab"), vec![(0, 2)]);
        assert!(matcher.matches("xabcab").is_empty());
    }

    #[test]
    fn regex_zero_width_matches_are_skipped() {
        // A trailing `.*` and empty `a*` runs would otherwise inflate the match
        // count with nothing to highlight.
        let matcher = SearchMatcher::build("a*", true, true);
        assert_eq!(matcher.matches("baa"), vec![(1, 2)]);
    }

    #[test]
    fn invalid_regex_matches_nothing() {
        let matcher = SearchMatcher::build("(unclosed", true, true);
        assert!(matcher.is_empty());
        assert!(matcher.matches("(unclosed group here").is_empty());
    }

    #[test]
    fn empty_query_matches_nothing() {
        assert!(SearchMatcher::build("", false, false).is_empty());
        assert!(SearchMatcher::build("", false, true).is_empty());
    }

    // A viewport frozen by scrolling up must stay on its lines while the buffer
    // rotates underneath it (issue #218).
    mod frozen_viewport {
        use super::super::Screen;
        use crate::graphics::buffer::{Buffer, BufferLine, BufferPosition};
        use crate::graphics::selection::Selection;
        use chrono::Local;
        use ratatui::layout::Rect;

        /// A buffer filled to `capacity`, each line labelled with its ordinal so
        /// a test can tell which lines the viewport is showing.
        fn full_buffer(capacity: usize) -> Buffer {
            let mut buffer = Buffer::new(capacity);
            for i in 0..capacity {
                buffer += BufferLine::new_rx(Local::now(), format!("L{i:03}").into_bytes());
            }
            buffer
        }

        fn push(buffer: &mut Buffer, text: &str) {
            *buffer += BufferLine::new_rx(Local::now(), text.as_bytes().to_vec());
        }

        /// A screen `height` rows tall (so `height - 2` content rows), already
        /// reconciled with `buffer` and therefore sitting at the bottom.
        fn screen_on(buffer: &Buffer, height: u16) -> Screen {
            let mut screen = Screen::default();
            screen.set_size(
                Rect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height,
                },
                buffer.len(),
            );
            screen.update_after_new_lines(buffer);
            screen
        }

        /// The messages the viewport currently shows, top to bottom.
        fn visible(screen: &Screen, buffer: &Buffer) -> Vec<String> {
            let start = screen.position.line;
            let rows = screen.size.height.saturating_sub(2) as usize;

            buffer
                .get_range(start, start + rows)
                .map(|line| String::from_utf8_lossy(&line.message).to_string())
                .collect()
        }

        #[test]
        fn a_frozen_viewport_keeps_showing_the_same_lines() {
            // 20 lines over 7 content rows: the bottom offset is 13.
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);
            assert!(screen.auto_scroll);

            // Scroll up into the middle of the history.
            screen.scroll_vertical(-8, 13);
            assert!(!screen.auto_scroll);
            let frozen = visible(&screen, &buffer);
            assert_eq!(frozen.first().unwrap(), "L005");

            // Three new lines evict L000..L002 and re-index the rest, so the
            // offset has to come down by three to stay on the same content.
            for text in ["N1", "N2", "N3"] {
                push(&mut buffer, text);
            }
            screen.update_after_new_lines(&buffer);

            assert_eq!(visible(&screen, &buffer), frozen);
            assert_eq!(screen.position.line, 2);
        }

        #[test]
        fn auto_scroll_still_follows_the_bottom_through_evictions() {
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);

            push(&mut buffer, "N1");
            screen.update_after_new_lines(&buffer);

            assert!(screen.auto_scroll);
            assert_eq!(visible(&screen, &buffer).last().unwrap(), "N1");
        }

        #[test]
        fn the_top_of_the_history_is_the_floor() {
            // At offset 0 the lines on screen are the ones being evicted: there
            // is nothing above to slide to, so here the content does move.
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);
            screen.disable_auto_scroll();
            screen.jump_to_start();

            push(&mut buffer, "N1");
            screen.update_after_new_lines(&buffer);

            assert_eq!(screen.position.line, 0);
            assert_eq!(visible(&screen, &buffer).first().unwrap(), "L001");
        }

        #[test]
        fn a_selection_slides_with_the_content() {
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);
            screen.selection = Some(Selection::new(
                BufferPosition {
                    line: 10,
                    column: 1,
                },
                BufferPosition {
                    line: 12,
                    column: 4,
                },
            ));

            push(&mut buffer, "N1");
            screen.update_after_new_lines(&buffer);

            let selection = screen.selection.expect("the selection is still valid");
            assert_eq!(selection.start, BufferPosition { line: 9, column: 1 });
            assert_eq!(
                selection.end,
                BufferPosition {
                    line: 11,
                    column: 4
                }
            );
        }

        #[test]
        fn a_selection_whose_lines_are_all_evicted_is_dropped() {
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);
            screen.selection = Some(Selection::new(
                BufferPosition { line: 0, column: 0 },
                BufferPosition { line: 1, column: 2 },
            ));

            for text in ["N1", "N2", "N3"] {
                push(&mut buffer, text);
            }
            screen.update_after_new_lines(&buffer);

            assert!(screen.selection.is_none());
        }

        #[test]
        fn clearing_resets_the_eviction_bookkeeping() {
            let mut buffer = full_buffer(20);
            let mut screen = screen_on(&buffer, 9);
            push(&mut buffer, "N1");
            screen.update_after_new_lines(&buffer);
            assert_eq!(screen.evicted_seen, 1);

            buffer.clear();
            screen.clear();

            assert_eq!(screen.evicted_seen, 0);
            assert_eq!(buffer.evicted(), 0);
        }
    }

    // The hit list is re-scanned whenever new lines land, and must come back
    // pointing at the hit the user navigated to even though the buffer
    // re-indexed its lines underneath it (issue #218).
    mod frozen_search {
        use super::super::{Screen, ScreenMode, SearchHit};
        use crate::graphics::buffer::BufferPosition;

        fn hit(line: usize, line_id: u64) -> SearchHit {
            SearchHit {
                position: BufferPosition { line, column: 0 },
                line_id,
            }
        }

        /// A search over three hits, sitting on the one given by `current`.
        fn searching_on(current: usize, hits: &[SearchHit]) -> Screen {
            let mut screen = Screen::default();
            screen.change_mode_to_search("x".to_string(), true, false);
            for hit in hits {
                screen.mode_mut().add_entry(*hit);
            }
            if let ScreenMode::Search { current: c, .. } = screen.mode_mut() {
                *c = current;
            }
            screen
        }

        #[test]
        fn a_rescan_stays_on_the_same_hit_after_the_lines_slid_up() {
            let mut screen = searching_on(1, &[hit(4, 104), hit(9, 109), hit(15, 115)]);

            let previous = screen.mode_mut().take_hits_for_rescan();
            assert_eq!(screen.search_indexes(), Some((1, 0)));

            // Three lines were evicted, so every hit is three indices lower.
            for h in [hit(1, 104), hit(6, 109), hit(12, 115)] {
                screen.mode_mut().add_entry(h);
            }
            screen.mode_mut().restore_current(previous);

            assert_eq!(screen.search_indexes(), Some((1, 3)));
        }

        #[test]
        fn new_hits_appended_below_do_not_move_the_position() {
            let mut screen = searching_on(1, &[hit(4, 104), hit(9, 109)]);

            let previous = screen.mode_mut().take_hits_for_rescan();
            for h in [hit(4, 104), hit(9, 109), hit(20, 130)] {
                screen.mode_mut().add_entry(h);
            }
            screen.mode_mut().restore_current(previous);

            assert_eq!(screen.search_indexes(), Some((1, 3)));
        }

        #[test]
        fn a_hit_scrolled_out_of_the_history_falls_to_the_next_one_in_time() {
            let mut screen = searching_on(0, &[hit(0, 104), hit(5, 109)]);

            let previous = screen.mode_mut().take_hits_for_rescan();
            // L104 is gone; ids grow monotonically, so 109 is the next hit in
            // time and the closest thing to where the user was.
            for h in [hit(2, 109), hit(8, 115)] {
                screen.mode_mut().add_entry(h);
            }
            screen.mode_mut().restore_current(previous);

            assert_eq!(screen.search_indexes(), Some((0, 2)));
        }

        #[test]
        fn a_hit_past_the_end_of_the_new_list_falls_back_to_the_first() {
            let mut screen = searching_on(1, &[hit(0, 104), hit(5, 109)]);

            let previous = screen.mode_mut().take_hits_for_rescan();
            // The filter changed and only an older line still matches.
            screen.mode_mut().add_entry(hit(0, 100));
            screen.mode_mut().restore_current(previous);

            assert_eq!(screen.search_indexes(), Some((0, 1)));
        }

        #[test]
        fn a_rescan_that_finds_nothing_leaves_a_valid_position() {
            let mut screen = searching_on(1, &[hit(0, 104), hit(5, 109)]);

            let previous = screen.mode_mut().take_hits_for_rescan();
            screen.mode_mut().restore_current(previous);

            assert_eq!(screen.search_indexes(), Some((0, 0)));
        }

        #[test]
        fn an_empty_query_is_not_worth_rescanning() {
            let mut screen = Screen::default();
            assert!(!screen.is_searching());

            screen.change_mode_to_search(String::new(), true, false);
            assert!(!screen.is_searching());

            screen.change_mode_to_search("x".to_string(), true, false);
            assert!(screen.is_searching());
        }
    }

    // Bookmarks (issue #208).
    mod bookmarks {
        use super::super::{Screen, ScreenPosition};
        use crate::graphics::buffer::{Buffer, BufferLine};
        use chrono::Local;
        use ratatui::layout::Rect;

        fn buffer_with(n: usize) -> Buffer {
            let mut buffer = Buffer::new(n.max(1));
            for _ in 0..n {
                buffer += BufferLine::new_rx(Local::now(), b"x".to_vec());
            }
            buffer
        }

        fn ids(buffer: &Buffer) -> Vec<u64> {
            buffer.iter().map(|line| line.id).collect()
        }

        fn sized_screen(height: u16, buffer_len: usize) -> Screen {
            let mut screen = Screen::default();
            screen.set_size(
                Rect {
                    x: 0,
                    y: 0,
                    width: 80,
                    height,
                },
                buffer_len,
            );
            screen
        }

        #[test]
        fn right_click_toggles_the_line_under_the_cursor() {
            let buffer = buffer_with(5);
            let ids = ids(&buffer);
            let mut screen = sized_screen(10, buffer.len());

            // Content starts one row below the top border, so line 2 is at y=3.
            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
            assert!(screen.bookmarks.contains(&ids[2]));

            // A second right-click on the same line removes it.
            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
            assert!(!screen.bookmarks.contains(&ids[2]));
        }

        #[test]
        fn clicks_off_the_content_are_ignored() {
            let buffer = buffer_with(5);
            let mut screen = sized_screen(10, buffer.len());

            // Top border row.
            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 0 });
            // Well past the last line.
            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 50 });

            assert!(screen.bookmarks.is_empty());
        }

        #[test]
        fn removing_the_current_bookmark_forgets_it() {
            let buffer = buffer_with(5);
            let ids = ids(&buffer);
            let mut screen = sized_screen(10, buffer.len());

            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
            screen.current_bookmark = Some(ids[2]);

            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
            assert_eq!(screen.current_bookmark, None);
        }

        #[test]
        fn clear_drops_all_bookmarks() {
            let buffer = buffer_with(5);
            let mut screen = sized_screen(10, buffer.len());
            screen.toggle_bookmark(&buffer, ScreenPosition { x: 0, y: 3 });
            screen.current_bookmark = Some(0);

            screen.clear();

            assert!(screen.bookmarks.is_empty());
            assert_eq!(screen.current_bookmark, None);
        }

        #[test]
        fn next_index_without_current_anchors_to_the_viewport() {
            let positions = [(2, 100), (10, 101), (18, 102)];

            // Forward: first bookmark below the anchor line.
            assert_eq!(
                Screen::next_bookmark_index(&positions, None, 4, true),
                Some(1)
            );
            // Backward: last bookmark above the anchor line.
            assert_eq!(
                Screen::next_bookmark_index(&positions, None, 4, false),
                Some(0)
            );
            // Forward with nothing below wraps to the first.
            assert_eq!(
                Screen::next_bookmark_index(&positions, None, 100, true),
                Some(0)
            );
            // Backward with nothing above wraps to the last.
            assert_eq!(
                Screen::next_bookmark_index(&positions, None, 0, false),
                Some(2)
            );
        }

        #[test]
        fn next_index_steps_and_wraps_from_the_current_bookmark() {
            let positions = [(2, 100), (10, 101), (18, 102)];

            assert_eq!(
                Screen::next_bookmark_index(&positions, Some(101), 999, true),
                Some(2)
            );
            // Forward off the end wraps to the first.
            assert_eq!(
                Screen::next_bookmark_index(&positions, Some(102), 999, true),
                Some(0)
            );
            // Backward off the front wraps to the last.
            assert_eq!(
                Screen::next_bookmark_index(&positions, Some(100), 999, false),
                Some(2)
            );
        }

        #[test]
        fn next_index_falls_back_to_anchor_when_current_is_gone() {
            let positions = [(2, 100), (10, 101), (18, 102)];
            // Id 999 is not among the positions (its line was filtered out or
            // scrolled off), so navigation restarts from the viewport anchor.
            assert_eq!(
                Screen::next_bookmark_index(&positions, Some(999), 4, true),
                Some(1)
            );
        }

        #[test]
        fn next_index_is_none_when_there_are_no_bookmarks() {
            assert_eq!(Screen::next_bookmark_index(&[], None, 4, true), None);
            assert_eq!(Screen::next_bookmark_index(&[], Some(1), 4, false), None);
        }

        #[test]
        fn navigation_cycles_through_bookmarks_by_id() {
            let buffer = buffer_with(20);
            let ids = ids(&buffer);
            let mut screen = sized_screen(10, buffer.len());
            for line in [2usize, 10, 18] {
                screen.bookmarks.insert(ids[line]);
            }
            let max_main_axis = buffer.len().saturating_sub(8);

            // Fresh (no current): anchor is the screen centre (line 4), so the
            // first Tab lands on the first bookmark below it.
            screen.jump_to_next_bookmark(&buffer, max_main_axis);
            assert_eq!(screen.current_bookmark, Some(ids[10]));

            screen.jump_to_next_bookmark(&buffer, max_main_axis);
            assert_eq!(screen.current_bookmark, Some(ids[18]));

            // Wrap around to the top.
            screen.jump_to_next_bookmark(&buffer, max_main_axis);
            assert_eq!(screen.current_bookmark, Some(ids[2]));

            // Shift+Tab steps back, wrapping to the bottom.
            screen.jump_to_previous_bookmark(&buffer, max_main_axis);
            assert_eq!(screen.current_bookmark, Some(ids[18]));
        }

        #[test]
        fn navigation_is_a_no_op_without_bookmarks() {
            let buffer = buffer_with(20);
            let mut screen = sized_screen(10, buffer.len());
            let max_main_axis = buffer.len().saturating_sub(8);

            screen.jump_to_next_bookmark(&buffer, max_main_axis);
            assert_eq!(screen.current_bookmark, None);
        }

        // The `Tab`-focused bookmark must stand out from the others: it keeps the
        // full yellow-background highlight, while every other bookmark drops to a
        // subtler yellow foreground on the normal background.
        #[test]
        fn current_bookmark_is_highlighted_apart_from_other_bookmarks() {
            use super::super::ScreenMode;
            use ratatui::style::Color;

            let ts = Local::now();
            let style_of = |is_bookmarked, is_current| {
                ScreenMode::timestamp_line(ts, false, is_bookmarked, is_current)[0].style
            };

            // A plain (non-bookmarked) line: dim gray text, no background.
            let plain = style_of(false, false);
            assert_eq!(plain.bg, None);
            assert_eq!(plain.fg, Some(Color::DarkGray));

            // A bookmark that isn't the current one: yellow text, normal background.
            let other = style_of(true, false);
            assert_eq!(other.bg, None);
            assert_eq!(other.fg, Some(Color::Yellow));

            // The current bookmark: full yellow-background highlight.
            let current = style_of(true, true);
            assert_eq!(current.bg, Some(Color::Yellow));
            assert_eq!(current.fg, Some(Color::Black));
        }
    }
}
