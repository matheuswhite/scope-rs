use crate::graphics::special_char::{SpecialCharItem, ToSpecialChar};
use ratatui::{style::Color, text::Span};

#[allow(clippy::upper_case_acronyms)]
pub struct ANSI;

impl ANSI {
    const PATTERNS: [(&str, Color); 12] = [
        ("\\x1b[0m", Color::Reset),
        ("\\x1b[30m", Color::Black),
        ("\\x1b[32m", Color::Green),
        ("\\x1b[1;32m", Color::Green),
        ("\\x1b[31m", Color::Red),
        ("\\x1b[1;31m", Color::Red),
        ("\\x1b[33m", Color::Yellow),
        ("\\x1b[1;33m", Color::Yellow),
        ("\\x1b[34m", Color::Blue),
        ("\\x1b[35m", Color::Magenta),
        ("\\x1b[36m", Color::Cyan),
        ("\\x1b[37m", Color::White),
    ];

    pub fn decode(input: Span) -> Vec<Span> {
        let mut spans = vec![];
        let style = input.style;
        let mut color = input.style.fg.unwrap_or(Color::Reset);
        let iter = input.content.to_special_char(|string| {
            let mut least_pos = usize::MAX;
            let mut found_pattern = None;

            for (pattern, _) in Self::PATTERNS {
                if let Some(start) = string.find(pattern)
                    && start < least_pos
                {
                    least_pos = start;
                    found_pattern = Some(pattern);
                }
            }

            found_pattern.map(|found_pattern| {
                let start = string[..least_pos].chars().count();
                let length = found_pattern.chars().count();
                (start, length).into()
            })
        });

        for item in iter {
            match item {
                SpecialCharItem::Plain(plain) => {
                    spans.push(Span::styled(plain, style.fg(color)));
                }
                SpecialCharItem::Special(special, _) => {
                    'pattern_loop: for (pattern, new_fg) in Self::PATTERNS {
                        if special == pattern {
                            color = new_fg;
                            break 'pattern_loop;
                        }
                    }
                }
            }
        }

        spans
    }

    pub fn remove_encoding(input: String) -> String {
        let mut result = String::new();
        let iter = input.to_special_char(|string| {
            let mut least_pos = usize::MAX;
            let mut found_pattern = None;

            for (pattern, _) in Self::PATTERNS {
                if let Some(start) = string.find(pattern)
                    && start < least_pos
                {
                    least_pos = start;
                    found_pattern = Some(pattern);
                }
            }

            found_pattern.map(|found_pattern| {
                let start = string[..least_pos].chars().count();
                let length = found_pattern.chars().count();
                (start, length).into()
            })
        });

        for item in iter {
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
}
