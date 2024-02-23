use crate::text::ViewData;
use chrono::{DateTime, Local};
use std::collections::HashMap;
use tui::style::{Color, Style};
use tui::text::Span;

pub struct RichText {
    content: Vec<u8>,
    fg: Color,
    bg: Color,
}

impl RichText {
    pub fn new(content: Vec<u8>, fg: Color, bg: Color) -> Self {
        Self { content, fg, bg }
    }

    pub fn from_string(content: String, fg: Color, bg: Color) -> Self {
        Self {
            content: content.as_bytes().to_vec(),
            fg,
            bg,
        }
    }

    pub fn decode_ansi_color(self) -> RichTextAnsi {
        RichTextAnsi::new(self)
    }

    pub fn highlight_invisible(self) -> RichTextWithInvisible {
        RichTextWithInvisible::new(self)
    }

    pub fn to_span<'a>(&self) -> Span<'a> {
        Span::styled(
            String::from_utf8_lossy(&self.content).to_string(),
            Style::default().bg(self.bg).fg(self.fg),
        )
    }

    pub fn crop_prefix_len(&self, len: usize) -> Self {
        Self {
            content: if len >= self.content.len() {
                vec![]
            } else {
                self.content[len..].to_vec()
            },
            fg: self.fg,
            bg: self.bg,
        }
    }
}

pub struct RichTextWithInvisible {
    rich_texts: Vec<RichText>,
}

impl RichTextWithInvisible {
    pub fn into_view_data(self, timestamp: DateTime<Local>) -> ViewData {
        ViewData::new(timestamp, self.rich_texts)
    }

    fn new(rich_text: RichText) -> Self {
        if rich_text.content.is_empty() {
            return Self { rich_texts: vec![] };
        }

        enum State {
            None,
            Visible,
            Invisible,
        }

        let (fg, bg) = (rich_text.fg, rich_text.bg);
        let (hl_fg, hl_bg) = Self::get_colors(rich_text.fg, rich_text.bg, true);

        let (buffer, state, acc) = rich_text.content.into_iter().fold(
            (vec![], State::None, vec![]),
            |(buffer, state, acc), byte| match state {
                State::None => (
                    vec![byte],
                    Self::is_visible(byte)
                        .then_some(State::Visible)
                        .unwrap_or(State::Invisible),
                    acc,
                ),
                State::Visible => {
                    if Self::is_visible(byte) {
                        (
                            buffer.into_iter().chain([byte]).collect(),
                            State::Visible,
                            acc,
                        )
                    } else {
                        (
                            vec![byte],
                            State::Invisible,
                            acc.into_iter()
                                .chain([RichText::new(buffer, rich_text.fg, rich_text.bg)])
                                .collect(),
                        )
                    }
                }
                State::Invisible => {
                    if Self::is_visible(byte) {
                        (
                            vec![byte],
                            State::Visible,
                            acc.into_iter()
                                .chain([RichText::new(Self::bytes_to_rich(buffer), hl_fg, hl_bg)])
                                .collect(),
                        )
                    } else {
                        (
                            buffer.into_iter().chain([byte]).collect(),
                            State::Invisible,
                            acc,
                        )
                    }
                }
            },
        );

        Self {
            rich_texts: if buffer.is_empty() {
                acc
            } else {
                let (fg, bg) = Self::get_colors(fg, bg, matches!(state, State::Invisible));
                let buffer = if matches!(state, State::Invisible) {
                    Self::bytes_to_rich(buffer)
                } else {
                    buffer
                };

                acc.into_iter()
                    .chain([RichText::new(buffer, fg, bg)])
                    .collect()
            },
        }
    }

    fn is_visible(byte: u8) -> bool {
        0x20 <= byte && byte <= 0x7E
    }

    fn bytes_to_rich(bytes: Vec<u8>) -> Vec<u8> {
        bytes
            .into_iter()
            .flat_map(|b| match b {
                b'\n' => b"\\n".to_vec(),
                b'\r' => b"\\r".to_vec(),
                b'\0' => b"\\0".to_vec(),
                x => format!("\\x{:02x}", x).as_bytes().to_vec(),
            })
            .collect()
    }

    fn get_colors(fg: Color, bg: Color, is_highlight: bool) -> (Color, Color) {
        if !is_highlight {
            return (fg, bg);
        }

        if bg == Color::Magenta
            || bg == Color::LightMagenta
            || fg == Color::Magenta
            || fg == Color::LightMagenta
        {
            (Color::LightYellow, bg)
        } else {
            (Color::LightMagenta, bg)
        }
    }
}

pub struct RichTextAnsi {
    rich_texts: Vec<RichText>,
}

impl RichTextAnsi {
    pub fn highlight_invisible(self) -> RichTextWithInvisible {
        let rich_texts = self
            .rich_texts
            .into_iter()
            .flat_map(|rich_text| RichTextWithInvisible::new(rich_text).rich_texts)
            .collect::<Vec<_>>();

        RichTextWithInvisible { rich_texts }
    }

