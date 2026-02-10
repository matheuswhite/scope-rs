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
