use crate::graphics::{
    buffer::{BufferLine, LineBytes},
    screen::ScreenDecoder,
};
use regex::Regex;

/// A regex applied line by line to decide which messages appear in the history
/// view. It has two modes, driven by two commands that share the same slot:
///
/// - **allow** (`!filter`, the default): a line is shown when it matches the
///   pattern. `!filter` with no pattern resets to `.*`, showing everything.
/// - **mute** (`!mute`): a line is *hidden* when it matches the pattern.
///   `!mute` with no pattern mutes everything (`.*` in mute mode).
///
/// Only received (RX) data is subject to it; transmitted data and system logs
/// are always shown. The filter never touches what is persisted to disk
/// (session file, crash backup, recording).
///
/// The pattern is matched with [`Regex::is_match`], which is **not anchored**:
/// it succeeds when the pattern is found anywhere in the line. Use `^`/`$` to
/// anchor to the start/end of a line.
pub struct MessageFilter {
    pattern: String,
    regex: Regex,
    exclude: bool,
}

impl Default for MessageFilter {
    fn default() -> Self {
        Self::new(Self::DEFAULT_PATTERN, false)
            .expect("the default message filter pattern must always compile")
    }
}

impl MessageFilter {
    /// The pattern applied when no filter is set (and on every new session):
    /// `.*` matches every line, so all messages are shown.
    pub const DEFAULT_PATTERN: &'static str = ".*";

    pub fn new(pattern: &str, exclude: bool) -> Result<Self, String> {
        let regex = Regex::new(pattern).map_err(|err| err.to_string())?;
        Ok(Self {
            pattern: pattern.to_string(),
            regex,
            exclude,
        })
    }

    /// The filter produced by `!mute` with no pattern: `.*` in mute mode, so
    /// every received line is hidden.
    pub fn mute_all() -> Self {
        Self::new(Self::DEFAULT_PATTERN, true)
            .expect("the default message filter pattern must always compile")
    }

    /// The indicator shown in the command bar. Both modes share this slot and
    /// stay distinguishable by a leading `!`: `[<pattern>]` when filtering
    /// (allow mode) and `![<pattern>]` when muting (exclude mode). The brackets
    /// are part of the label so the `!` can sit outside them.
    pub fn label(&self) -> String {
        if self.exclude {
            format!("![{}]", self.pattern)
        } else {
            format!("[{}]", self.pattern)
        }
    }

