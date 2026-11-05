//! User theming (issue #68): the colors, icons and background the TUI draws
//! with, overridable from the optional `[theme]` / `[theme.icons]` tables of
//! `config.toml`. There is no CLI flag, so precedence is config.toml > built-in
//! default, and every default reproduces the look scope had before theming.
//!
//! Colors are given by role (what the color *means* — `connected`, `search`,
//! `hint`…) rather than by widget, so one key recolors every place that role
//! shows up. A value is a terminal color name (`red`, `light_green`,
//! `dark_gray`, `reset`…), a `#rrggbb` hex triplet, or an ANSI-256 index
//! (`"0"`..`"255"`), parsed by ratatui's own `Color::from_str`.
//!
//! **Transparency** is the `background` role. A terminal application cannot
//! change the window's opacity; what it controls is whether it paints a
//! background at all. The default, `reset`, paints none, so the terminal's own
//! background — transparent or not — shows through. Any other color fills every
//! cell that would otherwise be left unpainted ([`paint_background`]).
//!
//! The theme is resolved once in `main`, before the picker or any task starts,
//! and installed in a process-wide [`OnceLock`] ([`install`]/[`theme`]). It is
//! immutable for the whole run and read from ~20 render functions spread over
//! the graphics task, the screen, the headless bridge and the picker, so a
//! global read beats threading a parameter through all of their signatures.
//! Anything that runs without an installed theme (unit tests) gets the default.

use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::text::Span;
use serde::{Deserialize, Deserializer};
use std::str::FromStr;
use std::sync::OnceLock;

static THEME: OnceLock<Theme> = OnceLock::new();

/// The installed theme, or the built-in default when none was installed.
pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::default)
}

/// Install the theme for the rest of the run. Only the first call wins; it is
/// made once, from `main`, before anything renders.
pub fn install(theme: Theme) {
    let _ = THEME.set(theme);
}

/// Fill every cell left on the terminal's default background with the theme's
/// `background`. Run at the end of a frame, after every widget rendered, so a
/// widget's explicit background (a log line, a selection bar…) wins and only
/// the unpainted cells are filled. A `reset` background is a no-op.
pub fn paint_background(buffer: &mut Buffer) {
    fill_background(buffer, theme().background);
}

fn fill_background(buffer: &mut Buffer, background: Color) {
    if background == Color::Reset {
        return;
    }

    for cell in buffer.content.iter_mut() {
        if cell.bg == Color::Reset {
            cell.bg = background;
        }
    }
}

/// Every color and icon the TUI uses. Field names are the `[theme]` keys.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    /// Background of every unpainted cell; `reset` keeps the terminal's own
    /// (and with it any transparency).
    #[serde(deserialize_with = "color")]
    pub background: Color,
    /// Foreground of received data.
    #[serde(deserialize_with = "color")]
    pub text: Color,
    /// Foreground of sent data.
    #[serde(deserialize_with = "color")]
    pub tx_text: Color,
    /// Background of sent data.
    #[serde(deserialize_with = "color")]
    pub tx_background: Color,
    /// Border of the scrollback in normal mode, and of the history pop-up.
    #[serde(deserialize_with = "color")]
    pub border: Color,
    /// Command-bar border while the interface is connected, and the clipboard
    /// copy flash.
    #[serde(deserialize_with = "color")]
    pub connected: Color,
    /// Command-bar border while the interface is disconnected.
    #[serde(deserialize_with = "color")]
    pub disconnected: Color,
    /// Scrollback border while `!record` is running.
    #[serde(deserialize_with = "color")]
    pub recording: Color,
    /// Search mode: borders, match highlight, the chosen match's background.
    #[serde(deserialize_with = "color")]
    pub search: Color,
    /// The search query's color when it matches nothing.
    #[serde(deserialize_with = "color")]
    pub no_match: Color,
    /// Special characters and `@tags` in the command bar, the tag
    /// autocomplete pop-up and the matched letters of the history search.
    #[serde(deserialize_with = "color")]
    pub accent: Color,
    /// Low-emphasis text: timestamps, hints, the scrollbar track, lines that
    /// do not match a search.
    #[serde(deserialize_with = "color")]
    pub hint: Color,
    /// Background of a selected line's timestamp.
    #[serde(deserialize_with = "color")]
    pub selection: Color,
    /// Timestamp of a bookmarked line.
    #[serde(deserialize_with = "color")]
    pub bookmark: Color,
    /// The serial/RTT picker shown when launched without a port or target.
    #[serde(deserialize_with = "color")]
    pub picker: Color,
    /// Picker validation errors.
    #[serde(deserialize_with = "color")]
    pub picker_error: Color,
    /// Background of an error log line.
    #[serde(deserialize_with = "color")]
    pub log_error: Color,
    /// Background of a warning log line.
    #[serde(deserialize_with = "color")]
    pub log_warning: Color,
    /// Background of a success log line.
    #[serde(deserialize_with = "color")]
    pub log_success: Color,
    /// Background of an info log line.
    #[serde(deserialize_with = "color")]
    pub log_info: Color,
    /// Background of a debug log line.
    #[serde(deserialize_with = "color")]
    pub log_debug: Color,
    /// The `[theme.icons]` table.
    pub icons: Icons,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::Reset,
            text: Color::Reset,
            tx_text: Color::Black,
            tx_background: Color::White,
            border: Color::White,
            connected: Color::Green,
            disconnected: Color::Red,
            recording: Color::Red,
            search: Color::Yellow,
            no_match: Color::Red,
            accent: Color::Magenta,
            hint: Color::DarkGray,
            selection: Color::White,
            bookmark: Color::Yellow,
            picker: Color::Yellow,
            picker_error: Color::Red,
            log_error: Color::Red,
            log_warning: Color::Yellow,
            log_success: Color::LightGreen,
            log_info: Color::Cyan,
            log_debug: Color::DarkGray,
            icons: Icons::default(),
        }
    }
}

