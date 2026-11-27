//! Matching of received lines for `serial.send_and_wait` / `rtt.send_and_wait`.

use regex::bytes::Regex;
use std::time::Instant;

/// What a plugin waits for after sending, as `send_and_wait` built it from the
/// call arguments and the plugin's `M.config`.
#[derive(Clone, Debug)]
pub struct WaitSpec {
    pub expected: String,
    pub error_pattern: Option<String>,
    pub ignore: Vec<String>,
    pub deadline: Option<Instant>,
}

/// The lines received before the expected one and the expected line itself,
/// or why the wait failed.
pub type WaitResult = Result<(Vec<Vec<u8>>, Vec<u8>), String>;

#[derive(Debug, PartialEq)]
pub enum WaitOutcome {
    /// The expected line arrived; `lines` are the ones received before it.
    Matched {
        lines: Vec<Vec<u8>>,
        matched: Vec<u8>,
    },
    /// A line matched the error pattern.
    Failed { line: Vec<u8> },
}

pub struct LineWaiter {
    expected: Regex,
    error: Option<Regex>,
    ignore: Vec<Regex>,
    deadline: Option<Instant>,
    lines: Vec<Vec<u8>>,
}

impl LineWaiter {
    pub fn new(spec: &WaitSpec) -> Result<Self, String> {
        let error = spec
            .error_pattern
            .as_deref()
            .map(|pattern| whole_line(pattern, "error_pattern"))
            .transpose()?;
        let ignore = spec
            .ignore
            .iter()
            .map(|pattern| whole_line(pattern, "ignore_msg"))
            .collect::<Result<_, _>>()?;

        Ok(Self {
            expected: whole_line(&spec.expected, "expected")?,
            error,
            ignore,
            deadline: spec.deadline,
            lines: vec![],
        })
    }

    /// Takes one received line, its terminator included.
    pub fn feed(&mut self, line: &[u8]) -> Option<WaitOutcome> {
        let line = trim_line_ending(line);

        if line.is_empty() || self.ignore.iter().any(|re| re.is_match(line)) {
            return None;
        }

        if self.error.as_ref().is_some_and(|re| re.is_match(line)) {
            return Some(WaitOutcome::Failed {
                line: line.to_vec(),
            });
        }

        if self.expected.is_match(line) {
            return Some(WaitOutcome::Matched {
                lines: std::mem::take(&mut self.lines),
                matched: line.to_vec(),
            });
        }

        self.lines.push(line.to_vec());
        None
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
}

fn whole_line(pattern: &str, what: &str) -> Result<Regex, String> {
    Regex::new(&format!("^(?:{pattern})$"))
        .map_err(|err| format!("invalid {what} pattern \"{pattern}\": {err}"))
}

fn trim_line_ending(mut line: &[u8]) -> &[u8] {
    while let [rest @ .., b'\r' | b'\n'] = line {
        line = rest;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec(expected: &str) -> WaitSpec {
        WaitSpec {
            expected: expected.to_string(),
            error_pattern: None,
            ignore: vec![],
            deadline: None,
        }
    }

    fn feed_all(waiter: &mut LineWaiter, lines: &[&str]) -> Option<WaitOutcome> {
        lines.iter().find_map(|line| waiter.feed(line.as_bytes()))
    }

    fn matched(lines: &[&str], matched: &str) -> Option<WaitOutcome> {
        Some(WaitOutcome::Matched {
            lines: lines.iter().map(|l| l.as_bytes().to_vec()).collect(),
            matched: matched.as_bytes().to_vec(),
        })
    }

    #[test]
    fn returns_the_lines_before_the_expected_one_without_terminators() {
        let mut waiter = LineWaiter::new(&spec("OK")).unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["+COPS: 0,0,\"TIM\",7\r\n", "OK\r\n"]),
            matched(&["+COPS: 0,0,\"TIM\",7"], "OK")
        );
    }

    #[test]
    fn the_expected_pattern_must_match_the_whole_line() {
        let mut waiter = LineWaiter::new(&spec("OK")).unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["+CPBR: 1,\"BOOK\"\r\n", "OK\r\n"]),
            matched(&["+CPBR: 1,\"BOOK\""], "OK")
        );
    }

    #[test]
    fn empty_lines_are_skipped() {
        let mut waiter = LineWaiter::new(&spec("OK")).unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["\r\n", "+CSQ: 20,99\r\n", "\r\n", "OK\r\n"]),
            matched(&["+CSQ: 20,99"], "OK")
        );
    }

    #[test]
    fn ignored_lines_are_neither_returned_nor_matched() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            ignore: vec!["uart.*".to_string()],
            ..spec("OK|uart.*")
        })
        .unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["uart:~$ modem at AT\r\n", "OK\r\n"]),
            matched(&[], "OK")
        );
    }

    #[test]
    fn an_error_line_ends_the_wait() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            error_pattern: Some(r"\+CME ERROR: .*|ERROR".to_string()),
            ..spec("OK")
        })
        .unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["+CME ERROR: 10\r\n", "OK\r\n"]),
            Some(WaitOutcome::Failed {
                line: b"+CME ERROR: 10".to_vec()
            })
        );
    }

    #[test]
    fn the_error_pattern_is_checked_before_the_expected_one() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            error_pattern: Some("ERROR".to_string()),
            ..spec(".*")
        })
        .unwrap();

        assert!(matches!(
            waiter.feed(b"ERROR\r\n"),
            Some(WaitOutcome::Failed { .. })
        ));
    }

    #[test]
    fn an_invalid_pattern_names_the_setting() {
        let err = LineWaiter::new(&WaitSpec {
            ignore: vec!["(".to_string()],
            ..spec("OK")
        })
        .err()
        .unwrap();

        assert!(err.starts_with("invalid ignore_msg pattern \"(\""), "{err}");
    }

    #[test]
    fn expires_at_the_deadline() {
        let now = Instant::now();
        let waiter = LineWaiter::new(&WaitSpec {
            deadline: Some(now + Duration::from_millis(200)),
            ..spec("OK")
        })
        .unwrap();

        assert!(!waiter.expired(now));
        assert!(waiter.expired(now + Duration::from_millis(200)));
    }

    #[test]
    fn never_expires_without_a_deadline() {
        let waiter = LineWaiter::new(&spec("OK")).unwrap();

        assert!(!waiter.expired(Instant::now() + Duration::from_secs(3600)));
    }
}