    /// Whether `line` should be displayed. The filter only applies to received
    /// (RX) data lines; transmitted data (`is_tx`) and system logs (`level`) are
    /// always allowed through. The pattern is matched against the line as the
    /// screen shows it ([`ScreenDecoder::plain_text`]), mirroring what search
    /// matches against. In exclude
    /// mode the match result is inverted.
    pub fn allows(&self, line: &BufferLine<LineBytes>, decoder: ScreenDecoder) -> bool {
        if line.level.is_some() || line.is_tx {
            return true;
        }

        let message = decoder.plain_text(&line.message);
        // allow mode: keep matches; exclude mode: keep non-matches.
        self.regex.is_match(&message) != self.exclude
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn rx(message: &str) -> BufferLine<LineBytes> {
        BufferLine::new_rx(Local::now(), message.as_bytes().to_vec())
    }

    fn tx(message: &str) -> BufferLine<LineBytes> {
        BufferLine::new_tx(Local::now(), message.as_bytes().to_vec())
    }

    fn log(message: &str) -> BufferLine<LineBytes> {
        BufferLine::new_log(
            Local::now(),
            crate::infra::LogLevel::Error,
            message.as_bytes().to_vec(),
        )
    }

    // Issue #239: the filter matches what the screen shows.
    #[test]
    fn filter_matches_bytes_in_the_display_format() {
        use crate::graphics::screen::HexFormat;

        let line = BufferLine::new_rx(Local::now(), vec![b'x', 0xa5]);
        let filter = MessageFilter::new("0xA5", false).unwrap();

        assert!(filter.allows(&line, ScreenDecoder::new(HexFormat::PrefixedUpper)));
        assert!(!filter.allows(&line, ScreenDecoder::default()));
    }

    #[test]
    fn default_pattern_allows_every_line() {
        let filter = MessageFilter::default();

        assert_eq!(filter.label(), "[.*]");
        assert!(filter.allows(&rx("anything at all"), ScreenDecoder::default()));
        assert!(filter.allows(&rx(""), ScreenDecoder::default()));
    }

    #[test]
    fn allow_mode_keeps_only_matching_lines() {
        let filter = MessageFilter::new("sensor", false).unwrap();

        assert!(filter.allows(&rx("sensor: 42"), ScreenDecoder::default()));
        assert!(!filter.allows(&rx("dbg: heartbeat"), ScreenDecoder::default()));
    }

    #[test]
    fn exclude_mode_hides_matching_lines() {
        let filter = MessageFilter::new("dbg", true).unwrap();

        assert_eq!(filter.label(), "![dbg]");
        // Matching lines are hidden, everything else is shown.
        assert!(!filter.allows(&rx("dbg: heartbeat"), ScreenDecoder::default()));
        assert!(filter.allows(&rx("sensor: 42"), ScreenDecoder::default()));
    }

    #[test]
    fn mute_all_hides_every_received_line() {
        let filter = MessageFilter::mute_all();

        assert_eq!(filter.label(), "![.*]");
        // Every RX line is hidden...
        assert!(!filter.allows(&rx("anything at all"), ScreenDecoder::default()));
        assert!(!filter.allows(&rx(""), ScreenDecoder::default()));
        // ...but transmitted data and system logs still come through, so the
        // "everything is muted" warning itself remains visible.
        assert!(filter.allows(&tx("a command I sent"), ScreenDecoder::default()));
        assert!(filter.allows(&log("all messages are muted"), ScreenDecoder::default()));
    }

    #[test]
    fn is_match_is_unanchored_use_caret_for_line_start() {
        // Unanchored: "dbg" anywhere in the line counts as a match.
        let anywhere = MessageFilter::new("dbg", true).unwrap();
        assert!(!anywhere.allows(&rx("late dbg here"), ScreenDecoder::default())); // hidden

        // Anchored to the start with ^: only lines that begin with "dbg".
        let at_start = MessageFilter::new("^dbg", true).unwrap();
        assert!(!at_start.allows(&rx("dbg: tick"), ScreenDecoder::default())); // hidden
        assert!(at_start.allows(&rx("late dbg here"), ScreenDecoder::default())); // shown
    }

    #[test]
    fn exclude_lines_starting_with_bracket() {
        // The "[00:00:..]" clock noise starts with '[' (escaped as \[).
        let filter = MessageFilter::new(r"^\[", true).unwrap();

        assert!(!filter.allows(&rx("[00:00:35] clock init"), ScreenDecoder::default())); // hidden
        assert!(filter.allows(&rx("Die temperature: 27 C"), ScreenDecoder::default())); // shown
    }

    #[test]
    fn filter_ignores_ansi_when_matching() {
        // ANSI escapes are stripped before matching, so ^\[ sees the real first
        // glyph, not the color code.
        let filter = MessageFilter::new(r"^\[", true).unwrap();

        assert!(!filter.allows(
            &rx("\x1b[0m[00:00:35] init\x1b[0m"),
            ScreenDecoder::default()
        )); // hidden
        assert!(filter.allows(&rx("\x1b[1;31m--- dropped ---"), ScreenDecoder::default())); // shown
    }

    #[test]
    fn tx_and_log_lines_are_always_allowed() {
        // Even in exclude mode with a pattern that matches them.
        let filter = MessageFilter::new("dbg", true).unwrap();

        assert!(filter.allows(&tx("dbg: a command I sent"), ScreenDecoder::default()));
        assert!(filter.allows(&log("dbg in a log message"), ScreenDecoder::default()));
        assert!(!filter.allows(&rx("dbg: received noise"), ScreenDecoder::default()));
    }

    #[test]
    fn invalid_pattern_is_rejected() {
        assert!(MessageFilter::new("(unclosed", false).is_err());
        assert!(MessageFilter::new("(unclosed", true).is_err());
    }
}