/// The glyphs the TUI draws, e.g. to swap them for ASCII on a terminal font
/// without the Unicode ones. Field names are the `[theme.icons]` keys.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Icons {
    /// Marks the highlighted entry of a list (history search, picker).
    pub highlight: String,
    /// Shown in the scrollback title while `!record` is running.
    pub recording: String,
    /// Top arrow of the scrollbar. Must be one cell wide.
    pub scroll_up: String,
    /// Bottom arrow of the scrollbar. Must be one cell wide.
    pub scroll_down: String,
    /// The text cursor of the picker's input fields. Must be one cell wide.
    pub cursor: String,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            highlight: "▶ ".to_string(),
            recording: "◉".to_string(),
            scroll_up: "▲".to_string(),
            scroll_down: "▼".to_string(),
            cursor: "▏".to_string(),
        }
    }
}

impl Theme {
    /// Reject what the TOML parser cannot: an icon drawn into a single cell
    /// that is not exactly one cell wide would shift (or be clipped out of)
    /// the layout around it.
    pub fn validate(self) -> Result<Self, String> {
        for (key, icon) in [
            ("scroll_up", &self.icons.scroll_up),
            ("scroll_down", &self.icons.scroll_down),
            ("cursor", &self.icons.cursor),
        ] {
            let width = Span::raw(icon.as_str()).width();
            if width != 1 {
                return Err(format!(
                    "Invalid icon `theme.icons.{key} = {icon:?}`: it must be exactly one \
                     cell wide, but it is {width}"
                ));
            }
        }

        Ok(self)
    }
}

/// The text a [`Color`] value is read from, with an error that names the
/// accepted forms (ratatui's own `Deserialize` only says "Failed to parse
/// Colors").
fn color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let value = String::deserialize(deserializer)?;
    Color::from_str(&value).map_err(|_| {
        serde::de::Error::custom(format!(
            "invalid color `{value}`: expected a name (reset, black, red, green, yellow, \
             blue, magenta, cyan, gray, dark_gray, light_red, light_green, light_yellow, \
             light_blue, light_magenta, light_cyan, white), a hex `#rrggbb` or an \
             ANSI-256 index `0`..`255`"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::style::Style;

    fn parse(toml_text: &str) -> Result<Theme, String> {
        toml::from_str::<Theme>(toml_text).map_err(|err| err.to_string())
    }

    #[test]
    fn empty_table_is_the_default_theme() {
        assert_eq!(parse("").unwrap(), Theme::default());
    }

    #[test]
    fn accepts_names_hex_and_indexed_colors() {
        let theme = parse(
            "background = \"#1e1e2e\"\naccent = \"light_magenta\"\nhint = \"244\"\n\
             border = \"Dark Gray\"\n",
        )
        .unwrap();

        assert_eq!(theme.background, Color::Rgb(0x1e, 0x1e, 0x2e));
        assert_eq!(theme.accent, Color::LightMagenta);
        assert_eq!(theme.hint, Color::Indexed(244));
        assert_eq!(theme.border, Color::DarkGray);
        // An omitted key keeps its default.
        assert_eq!(theme.search, Color::Yellow);
        assert_eq!(theme.icons, Icons::default());
    }

    #[test]
    fn rejects_an_unknown_color_naming_the_accepted_forms() {
        let err = parse("search = \"purpleish\"\n").unwrap_err();

        assert!(err.contains("invalid color `purpleish`"), "{err}");
        assert!(err.contains("#rrggbb"), "{err}");
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn rejects_an_unknown_role() {
        let err = parse("borders = \"red\"\n").unwrap_err();

        assert!(err.contains("unknown field `borders`"), "{err}");
    }

    #[test]
    fn parses_the_icons_table() {
        let theme = parse("[icons]\nhighlight = \"> \"\nscroll_up = \"^\"\n").unwrap();

        assert_eq!(theme.icons.highlight, "> ");
        assert_eq!(theme.icons.scroll_up, "^");
        assert_eq!(theme.icons.scroll_down, "▼");
    }

    #[test]
    fn rejects_an_unknown_icon() {
        let err = parse("[icons]\narrow = \">\"\n").unwrap_err();

        assert!(err.contains("unknown field `arrow`"), "{err}");
    }

    #[test]
    fn single_cell_icons_must_be_one_cell_wide() {
        assert!(Theme::default().validate().is_ok());

        for (key, value) in [("scroll_up", "^^"), ("scroll_down", ""), ("cursor", "宽")] {
            let theme = parse(&format!("[icons]\n{key} = {value:?}\n")).unwrap();
            let err = theme.validate().unwrap_err();
            assert!(err.contains(&format!("theme.icons.{key}")), "{err}");
        }
    }

    #[test]
    fn free_width_icons_accept_any_width() {
        let theme = parse("[icons]\nhighlight = \"\"\nrecording = \"REC\"\n").unwrap();

        assert!(theme.validate().is_ok());
    }

    #[test]
    fn painting_the_background_fills_only_unpainted_cells() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_style(Rect::new(1, 0, 1, 1), Style::default().bg(Color::Red));

        fill_background(&mut buffer, Color::Blue);

        let bgs = buffer.content.iter().map(|c| c.bg).collect::<Vec<_>>();
        assert_eq!(bgs, vec![Color::Blue, Color::Red, Color::Blue]);
    }

    #[test]
    fn a_reset_background_paints_nothing() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));

        fill_background(&mut buffer, Color::Reset);

        assert!(buffer.content.iter().all(|c| c.bg == Color::Reset));
    }
}
