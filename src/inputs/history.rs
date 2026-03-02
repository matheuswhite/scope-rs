use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo};

struct Matcher {
    matcher: Nucleo<usize>,
}

impl Matcher {
    fn new() -> Self {
        Self {
            matcher: Nucleo::new(Config::DEFAULT, Arc::new(|| {}), Some(1), 1),
        }
    }

    fn fuzy_match_idx(&mut self, haystack: &[String], needle: &str) -> Vec<usize> {
        if needle.is_empty() {
            return (0..haystack.len()).collect();
        }

        self.matcher.restart(true);

        let injector = self.matcher.injector();

        for (i, entry) in haystack.iter().enumerate() {
            let text: nucleo::Utf32String = entry.as_str().into();
            injector.push(i, |_, cols| {
                cols[0] = text;
            });
        }

        self.matcher
            .pattern
            .reparse(0, needle, CaseMatching::Ignore, Normalization::Never, false);

        // Wait for matcher to finish, so to not block other threads
        loop {
            let status = self.matcher.tick(10);
            if !status.running {
                break;
            }
        }

        let snapshot = self.matcher.snapshot();
        snapshot.matched_items(..).map(|item| *item.data).collect()
    }
}

pub enum HistoryNavResult<'a> {
    /// History is empty; nothing to navigate.
    Empty,
    /// Show this history entry.
    Entry(&'a str),
    /// Navigated past the newest entry; restore what was typed before navigation began.
    RestoreBackup,
}

// ── History ──────────────────────────────────────────────────────────────────

/// Pure in-memory history with keyboard navigation. No file I/O.
/// Suitable as the base for any kind of history (command, search, fuzzy, …).
pub struct History {
    entries: Vec<String>,
    index: Option<usize>,
    /// Snapshot of the input before the user started navigating.
    backup: String,

    matcher: Matcher,
    fuzy_entries: Vec<usize>,
}

