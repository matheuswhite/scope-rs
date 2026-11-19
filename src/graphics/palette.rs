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
            // A themed background (issue #68) can be any color, not just the 16
            // named ones: pick black or white by its perceived brightness.
            Color::Rgb(r, g, b) => Self::contrast(r, g, b),
            Color::Indexed(index) => match Self::indexed_rgb(index) {
                Some((r, g, b)) => Self::contrast(r, g, b),
                None => Self::fg(Self::NAMED[index as usize]),
            },
            Color::Reset => Color::Reset,
        }
    }

    /// The 16 base colors in ANSI-index order, so `Indexed(0..16)` resolves
    /// like the named color it stands for.
    const NAMED: [Color; 16] = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];

    /// Black on a bright background, white on a dark one (ITU-R BT.601 luma).
    fn contrast(r: u8, g: u8, b: u8) -> Color {
        let luma = 299 * r as u32 + 587 * g as u32 + 114 * b as u32;
        if luma >= 128 * 1000 {
            Color::Black
        } else {
            Color::White
        }
    }

    /// The RGB value of an ANSI-256 index past the 16 base colors (whose RGB
    /// depends on the terminal's palette, hence `None`): the 6x6x6 cube, then
    /// the 24-step grayscale ramp.
    fn indexed_rgb(index: u8) -> Option<(u8, u8, u8)> {
        match index {
            0..=15 => None,
            16..=231 => {
                let level = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };
                let n = index - 16;
                Some((level(n / 36), level(n / 6 % 6), level(n % 6)))
            }
            232..=255 => {
                let gray = 8 + 10 * (index - 232);
                Some((gray, gray, gray))
            }
        }
    }

    /// The SGR parameters selecting `color` as the foreground, for writing
    /// straight to a terminal (headless mode has no ratatui backend).
    pub fn sgr_fg(color: Color) -> String {
        Self::sgr(color, 0)
    }

    /// The SGR parameters selecting `color` as the background.
    pub fn sgr_bg(color: Color) -> String {
        Self::sgr(color, 10)
    }

    fn sgr(color: Color, offset: u8) -> String {
        let base = match color {
            Color::Reset => 39,
            Color::Black => 30,
            Color::Red => 31,
            Color::Green => 32,
            Color::Yellow => 33,
            Color::Blue => 34,
            Color::Magenta => 35,
            Color::Cyan => 36,
            Color::Gray => 37,
            Color::DarkGray => 90,
            Color::LightRed => 91,
            Color::LightGreen => 92,
            Color::LightYellow => 93,
            Color::LightBlue => 94,
            Color::LightMagenta => 95,
            Color::LightCyan => 96,
            Color::White => 97,
            Color::Indexed(index) => return format!("{};5;{index}", 38 + offset),
            Color::Rgb(r, g, b) => return format!("{};2;{r};{g};{b}", 38 + offset),
        };
        (base + offset).to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_backgrounds_keep_their_historical_foreground() {
        assert_eq!(Palette::fg(Color::Red), Color::White);
        assert_eq!(Palette::fg(Color::Yellow), Color::Black);
        assert_eq!(Palette::fg(Color::Reset), Color::Reset);
    }

    #[test]
    fn rgb_and_indexed_backgrounds_get_a_contrasting_foreground() {
        assert_eq!(Palette::fg(Color::Rgb(0x1e, 0x1e, 0x2e)), Color::White);
        assert_eq!(Palette::fg(Color::Rgb(0xf5, 0xe0, 0xdc)), Color::Black);
        // Base indexes resolve like their named color.
        assert_eq!(Palette::fg(Color::Indexed(1)), Color::White);
        assert_eq!(Palette::fg(Color::Indexed(11)), Color::Black);
        // Cube corners and the grayscale ramp ends.
        assert_eq!(Palette::fg(Color::Indexed(16)), Color::White);
        assert_eq!(Palette::fg(Color::Indexed(231)), Color::Black);
        assert_eq!(Palette::fg(Color::Indexed(232)), Color::White);
        assert_eq!(Palette::fg(Color::Indexed(255)), Color::Black);
    }

    #[test]
    fn sgr_covers_named_indexed_and_rgb_colors() {
        assert_eq!(Palette::sgr_fg(Color::White), "97");
        assert_eq!(Palette::sgr_bg(Color::Red), "41");
        assert_eq!(Palette::sgr_bg(Color::LightGreen), "102");
        assert_eq!(Palette::sgr_bg(Color::DarkGray), "100");
        assert_eq!(Palette::sgr_fg(Color::Reset), "39");
        assert_eq!(Palette::sgr_bg(Color::Reset), "49");
        assert_eq!(Palette::sgr_fg(Color::Indexed(208)), "38;5;208");
        assert_eq!(Palette::sgr_bg(Color::Rgb(1, 2, 3)), "48;2;1;2;3");
    }
}