    fn new(rich_text: RichText) -> Self {
        if rich_text.content.is_empty() {
            return RichTextAnsi { rich_texts: vec![] };
        }

        enum State {
            None,
            Escape,
            Normal,
        }

        let (fg, bg) = (rich_text.fg, rich_text.bg);
        let (text_buffer, _color_pattern, state, acc, current_fg, current_bg) =
            rich_text.content.into_iter().fold(
                (vec![], vec![], State::None, Vec::<RichText>::new(), fg, bg),
                |(text_buffer, color_pattern, state, acc, current_fg, current_bg), byte| match state
                {
                    State::None => {
                        if byte == 0x1B {
                            (
                                vec![],
                                vec![byte],
                                State::Escape,
                                acc,
                                current_fg,
                                current_bg,
                            )
                        } else {
                            (
                                vec![byte],
                                vec![],
                                State::Normal,
                                acc,
                                current_fg,
                                current_bg,
                            )
                        }
                    }
                    State::Escape => {
                        let color_pattern_ext =
                            color_pattern.into_iter().chain([byte]).collect::<Vec<_>>();
                        match RichTextAnsi::match_color_pattern(&color_pattern_ext) {
                            Ok(Some((new_fg, new_bg))) => (
                                vec![],
                                vec![],
                                State::Normal,
                                acc.into_iter()
                                    .chain([RichText {
                                        content: text_buffer,
                                        fg: current_fg,
                                        bg: current_bg,
                                    }])
                                    .collect::<Vec<_>>(),
                                new_fg,
                                new_bg,
                            ),
                            Ok(None) => (
                                text_buffer,
                                color_pattern_ext,
                                State::Escape,
                                acc,
                                current_fg,
                                current_bg,
                            ),
                            Err(_) => (
                                text_buffer
                                    .into_iter()
                                    .chain(color_pattern_ext)
                                    .collect::<Vec<_>>(),
                                vec![],
                                State::Normal,
                                acc,
                                current_fg,
                                current_bg,
                            ),
                        }
                    }
                    State::Normal => {
                        if byte == 0x1B {
                            (
                                text_buffer,
                                vec![byte],
                                State::Escape,
                                acc,
                                current_fg,
                                current_bg,
                            )
                        } else {
                            (
                                text_buffer.into_iter().chain([byte]).collect(),
                                vec![],
                                State::Normal,
                                acc,
                                current_fg,
                                current_bg,
                            )
                        }
                    }
                },
            );

        Self {
            rich_texts: if text_buffer.is_empty() {
                acc
            } else {
                acc.into_iter()
                    .chain([match state {
                        State::None => unreachable!(),
                        State::Normal | State::Escape => RichText {
                            content: text_buffer,
                            fg: current_fg,
                            bg: current_bg,
                        },
                    }])
                    .collect()
            },
        }
    }

    fn match_color_pattern(pattern: &[u8]) -> Result<Option<(Color, Color)>, ()> {
        let pattern_lut = HashMap::new()
            .into_iter()
            .chain([
                (b"\x1B[m".to_vec(), (Color::Reset, Color::Reset)),
                (b"\x1B[0m".to_vec(), (Color::Reset, Color::Reset)),
                (b"\x1B[1m".to_vec(), (Color::Reset, Color::Reset)),
                (b"\x1B[30m".to_vec(), (Color::Black, Color::Reset)),
                (b"\x1B[0;30m".to_vec(), (Color::Black, Color::Reset)),
                (b"\x1B[1;30m".to_vec(), (Color::Black, Color::Reset)),
                (b"\x1B[31m".to_vec(), (Color::Red, Color::Reset)),
                (b"\x1B[0;31m".to_vec(), (Color::Red, Color::Reset)),
                (b"\x1B[1;31m".to_vec(), (Color::Red, Color::Reset)),
                (b"\x1B[32m".to_vec(), (Color::Green, Color::Reset)),
                (b"\x1B[0;32m".to_vec(), (Color::Green, Color::Reset)),
                (b"\x1B[1;32m".to_vec(), (Color::Green, Color::Reset)),
                (b"\x1B[33m".to_vec(), (Color::Yellow, Color::Reset)),
                (b"\x1B[0;33m".to_vec(), (Color::Yellow, Color::Reset)),
                (b"\x1B[1;33m".to_vec(), (Color::Yellow, Color::Reset)),
                (b"\x1B[34m".to_vec(), (Color::Blue, Color::Reset)),
                (b"\x1B[0;34m".to_vec(), (Color::Blue, Color::Reset)),
                (b"\x1B[1;34m".to_vec(), (Color::Blue, Color::Reset)),
                (b"\x1B[35m".to_vec(), (Color::Magenta, Color::Reset)),
                (b"\x1B[0;35m".to_vec(), (Color::Magenta, Color::Reset)),
                (b"\x1B[1;35m".to_vec(), (Color::Magenta, Color::Reset)),
                (b"\x1B[36m".to_vec(), (Color::Cyan, Color::Reset)),
                (b"\x1B[0;36m".to_vec(), (Color::Cyan, Color::Reset)),
                (b"\x1B[1;36m".to_vec(), (Color::Cyan, Color::Reset)),
                (b"\x1B[37m".to_vec(), (Color::Gray, Color::Reset)),
                (b"\x1B[0;37m".to_vec(), (Color::Gray, Color::Reset)),
                (b"\x1B[1;37m".to_vec(), (Color::Gray, Color::Reset)),
            ])
            .collect::<HashMap<Vec<u8>, (Color, Color)>>();

        if let Some((fg, bg)) = pattern_lut.get(pattern) {
            return Ok(Some((*fg, *bg)));
        }

        if pattern_lut.keys().any(|x| x.starts_with(pattern)) {
            return Ok(None);
        }

        if pattern.starts_with(b"\x1b[") && pattern[2..].iter().all(|x| x.is_ascii_digit()) {
            return Ok(None);
        }

        if pattern == b"\x1b[J" {
            return Ok(Some((Color::Reset, Color::Reset)));
        }

        if pattern.starts_with(b"\x1b[")
            && [b'A', b'B', b'C', b'D'].contains(pattern.last().unwrap())
        {
            return Ok(Some((Color::Reset, Color::Reset)));
        }

        Err(())
    }
}
