use crate::graphics::special_char::{SpecialCharItem, SpecialCharPosition, ToSpecialChar};
use ratatui::{style::Color, text::Span};

#[allow(clippy::upper_case_acronyms)]
pub struct ANSI;

impl ANSI {
    /// Locate the next CSI escape sequence (`\x1b[` … final byte) in the decoded
    /// text. The screen decoder renders the ESC byte as the literal characters
    /// `\x1b`, so this works on that escaped form. A sequence runs from `\x1b[`
    /// through its parameter bytes (digits and `;:<=>?`) up to and including the
    /// first letter (the CSI final byte), e.g. `\x1b[1;32m` or `\x1b[15C`.
    ///
    /// Recognising *any* terminated CSI sequence — not just a fixed table — is
    /// what fixes issue #119: previously unknown codes (`\x1b[39m`, `\x1b[0;32m`,
    /// multi-parameter SGR, …) were left in the text, so they showed up as
    /// highlighted special characters and their colour change was lost.
    fn next_csi(string: &str) -> Option<SpecialCharPosition> {
        const PREFIX: &str = "\\x1b[";
        let mut search_from = 0;

        while let Some(rel) = string[search_from..].find(PREFIX) {
            let start = search_from + rel;
            let params_start = start + PREFIX.len();
            let mut end = None;

            for (i, c) in string[params_start..].char_indices() {
                if c.is_ascii_digit() || matches!(c, ';' | ':' | '<' | '=' | '>' | '?') {
                    continue;
                }
                if c.is_ascii_alphabetic() {
                    end = Some(params_start + i + c.len_utf8());
                }
                break;
            }

            if let Some(end) = end {
                let start_chars = string[..start].chars().count();
                let length = string[start..end].chars().count();
                return Some((start_chars, length).into());
            }

            // Unterminated here (e.g. a truncated escape at end of line); keep
            // looking past this `\x1b[` for a later, complete sequence.
            search_from = params_start;
        }

        None
    }

    /// Apply a single escape sequence to the current foreground colour. Only SGR
    /// sequences (those ending in `m`) affect colour; cursor moves, erases, etc.
    /// are consumed without changing it. Parameters are applied left to right, so
    /// the last colour-setting code wins (e.g. `0;32` resets then turns green).
    fn apply_sgr(seq: &str, current: Color) -> Color {
        let Some(body) = seq
            .strip_prefix("\\x1b[")
            .and_then(|rest| rest.strip_suffix('m'))
        else {
            return current;
        };

        // `\x1b[m` is shorthand for `\x1b[0m`, a full reset.
        if body.is_empty() {
            return Color::Reset;
        }

        let mut color = current;
        let mut params = body.split(';');
        while let Some(param) = params.next() {
            let Ok(code) = param.parse::<u32>() else {
                continue;
            };

            match code {
                // Reset, or reset foreground to its default.
                0 | 39 => color = Color::Reset,
                // Standard and bright foreground colours.
                30..=37 => color = Self::base_color(code - 30),
                90..=97 => color = Self::base_color(code - 90),
                // Extended foreground: `38;5;n` (indexed) or `38;2;r;g;b` (RGB).
                38 => match params.next().and_then(|p| p.parse::<u32>().ok()) {
                    Some(5) => {
                        if let Some(n) = params.next().and_then(|p| p.parse::<u8>().ok()) {
                            color = Color::Indexed(n);
                        }
                    }
                    Some(2) => {
                        let r = params.next().and_then(|p| p.parse::<u8>().ok());
                        let g = params.next().and_then(|p| p.parse::<u8>().ok());
                        let b = params.next().and_then(|p| p.parse::<u8>().ok());
                        if let (Some(r), Some(g), Some(b)) = (r, g, b) {
                            color = Color::Rgb(r, g, b);
                        }
                    }
                    _ => {}
                },
                // Extended background: skip its arguments — we only track foreground.
                48 => match params.next().and_then(|p| p.parse::<u32>().ok()) {
                    Some(5) => {
                        params.next();
                    }
                    Some(2) => {
                        params.next();
                        params.next();
                        params.next();
                    }
                    _ => {}
                },
                // Intensity/style attributes and background colours: leave fg as is.
                _ => {}
            }
        }

        color
    }

    fn base_color(index: u32) -> Color {
        match index {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            _ => Color::White,
        }
    }

