use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Encodes a crossterm [`KeyEvent`] into the byte sequence a terminal would
/// send to a program over the wire. This is the inverse of crossterm's input
/// parser (which turns wire bytes into `KeyEvent`s); crossterm ships no
/// encoder, so headless raw mode needs this to forward each keystroke straight
/// to the serial/RTT interface.
///
/// Returns an empty vector for keys that have no sensible byte representation
/// (modifier-only presses, media keys, out-of-range function keys, …), meaning
/// "send nothing".
///
/// `Ctrl+K` is intercepted by the caller (it toggles the command mode) and
/// never reaches this function, so it is not special-cased here.
pub fn encode_key(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl && c.is_ascii_alphabetic() {
                // Ctrl+A..=Ctrl+Z map to the control bytes 0x01..=0x1a.
                vec![(c.to_ascii_uppercase() as u8) - b'A' + 1]
            } else {
                // `encode_utf8` keeps multi-byte characters (accents, emoji)
                // intact — `c as u8` would truncate them.
                let mut buf = [0u8; 4];
                let encoded = c.encode_utf8(&mut buf).as_bytes();

                if alt {
                    // xterm "meta sends escape": ESC followed by the character.
                    let mut out = Vec::with_capacity(encoded.len() + 1);
                    out.push(0x1b);
                    out.extend_from_slice(encoded);
                    out
                } else {
                    encoded.to_vec()
                }
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::Insert => b"\x1b[2~".to_vec(),
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::F(n) => encode_function_key(n),
        KeyCode::Null => vec![0x00],
        // CapsLock, ScrollLock, media keys, modifier-only presses, etc.
        _ => Vec::new(),
    }
}

/// The classic xterm/VT function-key sequences. F1..=F4 use the SS3 (`ESC O`)
/// form; F5..=F12 use the CSI `~` form (note the deliberate gaps at 16 and 22,
/// which is why this is a lookup, not arithmetic).
fn encode_function_key(n: u8) -> Vec<u8> {
    match n {
        1 => b"\x1bOP".to_vec(),
        2 => b"\x1bOQ".to_vec(),
        3 => b"\x1bOR".to_vec(),
        4 => b"\x1bOS".to_vec(),
        5 => b"\x1b[15~".to_vec(),
        6 => b"\x1b[17~".to_vec(),
        7 => b"\x1b[18~".to_vec(),
        8 => b"\x1b[19~".to_vec(),
        9 => b"\x1b[20~".to_vec(),
        10 => b"\x1b[21~".to_vec(),
        11 => b"\x1b[23~".to_vec(),
        12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn plain_ascii_char_is_its_byte() {
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            b"a"
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('Z'), KeyModifiers::SHIFT)),
            b"Z"
        );
    }

    #[test]
    fn multibyte_char_keeps_all_utf8_bytes() {
        // 'é' is U+00E9 -> 0xC3 0xA9 in UTF-8; must not be truncated to one byte.
        assert_eq!(
            encode_key(key(KeyCode::Char('é'), KeyModifiers::NONE)),
            "é".as_bytes()
        );
    }

    #[test]
    fn ctrl_letter_maps_to_control_byte() {
        // Ctrl+C is 0x03, Ctrl+A is 0x01 — case-insensitive.
        assert_eq!(
            encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            [0x03]
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            [0x01]
        );
        assert_eq!(
            encode_key(key(KeyCode::Char('A'), KeyModifiers::CONTROL)),
            [0x01]
        );
    }

    #[test]
    fn alt_char_is_escape_prefixed() {
        assert_eq!(
            encode_key(key(KeyCode::Char('a'), KeyModifiers::ALT)),
            [0x1b, b'a']
        );
    }

    #[test]
    fn named_keys_map_to_expected_sequences() {
        assert_eq!(encode_key(key(KeyCode::Enter, KeyModifiers::NONE)), b"\r");
        assert_eq!(encode_key(key(KeyCode::Tab, KeyModifiers::NONE)), b"\t");
        assert_eq!(
            encode_key(key(KeyCode::Backspace, KeyModifiers::NONE)),
            [0x7f]
        );
        assert_eq!(encode_key(key(KeyCode::Esc, KeyModifiers::NONE)), [0x1b]);
        assert_eq!(encode_key(key(KeyCode::Up, KeyModifiers::NONE)), b"\x1b[A");
        assert_eq!(
            encode_key(key(KeyCode::Down, KeyModifiers::NONE)),
            b"\x1b[B"
        );
        assert_eq!(
            encode_key(key(KeyCode::Right, KeyModifiers::NONE)),
            b"\x1b[C"
        );
        assert_eq!(
            encode_key(key(KeyCode::Left, KeyModifiers::NONE)),
            b"\x1b[D"
        );
        assert_eq!(
            encode_key(key(KeyCode::Delete, KeyModifiers::NONE)),
            b"\x1b[3~"
        );
    }

    #[test]
    fn function_keys_use_ss3_then_csi() {
        assert_eq!(
            encode_key(key(KeyCode::F(1), KeyModifiers::NONE)),
            b"\x1bOP"
        );
        assert_eq!(
            encode_key(key(KeyCode::F(5), KeyModifiers::NONE)),
            b"\x1b[15~"
        );
        assert_eq!(
            encode_key(key(KeyCode::F(12), KeyModifiers::NONE)),
            b"\x1b[24~"
        );
    }

    #[test]
    fn unmapped_keys_emit_nothing() {
        // Out-of-range function key and a lock key have no representation.
        assert!(encode_key(key(KeyCode::F(20), KeyModifiers::NONE)).is_empty());
        assert!(encode_key(key(KeyCode::CapsLock, KeyModifiers::NONE)).is_empty());
    }
}