impl History {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: None,
            backup: String::new(),
            matcher: Matcher::new(),
            fuzy_entries: Vec::new(),
        }
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    /// Pushes `entry` to the history if it's not a consecutive duplicate. Returns `Ok(true)` if
    /// the entry was added, `Ok(false)` if it was a consecutive duplicate, or `Err` if there was
    /// an error
    pub fn push(&mut self, entry: &str) -> Result<bool, String> {
        if self.entries.last().map(String::as_str) != Some(entry) {
            self.entries.push(entry.to_owned());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Navigate to an older entry. Saves `current_line` as backup on the first call.
    pub fn navigate_up(&mut self, current_line: &str) -> HistoryNavResult<'_> {
        if self.entries.is_empty() {
            return HistoryNavResult::Empty;
        }
        let idx = match self.index {
            None => {
                self.backup = current_line.to_owned();

                self.fuzy_entries = self.matcher.fuzy_match_idx(&self.entries, &self.backup);

                if self.fuzy_entries.is_empty() {
                    return HistoryNavResult::Empty;
                }

                self.fuzy_entries.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.index = Some(idx);

        if !self.fuzy_entries.is_empty() {
            HistoryNavResult::Entry(&self.entries[self.fuzy_entries[idx]])
        } else {
            HistoryNavResult::Empty
        }
    }

    /// Navigate to a newer entry, or back to the saved backup when past the newest.
    pub fn navigate_down(&mut self) -> HistoryNavResult<'_> {
        if self.fuzy_entries.is_empty() {
            return HistoryNavResult::Empty;
        }
        match self.index {
            None => HistoryNavResult::Empty,
            Some(i) if i == self.fuzy_entries.len() - 1 => {
                self.index = None;
                HistoryNavResult::RestoreBackup
            }
            Some(i) => {
                let next = i + 1;
                self.index = Some(next);
                HistoryNavResult::Entry(&self.entries[self.fuzy_entries[next]])
            }
        }
    }

    /// Reset the navigation cursor (e.g. after submitting or typing).
    pub fn reset_index(&mut self) {
        self.index = None;
    }

    pub fn backup(&self) -> &str {
        &self.backup
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

// ── PersistHistory ────────────────────────────────────────────────────────────

/// History backed by a file on disk. Loads existing entries at construction and
/// appends new ones on every `push`. Navigation is delegated to the inner `History`.
pub struct PersistHistory {
    inner: History,
    file_path: PathBuf,
}

impl PersistHistory {
    fn resolve_file_path(file_path: &str) -> Result<PathBuf, String> {
        let dir = dirs::data_dir().ok_or_else(|| "cannot determine data directory".to_string())?;
        let dir = dir.join("scope");
        fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
        Ok(dir.join(file_path))
    }

    pub fn new(file_path: &str) -> Result<Self, String> {
        let mut inner = History::new();

        let file_path = Self::resolve_file_path(file_path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .truncate(false)
            .create(true)
            .open(&file_path)
            .map_err(|err| {
                format!(
                    "cannot open history file at {}: {}",
                    file_path.display(),
                    err
                )
            })?;

        inner.entries = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|l| !l.is_empty())
            .collect();

        Ok(Self { inner, file_path })
    }

    /// Pushes `entry` to the inner history and appends it to the file if it was not a consecutive
    /// duplicate.
    ///
    /// Returns `Ok(true)` if the entry was added, `Ok(false)` if it was a consecutive duplicate,
    /// or Err if there was a file I/O error.
    pub fn push(&mut self, entry: &str) -> Result<bool, String> {
        if !self.inner.push(entry).is_ok_and(|x| x) {
            return Ok(false); // deduplicated — not a failure
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)
            .map_err(|err| {
                format!(
                    "cannot open history file at {}: {}",
                    self.file_path.display(),
                    err
                )
            })?;

        writeln!(file, "{entry}").map_err(|err| {
            format!(
                "cannot write to history file at {}: {}",
                self.file_path.display(),
                err
            )
        })?;

        Ok(true)
    }

    pub fn navigate_up(&mut self, current_line: &str) -> HistoryNavResult<'_> {
        self.inner.navigate_up(current_line)
    }

    pub fn navigate_down(&mut self) -> HistoryNavResult<'_> {
        self.inner.navigate_down()
    }

    pub fn reset_index(&mut self) {
        self.inner.reset_index();
    }

    pub fn backup(&self) -> &str {
        self.inner.backup()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

// ── AnyHistory ────────────────────────────────────────────────────────────────

/// Either a file-backed [`PersistHistory`] or a plain in-memory [`History`].
pub enum AnyHistory {
    Persist(PersistHistory),
    Base(History),
}

impl AnyHistory {
    pub fn push(&mut self, entry: &str) -> Result<bool, String> {
        match self {
            AnyHistory::Persist(h) => h.push(entry),
            AnyHistory::Base(h) => h.push(entry),
        }
    }

    pub fn navigate_up(&mut self, current_line: &str) -> HistoryNavResult<'_> {
        match self {
            AnyHistory::Persist(h) => h.navigate_up(current_line),
            AnyHistory::Base(h) => h.navigate_up(current_line),
        }
    }

    pub fn navigate_down(&mut self) -> HistoryNavResult<'_> {
        match self {
            AnyHistory::Persist(h) => h.navigate_down(),
            AnyHistory::Base(h) => h.navigate_down(),
        }
    }

    pub fn reset_index(&mut self) {
        match self {
            AnyHistory::Persist(h) => h.reset_index(),
            AnyHistory::Base(h) => h.reset_index(),
        }
    }

    pub fn backup(&self) -> &str {
        match self {
            AnyHistory::Persist(h) => h.backup(),
            AnyHistory::Base(h) => h.backup(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            AnyHistory::Persist(h) => h.is_empty(),
            AnyHistory::Base(h) => h.is_empty(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Test helpers ──────────────────────────────────────────────────────────

    impl History {
        pub fn iter(&self) -> std::slice::Iter<'_, String> {
            self.entries.iter()
        }

        fn new_with_entries(cmds: &[&str]) -> Self {
            let mut h = Self::new();
            for cmd in cmds {
                let _ = h.push(cmd);
            }
            h
        }
    }

    impl PersistHistory {
        pub fn iter(&self) -> std::slice::Iter<'_, String> {
            self.inner.iter()
        }

        fn new_with_path(path: PathBuf) -> Self {
            Self {
                inner: History::new(),
                file_path: path,
            }
        }

        /// Mirrors the loading logic of `PersistHistory::new()` but accepts an explicit path,
        /// bypassing `resolve_file_path`. Used only in tests.
        fn new_loaded_from_path(path: PathBuf) -> Result<Self, String> {
            let mut inner = History::new();
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .truncate(false)
                .create(true)
                .open(&path)
                .map_err(|err| {
                    format!("cannot open history file at {}: {}", path.display(), err)
                })?;
            inner.entries = BufReader::new(file)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.is_empty())
                .collect();
            Ok(Self {
                inner,
                file_path: path,
            })
        }
    }

    fn temp_path(suffix: &str) -> PathBuf {
        use std::time::SystemTime;
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        std::env::temp_dir().join(format!("scope_history_test_{}_{}.tmp", nanos, suffix))
    }

    // ── Matcher ───────────────────────────────────────────────────────────────

    mod matcher {
        use super::*;

        #[test]
        fn empty_haystack_returns_empty() {
            let mut m = Matcher::new();
            assert!(m.fuzy_match_idx(&[], "foo").is_empty());
        }

        #[test]
        fn empty_needle_returns_all_indices_in_order() {
            let haystack = ["a", "b", "c"].map(String::from).to_vec();
            let mut m = Matcher::new();
            assert_eq!(m.fuzy_match_idx(&haystack, ""), vec![0, 1, 2]);
        }

        #[test]
        fn empty_needle_on_empty_haystack_returns_empty() {
            let mut m = Matcher::new();
            assert!(m.fuzy_match_idx(&[], "").is_empty());
        }

        #[test]
        fn needle_matches_subset_of_entries() {
            let haystack = ["serial send", "rtt recv", "serial recv"]
                .map(String::from)
                .to_vec();
            let mut m = Matcher::new();
            let result = m.fuzy_match_idx(&haystack, "rtt");
            assert_eq!(result.len(), 1);
            assert_eq!(haystack[result[0]], "rtt recv");
        }

        #[test]
        fn needle_with_no_match_returns_empty() {
            let haystack = ["foo", "bar"].map(String::from).to_vec();
            let mut m = Matcher::new();
            assert!(m.fuzy_match_idx(&haystack, "xyz").is_empty());
        }

        #[test]
        fn match_is_case_insensitive() {
            let haystack = ["Hello World", "other"].map(String::from).to_vec();
            let mut m = Matcher::new();
            let result = m.fuzy_match_idx(&haystack, "hello");
            assert_eq!(result.len(), 1);
            assert_eq!(haystack[result[0]], "Hello World");
        }

        #[test]
        fn single_entry_exact_match() {
            let haystack = ["exact"].map(String::from).to_vec();
            let mut m = Matcher::new();
            assert_eq!(m.fuzy_match_idx(&haystack, "exact"), vec![0]);
        }

        #[test]
        fn single_entry_no_match() {
            let haystack = ["exact"].map(String::from).to_vec();
            let mut m = Matcher::new();
            assert!(m.fuzy_match_idx(&haystack, "xyz").is_empty());
        }

        #[test]
        fn all_entries_match_broad_needle() {
            let haystack = ["foo bar", "foo baz", "foo qux"].map(String::from).to_vec();
            let mut m = Matcher::new();
            assert_eq!(m.fuzy_match_idx(&haystack, "foo").len(), 3);
        }

        #[test]
        fn consecutive_calls_are_independent() {
            let haystack = ["alpha", "beta", "gamma"].map(String::from).to_vec();
            let mut m = Matcher::new();
            let first = m.fuzy_match_idx(&haystack, "alpha");
            let second = m.fuzy_match_idx(&haystack, "beta");
            assert_eq!(first.len(), 1);
            assert_eq!(haystack[first[0]], "alpha");
            assert_eq!(second.len(), 1);
            assert_eq!(haystack[second[0]], "beta");
        }
    }

    // ── History ───────────────────────────────────────────────────────────────

    mod history {
        use super::*;

        mod push {
            use super::*;

            #[test]
            fn adds_entry() {
                let mut h = History::new();
                let _ = h.push("hello");
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["hello"]);
            }

            #[test]
            fn returns_true_when_added() {
                let mut h = History::new();
                assert_eq!(h.push("cmd"), Ok(true));
            }

            #[test]
            fn returns_false_on_consecutive_duplicate() {
                let mut h = History::new();
                let _ = h.push("dup");
                assert_eq!(h.push("dup"), Ok(false));
            }

            #[test]
            fn deduplicates_consecutive_in_entries() {
                let mut h = History::new();
                let _ = h.push("dup");
                let _ = h.push("dup");
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["dup"]);
            }

            #[test]
            fn allows_non_consecutive_duplicates() {
                let mut h = History::new();
                let _ = h.push("a");
                let _ = h.push("b");
                let _ = h.push("a");
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["a", "b", "a"]);
            }

            #[test]
            fn allows_empty_string() {
                let mut h = History::new();
                assert_eq!(h.push(""), Ok(true));
                assert_eq!(h.iter().collect::<Vec<_>>(), vec![""]);
            }

            #[test]
            fn multiple_entries_preserve_order() {
                let mut h = History::new();
                let _ = h.push("first");
                let _ = h.push("second");
                let _ = h.push("third");
                assert_eq!(
                    h.iter().collect::<Vec<_>>(),
                    vec!["first", "second", "third"]
                );
            }
        }

        mod is_empty {
            use super::*;

            #[test]
            fn true_on_new() {
                assert!(History::new().is_empty());
            }

            #[test]
            fn false_after_push() {
                let mut h = History::new();
                let _ = h.push("cmd");
                assert!(!h.is_empty());
            }
        }

        mod navigate_up {
            use super::*;

            #[test]
            fn on_empty_history_returns_empty() {
                let mut h = History::new();
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Empty));
            }

            #[test]
            fn saves_backup_on_first_call() {
                let mut h = History::new_with_entries(&["a"]);
                h.navigate_up("my_draft");
                assert_eq!(h.backup(), "my_draft");
            }

            #[test]
            fn does_not_update_backup_on_subsequent_calls() {
                let mut h = History::new_with_entries(&["a", "b"]);
                // First up: backup set to "", index advances to Some(1)
                h.navigate_up("");
                // Second up: index is now Some(1), None arm is NOT entered; current_line is ignored
                h.navigate_up("should_be_ignored");
                assert_eq!(h.backup(), "");
            }

            #[test]
            fn returns_last_entry_first() {
                let mut h = History::new_with_entries(&["a", "b", "c"]);
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("c")));
            }

            #[test]
            fn traverses_entries_from_newest_to_oldest() {
                let mut h = History::new_with_entries(&["a", "b", "c"]);
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("c")));
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("b")));
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("a")));
            }

            #[test]
            fn clamps_at_oldest_entry() {
                let mut h = History::new_with_entries(&["a", "b"]);
                h.navigate_up("");
                h.navigate_up("");
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("a")));
            }

            #[test]
            fn single_entry_clamps_immediately() {
                let mut h = History::new_with_entries(&["only"]);
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("only")));
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("only")));
            }

            #[test]
            fn fuzzy_filter_excludes_non_matching_entries() {
                let mut h = History::new_with_entries(&["serial send", "rtt recv", "serial recv"]);
                // All navigate_up results must match "rtt"
                let r = h.navigate_up("rtt");
                if let HistoryNavResult::Entry(e) = r {
                    assert!(e.contains("rtt"));
                }
            }

            #[test]
            fn fuzzy_no_matches_returns_empty() {
                let mut h = History::new_with_entries(&["foo", "bar"]);
                assert!(matches!(h.navigate_up("xyz"), HistoryNavResult::Empty));
            }

            #[test]
            fn fuzzy_single_match_clamps_immediately() {
                let mut h = History::new_with_entries(&["rtt recv", "serial send"]);
                // "rtt" matches only "rtt recv"
                assert!(matches!(
                    h.navigate_up("rtt"),
                    HistoryNavResult::Entry("rtt recv")
                ));
                assert!(matches!(
                    h.navigate_up(""),
                    HistoryNavResult::Entry("rtt recv")
                ));
            }
        }

        mod navigate_down {
            use super::*;

            #[test]
            fn on_empty_history_returns_empty() {
                let mut h = History::new();
                assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
            }

            #[test]
            fn without_prior_up_returns_empty() {
                let mut h = History::new_with_entries(&["a", "b"]);
                assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
            }

            #[test]
            fn returns_newer_entry_after_going_up() {
                let mut h = History::new_with_entries(&["a", "b", "c"]);
                h.navigate_up("");
                h.navigate_up("");
                assert!(matches!(h.navigate_down(), HistoryNavResult::Entry("c")));
            }

            #[test]
            fn past_newest_returns_restore_backup() {
                let mut h = History::new_with_entries(&["a", "b"]);
                h.navigate_up(""); // At "b" (newest, empty query = all entries)
                assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
            }

            #[test]
            fn restore_backup_preserves_backup_value() {
                let mut h = History::new_with_entries(&["a"]);
                h.navigate_up("my_draft");
                h.navigate_down();
                assert_eq!(h.backup(), "my_draft");
            }

            #[test]
            fn full_up_down_cycle() {
                let mut h = History::new_with_entries(&["a", "b", "c"]);
                h.navigate_up("");
                h.navigate_up("");
                h.navigate_up("");
                assert!(matches!(h.navigate_down(), HistoryNavResult::Entry("b")));
                assert!(matches!(h.navigate_down(), HistoryNavResult::Entry("c")));
                assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
            }

            #[test]
            fn fuzzy_filtered_down_stays_within_matches() {
                // "rtt" matches only "rtt recv" (index 0)
                let mut h = History::new_with_entries(&["rtt recv", "serial send"]);
                h.navigate_up("rtt");
                // Only one match — going down should immediately restore
                assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
            }

            #[test]
            fn fuzzy_no_matches_then_down_returns_empty() {
                let mut h = History::new_with_entries(&["foo", "bar"]);
                h.navigate_up("xyz"); // Empty — no matches, index stays None
                assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
            }
        }

        mod reset_index {
            use super::*;

            #[test]
            fn navigate_down_returns_empty_after_reset() {
                let mut h = History::new_with_entries(&["a", "b"]);
                h.navigate_up("");
                h.reset_index();
                assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
            }

            #[test]
            fn navigate_up_restarts_from_newest_after_reset() {
                let mut h = History::new_with_entries(&["a", "b", "c"]);
                h.navigate_up("");
                h.navigate_up("");
                h.navigate_up("");
                h.reset_index();
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("c")));
            }

            #[test]
            fn reset_updates_backup_on_next_navigate_up() {
                let mut h = History::new_with_entries(&["a", "b"]);
                h.navigate_up("first_draft");
                h.reset_index();
                h.navigate_up("second_draft");
                assert_eq!(h.backup(), "second_draft");
            }
        }
    }

    // ── PersistHistory ────────────────────────────────────────────────────────

    mod persist_history {
        use super::*;

        mod push {
            use super::*;

            #[test]
            fn creates_file_and_persists_entries() {
                let path = temp_path("create");
                let _ = fs::remove_file(&path);
                {
                    let mut h = PersistHistory::new_with_path(path.clone());
                    let _ = h.push("first");
                    let _ = h.push("second");
                    let _ = h.push("third");
                }
                assert!(path.exists());
                assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\nthird\n");
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn returns_true_when_added() {
                let path = temp_path("push_true");
                let mut h = PersistHistory::new_with_path(path.clone());
                assert_eq!(h.push("cmd"), Ok(true));
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn returns_false_on_consecutive_duplicate() {
                let path = temp_path("push_false");
                let mut h = PersistHistory::new_with_path(path.clone());
                let _ = h.push("cmd");
                assert_eq!(h.push("cmd"), Ok(false));
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn deduplicates_consecutive_in_memory() {
                let path = temp_path("dedup_mem");
                let mut h = PersistHistory::new_with_path(path.clone());
                let _ = h.push("cmd");
                let _ = h.push("cmd");
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["cmd"]);
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn deduplicates_last_entry_across_restarts() {
                let path = temp_path("dedup_restart");
                {
                    let mut h = PersistHistory::new_with_path(path.clone());
                    let _ = h.push("first");
                    let _ = h.push("second");
                }
                {
                    let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                    assert_eq!(h.push("second"), Ok(false));
                    assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
                }
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn appends_new_entries_across_sessions() {
                let path = temp_path("append");
                {
                    let mut h = PersistHistory::new_with_path(path.clone());
                    let _ = h.push("first");
                }
                {
                    let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                    let _ = h.push("second");
                }
                assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond\n");
                let _ = fs::remove_file(&path);
            }

            #[test]
            #[cfg(unix)]
            fn returns_err_on_readonly_file() {
                use std::os::unix::fs::PermissionsExt;
                let path = temp_path("readonly");
                fs::write(&path, "").unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
                let mut h = PersistHistory::new_with_path(path.clone());
                assert!(h.push("cmd").is_err());
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
                let _ = fs::remove_file(&path);
            }
        }

        mod load {
            use super::*;

            #[test]
            fn loads_existing_entries() {
                let path = temp_path("load");
                fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();
                let h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["alpha", "beta", "gamma"]);
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn loads_empty_file_as_empty() {
                let path = temp_path("load_empty");
                fs::write(&path, "").unwrap();
                let h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert!(h.is_empty());
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn skips_blank_lines() {
                let path = temp_path("blank_lines");
                fs::write(&path, "a\n\nb\n\n").unwrap();
                let h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert_eq!(h.iter().collect::<Vec<_>>(), vec!["a", "b"]);
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn deduplicates_last_entry_from_file_on_push() {
                let path = temp_path("dup_in_file");
                fs::write(&path, "alpha\nalpha\nbeta\n").unwrap();
                let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert_eq!(h.push("beta"), Ok(false));
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn new_creates_file_if_missing() {
                let path = temp_path("create_on_load");
                assert!(!path.exists());
                let _ = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert!(path.exists());
                let _ = fs::remove_file(&path);
            }
        }

        mod navigation {
            use super::*;

            #[test]
            fn navigate_up_and_down_full_cycle() {
                let path = temp_path("nav_full");
                fs::write(&path, "cmd_a\ncmd_b\ncmd_c\n").unwrap();
                let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert!(matches!(
                    h.navigate_up(""),
                    HistoryNavResult::Entry("cmd_c")
                ));
                assert!(matches!(
                    h.navigate_up(""),
                    HistoryNavResult::Entry("cmd_b")
                ));
                assert!(matches!(
                    h.navigate_up(""),
                    HistoryNavResult::Entry("cmd_a")
                ));
                // Clamp at oldest
                assert!(matches!(
                    h.navigate_up(""),
                    HistoryNavResult::Entry("cmd_a")
                ));
                // Navigate back
                assert!(matches!(
                    h.navigate_down(),
                    HistoryNavResult::Entry("cmd_b")
                ));
                assert!(matches!(
                    h.navigate_down(),
                    HistoryNavResult::Entry("cmd_c")
                ));
                assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
                assert_eq!(h.backup(), "");
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn navigate_up_on_empty_returns_empty() {
                let path = temp_path("nav_empty");
                fs::write(&path, "").unwrap();
                let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                assert!(matches!(h.navigate_up(""), HistoryNavResult::Empty));
                let _ = fs::remove_file(&path);
            }

            #[test]
            fn reset_index_restarts_navigation() {
                let path = temp_path("nav_reset");
                fs::write(&path, "x\ny\n").unwrap();
                let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
                h.navigate_up("first");
                h.reset_index();
                h.navigate_up("second");
                assert_eq!(h.backup(), "second");
                let _ = fs::remove_file(&path);
            }
        }
    }

    // ── AnyHistory ────────────────────────────────────────────────────────────

    mod any_history {
        use super::*;

        fn base_with_entries(cmds: &[&str]) -> AnyHistory {
            let mut h = AnyHistory::Base(History::new());
            for cmd in cmds {
                let _ = h.push(cmd);
            }
            h
        }

        fn persist_with_entries(path: PathBuf, cmds: &[&str]) -> AnyHistory {
            let mut h = AnyHistory::Persist(PersistHistory::new_with_path(path));
            for cmd in cmds {
                let _ = h.push(cmd);
            }
            h
        }

        #[test]
        fn base_is_empty_on_new() {
            assert!(AnyHistory::Base(History::new()).is_empty());
        }

        #[test]
        fn base_is_empty_false_after_push() {
            let mut h = AnyHistory::Base(History::new());
            let _ = h.push("cmd");
            assert!(!h.is_empty());
        }

        #[test]
        fn base_navigate_up_and_down() {
            let mut h = base_with_entries(&["a", "b", "c"]);
            assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("c")));
            assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("b")));
            assert!(matches!(h.navigate_down(), HistoryNavResult::Entry("c")));
            assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
        }

        #[test]
        fn base_backup_and_reset() {
            let mut h = base_with_entries(&["a", "b"]);
            h.navigate_up("draft");
            assert_eq!(h.backup(), "draft");
            h.reset_index();
            assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
        }

        #[test]
        fn persist_is_empty_on_new() {
            let path = temp_path("any_empty");
            assert!(AnyHistory::Persist(PersistHistory::new_with_path(path.clone())).is_empty());
            let _ = fs::remove_file(&path);
        }

        #[test]
        fn persist_navigate_up_and_down() {
            let path = temp_path("any_nav");
            let mut h = persist_with_entries(path.clone(), &["x", "y", "z"]);
            assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("z")));
            assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
            let _ = fs::remove_file(&path);
        }

        #[test]
        fn persist_backup_value() {
            let path = temp_path("any_backup");
            let mut h = persist_with_entries(path.clone(), &["a"]);
            h.navigate_up("my_backup");
            assert_eq!(h.backup(), "my_backup");
            let _ = fs::remove_file(&path);
        }
    }
}