    pub fn decode(input: Span) -> Vec<Span> {
        let mut spans = vec![];
        let style = input.style;
        let mut color = input.style.fg.unwrap_or(Color::Reset);

        for item in input
            .content
            .to_special_char(|string| Self::next_csi(string))
        {
            match item {
                SpecialCharItem::Plain(plain) => {
                    spans.push(Span::styled(plain, style.fg(color)));
                }
                SpecialCharItem::Special(special, _) => {
                    color = Self::apply_sgr(&special, color);
                }
            }
        }

        spans
    }

    pub fn remove_encoding(input: String) -> String {
        let mut result = String::new();

        for item in input.to_special_char(|string| Self::next_csi(string)) {
            let SpecialCharItem::Plain(plain) = item else {
                continue;
            };

            result.push_str(&plain);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode() {
        let input = Span::raw("Hello \\x1b[31mRed\\x1b[0m World");
        let spans = ANSI::decode(input);

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "Hello ");
        assert_eq!(spans[0].style.fg, Some(Color::Reset));
        assert_eq!(spans[1].content, "Red");
        assert_eq!(spans[1].style.fg, Some(Color::Red));
        assert_eq!(spans[2].content, " World");
        assert_eq!(spans[2].style.fg, Some(Color::Reset));
    }

    #[test]
    fn test_remove_encoding() {
        let input = "Hello \\x1b[31mRed\\x1b[0m World".to_string();
        let output = ANSI::remove_encoding(input);

        assert_eq!(output, "Hello Red World");
    }

    #[test]
    fn test_decode_dyn() {
        let input = Span::raw("Hello \\x1b[15CRed Wo\\x1b[Crld\\x1b[");
        let spans = ANSI::decode(input);

        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content, "Hello ");
        assert_eq!(spans[0].style.fg, Some(Color::Reset));
        assert_eq!(spans[1].content, "Red Wo");
        assert_eq!(spans[1].style.fg, Some(Color::Reset));
        assert_eq!(spans[2].content, "rld\\x1b[");
        assert_eq!(spans[2].style.fg, Some(Color::Reset));
    }

    #[test]
    fn test_remove_dyn() {
        let input = "Hello \\x1b[15CRed Wo\\x1b[Crld\\x1b[".to_string();
        let output = ANSI::remove_encoding(input);

        assert_eq!(output, "Hello Red World\\x1b[");
    }

    // Issue #119: the Zephyr shell paints its `uart:~$` prompt green and then
    // resets the foreground with `\x1b[39m` (default foreground). The old fixed
    // table didn't know `\x1b[39m`, so the green bled into the rest of the line
    // and the escape rendered as a special character.
    #[test]
    fn test_decode_default_fg_resets_color() {
        let input = Span::raw("\\x1b[1;32muart:~$\\x1b[39m done");
        let spans = ANSI::decode(input);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "uart:~$");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[1].content, " done");
        assert_eq!(spans[1].style.fg, Some(Color::Reset));
    }

    #[test]
    fn test_decode_zero_semicolon_color() {
        // `\x1b[0;32m` (reset-then-green) was another unrecognised form.
        let input = Span::raw("\\x1b[0;32mfoo\\x1b[0mbar");
        let spans = ANSI::decode(input);

        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content, "foo");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
        assert_eq!(spans[1].content, "bar");
        assert_eq!(spans[1].style.fg, Some(Color::Reset));
    }

    #[test]
    fn test_decode_multi_parameter_with_background() {
        // Bold green on a black background: the background param must not stop
        // the green from being applied, and the sequence must not leak as text.
        let input = Span::raw("\\x1b[1;32;40mok\\x1b[0m");
        let spans = ANSI::decode(input);

        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content, "ok");
        assert_eq!(spans[0].style.fg, Some(Color::Green));
    }

    #[test]
    fn test_decode_extended_colors() {
        let indexed = ANSI::decode(Span::raw("\\x1b[38;5;201mx"));
        assert_eq!(indexed[0].content, "x");
        assert_eq!(indexed[0].style.fg, Some(Color::Indexed(201)));

        let rgb = ANSI::decode(Span::raw("\\x1b[38;2;10;20;30my"));
        assert_eq!(rgb[0].content, "y");
        assert_eq!(rgb[0].style.fg, Some(Color::Rgb(10, 20, 30)));
    }

    #[test]
    fn test_remove_encoding_strips_unknown_sgr() {
        let input = "\\x1b[1;32muart:~$\\x1b[39m".to_string();
        assert_eq!(ANSI::remove_encoding(input), "uart:~$");
    }
}
