use ratatui::layout::Size;

#[derive(Clone, Copy, Default)]
pub struct Selection {
    pub start: ScreenPosition,
    pub end: ScreenPosition,
}

#[derive(Clone, Copy, Default)]
pub struct ScreenPosition {
    pub x: u16,
    pub y: u16,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct WorldPosition {
    pub column: usize,
    pub line: usize,
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
    pub fn new(start: ScreenPosition, end: ScreenPosition, size: Size) -> Self {
        let start = start.clamp(size);
        let end = end.clamp(size);

        Self { start, end }
    }

    pub fn update(&mut self, new_point: ScreenPosition, size: Size) {
        self.end = new_point.clamp(size);
    }

    pub fn selection_position(&self, line: usize, offset: (u16, u16)) -> SelectionPosition {
        let start = self.start.to_world_position(offset);
        let end = self.end.to_world_position(offset);

        let (start, end) = if start > end {
            (end, start)
        } else {
            (start, end)
        };

        if line < start.line || line > end.line {
            return SelectionPosition::Outside;
        }

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

    pub fn is_inside(&self, line: usize, offset: (u16, u16)) -> bool {
        let start = self.start.to_world_position(offset);
        let end = self.end.to_world_position(offset);

        let (start, end) = if start > end {
            (end, start)
        } else {
            (start, end)
        };

        start.line <= line && line <= end.line
    }
}

impl ScreenPosition {
    pub fn clamp(self, size: Size) -> Self {
        ScreenPosition {
            x: self.x.clamp(14, size.width - 2),
            y: self.y.clamp(1, size.height - 5),
        }
    }

    fn to_world_position(self, offset: (u16, u16)) -> WorldPosition {
        WorldPosition {
            column: (self.x - 14 + offset.0) as usize,
            line: (self.y - 1 + offset.1) as usize,
        }
    }
}

impl PartialOrd for WorldPosition {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        if self.line != other.line {
            self.line.partial_cmp(&other.line)
        } else {
            self.column.partial_cmp(&other.column)
        }
    }
}
