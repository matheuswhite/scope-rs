use ratatui::style::Color;

pub struct Palette;

impl Palette {
    pub fn fg(bg: Color) -> Color {
        match bg {
            Color::Black
            | Color::Red
            | Color::Blue
            | Color::Magenta
            | Color::Gray
            | Color::LightRed => Color::White,
            Color::Green
            | Color::Yellow
            | Color::Cyan
            | Color::DarkGray
            | Color::LightGreen
            | Color::LightYellow
            | Color::LightBlue
            | Color::LightMagenta
            | Color::LightCyan
            | Color::White => Color::Black,
            _ => Color::Reset,
        }
    }

    pub fn ascent_fg(bg: Color, fg: Color) -> Color {
        match bg {
            Color::Reset if fg == Color::Yellow => Color::Magenta,
            Color::Red | Color::LightRed => Color::Blue,
            Color::Green
            | Color::Yellow
            | Color::Blue
            | Color::Cyan
            | Color::LightGreen
            | Color::LightYellow
            | Color::LightBlue
            | Color::LightCyan
            | Color::White => Color::Magenta,
            _ => Color::Yellow,
        }
    }
}
