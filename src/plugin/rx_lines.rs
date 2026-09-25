//! Line framing for the `on_*_recv_line` plugin hooks (issue #250).
//!
//! The interface tasks do not publish received bytes in a fixed shape: the TUI
//! frames them into `\n`-terminated lines (flushing a partial one after an idle
//! second), while headless forwards them as they arrive — byte by byte on
//! serial, one read chunk at a time on RTT. The legacy `on_*_recv` hook sees
//! whatever shape the bus carries, so the same plugin behaved differently in
//! the two modes. The engine therefore re-frames the stream itself, and only
//! ever hands a plugin a complete line, whatever chunking produced it.

/// A partial line longer than this is dropped rather than buffered forever: a
/// binary stream, or a device that never sends `\n`, would otherwise grow the
/// buffer for as long as the session runs.
const MAX_LINE_LEN: usize = 64 * 1024;

#[derive(Default)]
pub struct RxLines {
    pending: Vec<u8>,
}

/// What one [`RxLines::push`] produced.
#[derive(Debug, Default, PartialEq)]
pub struct Framed {
    /// Every line completed by the pushed bytes, each ending in its `\n` (and
    /// carrying a `\r` before it, when the device sent one).
    pub lines: Vec<Vec<u8>>,
    /// How many bytes of an over-long partial line were thrown away.
    pub dropped: usize,
}

impl RxLines {
    pub fn push(&mut self, bytes: &[u8]) -> Framed {
        let mut framed = Framed::default();

        for &byte in bytes {
            self.pending.push(byte);

            if byte == b'\n' {
                framed.lines.push(std::mem::take(&mut self.pending));
            } else if self.pending.len() >= MAX_LINE_LEN {
                framed.dropped += self.pending.len();
                self.pending.clear();
            }
        }

        framed
    }

    /// Forget a partial line, so it is not glued to the first bytes of the
    /// next connection.
    pub fn reset(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(framed: Framed) -> Vec<Vec<u8>> {
        assert_eq!(framed.dropped, 0);
        framed.lines
    }

    #[test]
    fn a_whole_line_is_passed_through_with_its_terminator() {
        let mut rx = RxLines::default();
        assert_eq!(lines(rx.push(b"hello\r\n")), vec![b"hello\r\n".to_vec()]);
    }

    #[test]
    fn byte_by_byte_input_yields_the_same_line() {
        // Headless serial publishes one byte per message.
        let mut rx = RxLines::default();
        let mut got = vec![];
        for byte in b"hello\r\n" {
            got.extend(lines(rx.push(&[*byte])));
        }
        assert_eq!(got, vec![b"hello\r\n".to_vec()]);
    }

    #[test]
    fn a_chunk_is_split_into_lines_and_the_tail_is_kept() {
        // Headless RTT publishes whole read chunks.
        let mut rx = RxLines::default();
        assert_eq!(
            lines(rx.push(b"one\ntwo\nthr")),
            vec![b"one\n".to_vec(), b"two\n".to_vec()]
        );
        assert_eq!(lines(rx.push(b"ee\n")), vec![b"three\n".to_vec()]);
    }

    #[test]
    fn a_partial_line_is_never_delivered() {
        // The TUI flushes a partial line after an idle second; that flush must
        // not reach the line hook as a line of its own.
        let mut rx = RxLines::default();
        assert!(lines(rx.push(b"$ ")).is_empty());
        assert_eq!(lines(rx.push(b"ls\n")), vec![b"$ ls\n".to_vec()]);
    }

    #[test]
    fn an_empty_line_is_still_a_line() {
        let mut rx = RxLines::default();
        assert_eq!(
            lines(rx.push(b"\n\r\n")),
            vec![b"\n".to_vec(), b"\r\n".to_vec()]
        );
    }

    #[test]
    fn reset_drops_the_partial_line() {
        let mut rx = RxLines::default();
        rx.push(b"stale");
        rx.reset();
        assert_eq!(lines(rx.push(b"fresh\n")), vec![b"fresh\n".to_vec()]);
    }

    #[test]
    fn an_over_long_partial_line_is_dropped() {
        let mut rx = RxLines::default();
        let framed = rx.push(&vec![b'x'; MAX_LINE_LEN + 3]);
        assert!(framed.lines.is_empty());
        assert_eq!(framed.dropped, MAX_LINE_LEN);
        // Only the bytes after the drop make it into the next line.
        assert_eq!(lines(rx.push(b"\n")), vec![b"xxx\n".to_vec()]);
    }
}
