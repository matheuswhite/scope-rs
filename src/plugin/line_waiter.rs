//! Matching of received lines for `serial.send_and_wait` / `rtt.send_and_wait`.

use regex::bytes::Regex;
use std::{
    collections::VecDeque,
    sync::LazyLock,
    time::{Duration, Instant},
};

/// An ANSI control sequence (CSI), as a shell uses to color its output.
static ANSI_ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap());

/// What a plugin waits for after sending, as `send_and_wait` built it from the
/// call arguments and the plugin's `M.config`.
#[derive(Clone, Debug)]
pub struct WaitSpec {
    pub expected: String,
    pub error_pattern: Option<String>,
    pub ignore: Vec<String>,
    /// Patterns removed from every received line before it is matched.
    pub strip: Vec<String>,
    /// The device's echo of what was sent: a line equal to one of these, or
    /// ending with a space and one of these, is dropped.
    pub echoes: Vec<Vec<u8>>,
    pub timeout: Option<Duration>,
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
    strip: Vec<Regex>,
    echoes: Vec<Vec<u8>>,
    timeout: Option<Duration>,
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
        let strip = spec
            .strip
            .iter()
            .map(|pattern| {
                Regex::new(pattern)
                    .map_err(|err| format!("invalid strip_msg pattern \"{pattern}\": {err}"))
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            expected: whole_line(&spec.expected, "expected")?,
            error,
            ignore,
            strip,
            echoes: spec.echoes.clone(),
            timeout: spec.timeout,
            deadline: None,
            lines: vec![],
        })
    }

    /// Takes one received line, its terminator included.
    pub fn feed(&mut self, line: &[u8]) -> Option<WaitOutcome> {
        let mut line = ANSI_ESCAPE.replace_all(line, &b""[..]).into_owned();
        for strip in &self.strip {
            line = strip.replace_all(&line, &b""[..]).into_owned();
        }
        let line = trim_end(&line);

        if line.is_empty() || self.is_echo(line) || self.ignore.iter().any(|re| re.is_match(line)) {
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

    fn is_echo(&self, line: &[u8]) -> bool {
        self.echoes
            .iter()
            .filter(|echo| !echo.is_empty())
            .any(|echo| {
                line == echo.as_slice()
                    || line
                        .strip_suffix(echo.as_slice())
                        .is_some_and(|before| before.ends_with(b" "))
            })
    }

    /// Starts the timeout, as the message is sent at `now`.
    pub fn start(&mut self, now: Instant) {
        self.deadline = self.timeout.and_then(|timeout| now.checked_add(timeout));
    }

    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|deadline| now >= deadline)
    }
}

/// The `send_and_wait` calls in flight: a call is sent only once the one
/// before it has ended.
pub struct WaitQueue<K> {
    waits: VecDeque<Queued<K>>,
}

struct Queued<K> {
    key: K,
    message: Option<Vec<u8>>,
    waiter: LineWaiter,
}

impl<K> Default for WaitQueue<K> {
    fn default() -> Self {
        Self {
            waits: VecDeque::new(),
        }
    }
}

impl<K> WaitQueue<K> {
    pub fn push(&mut self, key: K, message: Vec<u8>, waiter: LineWaiter) {
        self.waits.push_back(Queued {
            key,
            message: Some(message),
            waiter,
        });
    }

    /// The message of the call that has just become the one waiting, to be
    /// sent at `now`.
    pub fn next_to_send(&mut self, now: Instant) -> Option<Vec<u8>> {
        let active = self.waits.front_mut()?;
        let message = active.message.take()?;
        active.waiter.start(now);

        Some(message)
    }

    /// Takes one received line for the call whose message was sent, and
    /// returns that call's result when the line ends it.
    pub fn feed(&mut self, line: &[u8]) -> Option<(K, WaitResult)> {
        let active = self.waits.front_mut()?;
        if active.message.is_some() {
            return None;
        }

        let result = match active.waiter.feed(line)? {
            WaitOutcome::Matched { lines, matched } => Ok((lines, matched)),
            WaitOutcome::Failed { line } => Err(String::from_utf8_lossy(&line).into_owned()),
        };
        let active = self
            .waits
            .pop_front()
            .expect("the active wait was just fed");

        Some((active.key, result))
    }

    /// Ends the call that was sent and has waited past its timeout.
    pub fn expire(&mut self, now: Instant) -> Option<K> {
        let active = self.waits.front()?;
        if active.message.is_some() || !active.waiter.expired(now) {
            return None;
        }

        self.waits.pop_front().map(|active| active.key)
    }

    /// Drops every call whose key matches, sent or not.
    pub fn cancel(&mut self, mut matches: impl FnMut(&K) -> bool) -> Vec<K> {
        let (cancelled, kept) = std::mem::take(&mut self.waits)
            .into_iter()
            .partition::<VecDeque<_>, _>(|queued| matches(&queued.key));
        self.waits = kept;

        cancelled.into_iter().map(|queued| queued.key).collect()
    }
}

fn whole_line(pattern: &str, what: &str) -> Result<Regex, String> {
    Regex::new(&format!("^(?:{pattern})$"))
        .map_err(|err| format!("invalid {what} pattern \"{pattern}\": {err}"))
}

