use tui::style::Color;

#[derive(Copy, Clone)]
pub struct Theme {
    is_light: bool,
}

impl Theme {
    pub fn new(is_light: bool) -> Self {
        Self { is_light }
    }

    pub fn red(&self) -> Color {
        Color::LightRed
    }

    pub fn green(&self) -> Color {
        Color::LightGreen
    }

    pub fn blue(&self) -> Color {
        if self.is_light {
            Color::LightBlue
        } else {
            Color::LightCyan
        }
    }

    pub fn magenta(&self) -> Color {
        Color::LightMagenta
    }

    pub fn yellow(&self) -> Color {
        if self.is_light {
            Color::LightYellow
        } else {
            Color::Yellow
        }
    }

    pub fn gray(&self) -> Color {
        if self.is_light {
            Color::Gray
        } else {
            Color::DarkGray
        }
    }

    pub fn primary(&self) -> Color {
        if self.is_light {
            Color::White
        } else {
            Color::Black
        }
    }
}
