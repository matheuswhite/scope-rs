use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Turns a raw byte stream (as delivered by stdin or `/dev/tty`) back into
/// [`KeyEvent`]s. This is the inverse of [`encode_key`](super::key_encode::encode_key)
/// and the byte-level counterpart of crossterm's own input parser (which we
/// cannot reuse — it is private to crossterm's event source).
///
/// It exists so bytes arriving on stdin can be fed into the *same*
/// keystroke-handling path as the physical keyboard: `Ctrl+K` still opens the
/// command bar, `$..`/`@..`/`!..` are still parsed on Enter, etc. The keyboard
/// (via crossterm) and stdin (via this decoder) therefore converge on one
/// pipeline in both headless and TUI mode.
///
/// The decoder is stateful: [`feed`](Self::feed) buffers bytes that form an
/// incomplete UTF-8 character or escape sequence and completes them when more
/// bytes arrive. Call [`flush`](Self::flush) at end-of-input to release a
/// pending lone `ESC` as a bare `Esc` key.
#[derive(Default)]
pub struct KeyDecoder {
    buf: Vec<u8>,
}

enum Step {
    /// Emit this key, having consumed `usize` bytes from the front of the buffer.
    Emit(KeyEvent, usize),
    /// Consume `usize` bytes without emitting (unrecognized escape sequence).
    Skip(usize),
    /// The buffer holds a valid but incomplete prefix; wait for more bytes.
    NeedMore,
}

/// Result of trying to read one (possibly multi-byte) character off a slice.
enum CharStep {
    Char(char, usize),
    NeedMore,
}

impl KeyDecoder {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Feed a chunk of bytes and return every complete key it produced.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<KeyEvent> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();

        loop {
            if self.buf.is_empty() {
                break;
            }
            match Self::decode_one(&self.buf) {
                Step::Emit(key, consumed) => {
                    self.buf.drain(..consumed);
                    out.push(key);
                }
                Step::Skip(consumed) => {
                    self.buf.drain(..consumed);
                }
                Step::NeedMore => break,
            }
        }

        // A lone trailing `ESC` is ambiguous (bare `Esc` vs the start of an
        // escape sequence). Terminals disambiguate with a timeout; a keyboard
        // and a scripted stdin both deliver a full escape sequence within a
        // single read, so if a bare `ESC` is all that's left at the end of a
        // chunk it was a real `Esc` press — emit it now instead of stranding it
        // until the next byte (which would break `Esc` to quit / go back).
        if self.buf == [0x1b] {
            self.buf.clear();
            out.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }

