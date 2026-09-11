use crate::graphics::buffer::BufferPosition;

#[derive(Clone, Copy, Default)]
pub struct Selection {
    pub start: BufferPosition,
    pub end: BufferPosition,
}

pub enum SelectionPosition {
    OneLine {
        start_column: usize,
        end_column: usize,
    },
    Top {
        column: usize,
    },
    Middle,
    Bottom {
        column: usize,
    },
    Outside,
}

impl Selection {
    pub fn new(start: BufferPosition, end: BufferPosition) -> Self {
        Self { start, end }
    }

    pub fn update(&mut self, new_point: BufferPosition) {
        self.end = new_point;
    }

    /// Slides the selection up by `lines` rows, following the content when the
    /// buffer drops that many lines off its front. Both endpoints are buffer
    /// *indices*, so without this the selection would stay on the same rows
    /// while the text under them moved (issue #218).
    ///
    /// Returns `false` when every selected line has been evicted, in which case
    /// the selection no longer refers to anything and the caller drops it.
    pub fn shift_up(&mut self, lines: usize) -> bool {
        if lines == 0 {
            return true;
        }

        let (earlier, later) = self.ordered_positions();
        if later.line < lines {
            return false;
        }

        // The earlier endpoint may be the one that got evicted: clamping its
        // line to 0 moves it to the oldest surviving line, where its old column
        // means nothing, so the selection starts at that line's beginning.
        let earlier_is_start = self.start < self.end;
        let earlier_evicted = earlier.line < lines;

        self.start.line = self.start.line.saturating_sub(lines);
        self.end.line = self.end.line.saturating_sub(lines);

        if earlier_evicted {
            if earlier_is_start {
                self.start.column = 0;
            } else {
                self.end.column = 0;
            }
        }

        true
    }

    pub fn ordered_positions(&self) -> (BufferPosition, BufferPosition) {
        if self.start < self.end {
            (self.start, self.end)
        } else {
            (self.end, self.start)
        }
    }

    pub fn selection_position(&self, line: usize) -> SelectionPosition {
        if !self.is_inside(line) {
            return SelectionPosition::Outside;
        }

        let (start, end) = self.ordered_positions();

        let one_line = start.line == end.line;
        if one_line {
            return SelectionPosition::OneLine {
                start_column: start.column,
                end_column: end.column,
            };
        }

        match line {
            l if l == start.line => SelectionPosition::Top {
                column: start.column,
            },
            l if l == end.line => SelectionPosition::Bottom { column: end.column },
            _ => SelectionPosition::Middle,
        }
    }

    pub fn is_inside(&self, line: usize) -> bool {
        let (start, end) = self.ordered_positions();

        start.line <= line && line <= end.line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(line: usize, column: usize) -> BufferPosition {
        BufferPosition { line, column }
    }

    // Issue #218: the buffer drops its oldest lines once at capacity, so an
    // active selection has to slide with the content it covers.
    #[test]
    fn shift_up_moves_both_endpoints() {
        let mut selection = Selection::new(pos(10, 3), pos(12, 7));

        assert!(selection.shift_up(4));

        assert_eq!(selection.start, pos(6, 3));
        assert_eq!(selection.end, pos(8, 7));
    }

    #[test]
    fn shift_up_by_zero_is_a_noop() {
        let mut selection = Selection::new(pos(10, 3), pos(12, 7));

        assert!(selection.shift_up(0));

        assert_eq!(selection.start, pos(10, 3));
        assert_eq!(selection.end, pos(12, 7));
    }

    #[test]
    fn a_fully_evicted_selection_is_rejected() {
        let mut selection = Selection::new(pos(1, 0), pos(3, 4));

        assert!(!selection.shift_up(4));
    }

    #[test]
    fn a_partially_evicted_selection_starts_at_the_oldest_line() {
        // Lines 0..=2 are gone; the selection now begins at the very start of
        // what used to be line 3, so the old start column is dropped.
        let mut selection = Selection::new(pos(1, 5), pos(6, 2));

        assert!(selection.shift_up(3));

        assert_eq!(selection.start, pos(0, 0));
        assert_eq!(selection.end, pos(3, 2));
    }

    // A selection dragged upwards has `end` before `start`; the truncation must
    // follow the earlier *position*, not the field name.
    #[test]
    fn a_backwards_selection_truncates_its_earlier_endpoint() {
        let mut selection = Selection::new(pos(6, 2), pos(1, 5));

        assert!(selection.shift_up(3));

        assert_eq!(selection.start, pos(3, 2));
        assert_eq!(selection.end, pos(0, 0));
    }
}