fn trim_end(mut line: &[u8]) -> &[u8] {
    while let [rest @ .., last] = line
        && last.is_ascii_whitespace()
    {
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
            strip: vec![],
            echoes: vec![],
            timeout: None,
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
    fn expires_once_the_timeout_has_run_from_the_send() {
        let now = Instant::now();
        let mut waiter = LineWaiter::new(&WaitSpec {
            timeout: Some(Duration::from_millis(200)),
            ..spec("OK")
        })
        .unwrap();

        assert!(!waiter.expired(now + Duration::from_secs(3600)));

        waiter.start(now);
        assert!(!waiter.expired(now + Duration::from_millis(199)));
        assert!(waiter.expired(now + Duration::from_millis(200)));
    }

    #[test]
    fn never_expires_without_a_timeout() {
        let mut waiter = LineWaiter::new(&spec("OK")).unwrap();
        let now = Instant::now();
        waiter.start(now);

        assert!(!waiter.expired(now + Duration::from_secs(3600)));
    }

    #[test]
    fn ansi_escapes_and_strip_patterns_are_removed_before_matching() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            strip: vec![r"uart:~\$ ?".to_string()],
            ..spec("OK")
        })
        .unwrap();

        assert_eq!(
            feed_all(
                &mut waiter,
                &[
                    "\x1b[1;32muart:~$ \x1b[m\r\n",
                    "uart:~$ \x1b[1m+COPS: 0,0,\"TIM\",7\x1b[0m \r\n",
                    "\x1b[mOK\r\n",
                ]
            ),
            matched(&["+COPS: 0,0,\"TIM\",7"], "OK")
        );
    }

    #[test]
    fn the_echo_of_the_message_is_dropped() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            echoes: vec![b"modem at \"AT+CSQ\"".to_vec(), b"AT+CSQ".to_vec()],
            ..spec("OK")
        })
        .unwrap();

        assert_eq!(
            feed_all(
                &mut waiter,
                &[
                    "uart:~$ modem at \"AT+CSQ\"\r\n",
                    "AT+CSQ\r\n",
                    "+CSQ: 20,99\r\n",
                    "OK\r\n"
                ]
            ),
            matched(&["+CSQ: 20,99"], "OK")
        );
    }

    #[test]
    fn a_line_that_only_ends_like_the_message_is_not_an_echo() {
        let mut waiter = LineWaiter::new(&WaitSpec {
            echoes: vec![b"AT".to_vec()],
            ..spec("OK")
        })
        .unwrap();

        assert_eq!(
            feed_all(&mut waiter, &["+QSPN: \"CHAT\"\r\n", "FLAT\r\n", "OK\r\n"]),
            matched(&["+QSPN: \"CHAT\"", "FLAT"], "OK")
        );
    }

    fn queue_with(keys: &[&'static str]) -> WaitQueue<&'static str> {
        let mut queue = WaitQueue::default();
        for key in keys {
            queue.push(
                *key,
                key.as_bytes().to_vec(),
                LineWaiter::new(&spec("OK")).unwrap(),
            );
        }
        queue
    }

    #[test]
    fn a_queued_call_is_sent_only_after_the_one_before_it_ends() {
        let mut queue = queue_with(&["AT+CSQ", "AT+COPS?"]);
        let now = Instant::now();

        assert_eq!(queue.next_to_send(now), Some(b"AT+CSQ".to_vec()));
        assert_eq!(queue.next_to_send(now), None);

        assert_eq!(queue.feed(b"+CSQ: 20,99\r\n"), None);
        assert_eq!(
            queue.feed(b"OK\r\n"),
            Some((
                "AT+CSQ",
                Ok((vec![b"+CSQ: 20,99".to_vec()], b"OK".to_vec()))
            ))
        );

        assert_eq!(queue.next_to_send(now), Some(b"AT+COPS?".to_vec()));
    }

    #[test]
    fn a_line_before_the_send_reaches_no_call() {
        let mut queue = queue_with(&["AT"]);

        assert_eq!(queue.feed(b"OK\r\n"), None);

        queue.next_to_send(Instant::now());
        assert!(queue.feed(b"OK\r\n").is_some());
    }

    #[test]
    fn an_error_line_ends_the_active_call() {
        let mut queue = WaitQueue::default();
        queue.push(
            "AT+CSQ",
            vec![],
            LineWaiter::new(&WaitSpec {
                error_pattern: Some(r"\+CME ERROR: .*".to_string()),
                ..spec("OK")
            })
            .unwrap(),
        );
        queue.next_to_send(Instant::now());

        assert_eq!(
            queue.feed(b"+CME ERROR: 10\r\n"),
            Some(("AT+CSQ", Err("+CME ERROR: 10".to_string())))
        );
    }

    #[test]
    fn only_the_sent_call_expires() {
        let mut queue = WaitQueue::default();
        for key in ["first", "second"] {
            queue.push(
                key,
                vec![],
                LineWaiter::new(&WaitSpec {
                    timeout: Some(Duration::from_millis(100)),
                    ..spec("OK")
                })
                .unwrap(),
            );
        }
        let now = Instant::now();
        let later = now + Duration::from_millis(100);

        assert_eq!(queue.expire(later), None);

        queue.next_to_send(now);
        assert_eq!(queue.expire(later), Some("first"));
        assert_eq!(queue.expire(later), None);

        queue.next_to_send(later);
        assert_eq!(queue.expire(later + Duration::from_millis(99)), None);
        assert_eq!(
            queue.expire(later + Duration::from_millis(100)),
            Some("second")
        );
    }

    #[test]
    fn cancel_drops_the_matching_calls_and_lets_the_next_one_go() {
        let mut queue = queue_with(&["a1", "b1", "a2"]);
        let now = Instant::now();
        queue.next_to_send(now);

        assert_eq!(queue.cancel(|key| key.starts_with('a')), vec!["a1", "a2"]);
        assert_eq!(queue.next_to_send(now), Some(b"b1".to_vec()));
    }
}