        out
    }

    /// Release any buffered-but-undecidable bytes at end-of-input. A lone `ESC`
    /// that never got a following byte becomes a bare `Esc`; other incomplete
    /// fragments (a truncated escape sequence or UTF-8 char) are dropped.
    pub fn flush(&mut self) -> Vec<KeyEvent> {
        let mut out = Vec::new();
        if self.buf.first() == Some(&0x1b) && self.buf.len() == 1 {
            out.push(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        self.buf.clear();
        out
    }

    fn decode_one(buf: &[u8]) -> Step {
        let b0 = buf[0];

        match b0 {
            0x1b => Self::decode_escape(buf),
            b'\r' | b'\n' => Step::Emit(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 1),
            b'\t' => Step::Emit(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE), 1),
            0x7f => Step::Emit(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE), 1),
            0x00 => Step::Emit(KeyEvent::new(KeyCode::Null, KeyModifiers::NONE), 1),
            // Ctrl+A..=Ctrl+Z arrive as 0x01..=0x1a (Tab/Enter already handled).
            0x01..=0x1a => {
                let letter = (b0 - 1 + b'a') as char;
                Step::Emit(
                    KeyEvent::new(KeyCode::Char(letter), KeyModifiers::CONTROL),
                    1,
                )
            }
            // Other C0 controls (FS/GS/RS/US) have no keyboard-equivalent our
            // encoder produces; pass them through as their Latin-1 char.
            0x1c..=0x1f => Step::Emit(
                KeyEvent::new(KeyCode::Char(b0 as char), KeyModifiers::NONE),
                1,
            ),
            _ => match Self::decode_char(buf) {
                CharStep::Char(c, len) => {
                    Step::Emit(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE), len)
                }
                CharStep::NeedMore => Step::NeedMore,
            },
        }
    }

    /// Decode a sequence beginning with `ESC` (buf[0] == 0x1b).
    fn decode_escape(buf: &[u8]) -> Step {
        if buf.len() < 2 {
            return Step::NeedMore;
        }

        match buf[1] {
            b'[' => Self::decode_csi(buf),
            b'O' => Self::decode_ss3(buf),
            // ESC + char == Alt/meta + char (xterm "meta sends escape").
            _ => match Self::decode_char(&buf[1..]) {
                CharStep::Char(c, len) => {
                    Step::Emit(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT), 1 + len)
                }
                CharStep::NeedMore => Step::NeedMore,
            },
        }
    }

    /// `ESC [ ...` — arrows, Home/End, BackTab, and the `<n>~` editing/function keys.
    fn decode_csi(buf: &[u8]) -> Step {
        if buf.len() < 3 {
            return Step::NeedMore;
        }

        // Single-letter finals.
        let code = match buf[2] {
            b'A' => Some(KeyCode::Up),
            b'B' => Some(KeyCode::Down),
            b'C' => Some(KeyCode::Right),
            b'D' => Some(KeyCode::Left),
            b'H' => Some(KeyCode::Home),
            b'F' => Some(KeyCode::End),
            b'Z' => Some(KeyCode::BackTab),
            _ => None,
        };
        if let Some(code) = code {
            return Step::Emit(KeyEvent::new(code, KeyModifiers::NONE), 3);
        }

        // `ESC [ <digits> ~` form.
        if buf[2].is_ascii_digit() {
            let mut i = 2;
            while i < buf.len() && buf[i].is_ascii_digit() {
                i += 1;
            }
            if i >= buf.len() {
                return Step::NeedMore; // digits not yet terminated
            }
            if buf[i] != b'~' {
                return Step::Skip(i + 1); // unknown modifier form; drop it
            }
            let num: u16 = std::str::from_utf8(&buf[2..i])
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let consumed = i + 1;
            let code = match num {
                2 => Some(KeyCode::Insert),
                3 => Some(KeyCode::Delete),
                5 => Some(KeyCode::PageUp),
                6 => Some(KeyCode::PageDown),
                15 => Some(KeyCode::F(5)),
                17 => Some(KeyCode::F(6)),
                18 => Some(KeyCode::F(7)),
                19 => Some(KeyCode::F(8)),
                20 => Some(KeyCode::F(9)),
                21 => Some(KeyCode::F(10)),
                23 => Some(KeyCode::F(11)),
                24 => Some(KeyCode::F(12)),
                _ => None,
            };
            return match code {
                Some(code) => Step::Emit(KeyEvent::new(code, KeyModifiers::NONE), consumed),
                None => Step::Skip(consumed),
            };
        }

        Step::Skip(3) // unrecognized CSI final byte
    }

    /// `ESC O <P|Q|R|S>` — the SS3 form for F1..=F4.
    fn decode_ss3(buf: &[u8]) -> Step {
        if buf.len() < 3 {
            return Step::NeedMore;
        }
        let code = match buf[2] {
            b'P' => Some(KeyCode::F(1)),
            b'Q' => Some(KeyCode::F(2)),
            b'R' => Some(KeyCode::F(3)),
            b'S' => Some(KeyCode::F(4)),
            _ => None,
        };
        match code {
            Some(code) => Step::Emit(KeyEvent::new(code, KeyModifiers::NONE), 3),
            None => Step::Skip(3),
        }
    }

    /// Read one UTF-8 character from the front of `buf`. Returns `NeedMore` when
    /// a multi-byte sequence is only partially present. Invalid lead/continuation
    /// bytes fall back to a single Latin-1 char so nothing is silently dropped.
    fn decode_char(buf: &[u8]) -> CharStep {
        let b0 = buf[0];
        let len = match b0 {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf7 => 4,
            // Stray continuation / invalid lead byte: emit as Latin-1.
            _ => return CharStep::Char(b0 as char, 1),
        };

        if len == 1 {
            return CharStep::Char(b0 as char, 1);
        }
        if buf.len() < len {
            return CharStep::NeedMore;
        }
        match std::str::from_utf8(&buf[..len]) {
            Ok(s) => match s.chars().next() {
                Some(c) => CharStep::Char(c, len),
                None => CharStep::Char(b0 as char, 1),
            },
            // Malformed sequence: consume just the lead byte as Latin-1.
            Err(_) => CharStep::Char(b0 as char, 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inputs::key_encode::encode_key;

    fn one(bytes: &[u8]) -> KeyEvent {
        let mut d = KeyDecoder::new();
        let keys = d.feed(bytes);
        assert_eq!(keys.len(), 1, "expected exactly one key from {:?}", bytes);
        keys.into_iter().next().unwrap()
    }

    #[test]
    fn plain_ascii() {
        assert_eq!(one(b"a").code, KeyCode::Char('a'));
        assert_eq!(one(b"a").modifiers, KeyModifiers::NONE);
    }

    #[test]
    fn control_letters_map_back() {
        // Ctrl+K (the command-bar trigger) is 0x0b.
        let k = one(&[0x0b]);
        assert_eq!(k.code, KeyCode::Char('k'));
        assert_eq!(k.modifiers, KeyModifiers::CONTROL);
        let c = one(&[0x03]);
        assert_eq!(c.code, KeyCode::Char('c'));
        assert_eq!(c.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn named_keys() {
        assert_eq!(one(b"\r").code, KeyCode::Enter);
        assert_eq!(one(b"\n").code, KeyCode::Enter);
        assert_eq!(one(b"\t").code, KeyCode::Tab);
        assert_eq!(one(&[0x7f]).code, KeyCode::Backspace);
        assert_eq!(one(b"\x1b[A").code, KeyCode::Up);
        assert_eq!(one(b"\x1b[B").code, KeyCode::Down);
        assert_eq!(one(b"\x1b[C").code, KeyCode::Right);
        assert_eq!(one(b"\x1b[D").code, KeyCode::Left);
        assert_eq!(one(b"\x1b[3~").code, KeyCode::Delete);
        assert_eq!(one(b"\x1b[H").code, KeyCode::Home);
        assert_eq!(one(b"\x1bOP").code, KeyCode::F(1));
        assert_eq!(one(b"\x1b[15~").code, KeyCode::F(5));
        assert_eq!(one(b"\x1b[24~").code, KeyCode::F(12));
    }

    #[test]
    fn alt_char() {
        let k = one(&[0x1b, b'a']);
        assert_eq!(k.code, KeyCode::Char('a'));
        assert_eq!(k.modifiers, KeyModifiers::ALT);
    }

    #[test]
    fn lone_esc_emitted_at_end_of_feed() {
        // A bare ESC that is the whole chunk is a real `Esc` press and must be
        // emitted immediately (a keyboard/stdin delivers escape sequences whole),
        // otherwise `Esc` to quit/go-back would never fire.
        let mut d = KeyDecoder::new();
        let keys = d.feed(&[0x1b]);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].code, KeyCode::Esc);
    }

    #[test]
    fn complete_escape_sequence_is_not_split() {
        // A full sequence in one chunk decodes to the key, not to Esc + chars.
        let mut d = KeyDecoder::new();
        let keys = d.feed(b"\x1b[A");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].code, KeyCode::Up);
    }

    #[test]
    fn multibyte_utf8_across_feeds() {
        // 'é' is 0xC3 0xA9; split across two feeds must not corrupt it.
        let mut d = KeyDecoder::new();
        assert!(d.feed(&[0xc3]).is_empty());
        let keys = d.feed(&[0xa9]);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].code, KeyCode::Char('é'));
    }

    #[test]
    fn multiple_keys_in_one_feed() {
        let mut d = KeyDecoder::new();
        let keys = d.feed(b"hi\r");
        let codes: Vec<KeyCode> = keys.iter().map(|k| k.code).collect();
        assert_eq!(
            codes,
            vec![KeyCode::Char('h'), KeyCode::Char('i'), KeyCode::Enter]
        );
    }

    #[test]
    fn round_trips_encode_key() {
        // Every key encode_key emits must decode back to the same key.
        let cases = [
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Insert, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('a'), KeyModifiers::ALT),
        ];
        for expected in cases {
            let bytes = encode_key(expected);
            let mut d = KeyDecoder::new();
            let mut got = d.feed(&bytes);
            got.extend(d.flush());
            assert_eq!(got.len(), 1, "round-trip of {:?} -> {:?}", expected, bytes);
            assert_eq!(got[0].code, expected.code, "code mismatch for {:?}", bytes);
            assert_eq!(
                got[0].modifiers, expected.modifiers,
                "modifier mismatch for {:?}",
                bytes
            );
        }
    }
}
