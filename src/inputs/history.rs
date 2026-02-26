use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

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
#[derive(Default)]
pub struct History {
    entries: Vec<String>,
    index: Option<usize>,
    /// Snapshot of the input before the user started navigating.
    backup: String,
}

impl History {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            index: None,
            backup: String::new(),
        }
    }

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
                self.entries.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.index = Some(idx);
        HistoryNavResult::Entry(&self.entries[idx])
    }

    /// Navigate to a newer entry, or back to the saved backup when past the newest.
    pub fn navigate_down(&mut self) -> HistoryNavResult<'_> {
        if self.entries.is_empty() {
            return HistoryNavResult::Empty;
        }
        match self.index {
            None => HistoryNavResult::Empty,
            Some(i) if i == self.entries.len() - 1 => {
                self.index = None;
                HistoryNavResult::RestoreBackup
            }
            Some(i) => {
                let next = i + 1;
                self.index = Some(next);
                HistoryNavResult::Entry(&self.entries[next])
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

    // ── History::push ────────────────────────────────────────────────────────

    #[test]
    fn push_adds_entry() {
        let mut h = History::new();
        let _ = h.push("hello");
        assert!(!h.is_empty());
        assert_eq!(h.iter().collect::<Vec<_>>(), vec!["hello"]);
    }

    #[test]
    fn push_deduplicates_consecutive() {
        let mut h = History::new();
        let _ = h.push("dup");
        let _ = h.push("dup");
        assert_eq!(h.iter().collect::<Vec<_>>(), vec!["dup"]);
    }

    #[test]
    fn push_allows_non_consecutive_duplicates() {
        let mut h = History::new();
        let _ = h.push("a");
        let _ = h.push("b");
        let _ = h.push("a");
        assert_eq!(h.iter().collect::<Vec<_>>(), vec!["a", "b", "a"]);
    }

    // ── History::is_empty ────────────────────────────────────────────────────

    #[test]
    fn is_empty_on_new() {
        assert!(History::new().is_empty());
    }

    #[test]
    fn is_empty_false_after_push() {
        let mut h = History::new();
        let _ = h.push("cmd");
        assert!(!h.is_empty());
    }

    // ── History::navigate_up ─────────────────────────────────────────────────

    #[test]
    fn navigate_up_on_empty_returns_empty() {
        let mut h = History::new();
        assert!(matches!(h.navigate_up(""), HistoryNavResult::Empty));
    }

    #[test]
    fn navigate_up_returns_last_entry_first() {
        let mut h = History::new_with_entries(&["a", "b", "c"]);
        assert!(matches!(
            h.navigate_up("current"),
            HistoryNavResult::Entry("c")
        ));
    }

    #[test]
    fn navigate_up_saves_backup_on_first_call() {
        let mut h = History::new_with_entries(&["a"]);
        h.navigate_up("my_draft");
        assert_eq!(h.backup(), "my_draft");
    }

    #[test]
    fn navigate_up_traverses_to_oldest() {
        let mut h = History::new_with_entries(&["a", "b", "c"]);
        h.navigate_up("");
        h.navigate_up("");
        assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("a")));
    }

    #[test]
    fn navigate_up_clamps_at_oldest() {
        let mut h = History::new_with_entries(&["a", "b"]);
        h.navigate_up("");
        h.navigate_up("");
        // Extra up — should stay at "a"
        assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("a")));
    }

    // ── History::navigate_down ───────────────────────────────────────────────

    #[test]
    fn navigate_down_without_navigation_returns_empty() {
        let mut h = History::new_with_entries(&["a", "b"]);
        assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
    }

    #[test]
    fn navigate_down_returns_newer_entry() {
        let mut h = History::new_with_entries(&["a", "b", "c"]);
        h.navigate_up("");
        h.navigate_up("");
        // Now at "b"; down should return "c"
        assert!(matches!(h.navigate_down(), HistoryNavResult::Entry("c")));
    }

    #[test]
    fn navigate_down_past_newest_restores_backup() {
        let mut h = History::new_with_entries(&["a", "b"]);
        h.navigate_up("draft");
        // At "b" (newest); down should restore
        assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
        assert_eq!(h.backup(), "draft");
    }

    #[test]
    fn navigate_down_on_empty_returns_empty() {
        let mut h = History::new();
        assert!(matches!(h.navigate_down(), HistoryNavResult::Empty));
    }

    // ── History::reset_index ─────────────────────────────────────────────────

    #[test]
    fn reset_index_allows_fresh_navigation() {
        let mut h = History::new_with_entries(&["a", "b"]);
        h.navigate_up("first_draft");
        h.reset_index();
        // After reset, up should start from newest again and save new backup
        h.navigate_up("second_draft");
        assert_eq!(h.backup(), "second_draft");
        assert!(matches!(h.navigate_up(""), HistoryNavResult::Entry("a")));
    }

    // ── History iterators ────────────────────────────────────────────────────

    #[test]
    fn ref_iterator_yields_all_entries_in_order() {
        let h = History::new_with_entries(&["x", "y", "z"]);
        let collected = h.iter().collect::<Vec<_>>();
        assert_eq!(collected, vec!["x", "y", "z"]);
    }

    // ── PersistHistory file persistence ──────────────────────────────────────

    /// Verifies that push() creates the history file and writes entries to it.
    /// Uses std::env::temp_dir() so the test works on Linux, macOS, and Windows.
    #[test]
    fn file_is_created_and_entries_are_persisted() {
        let path = std::env::temp_dir().join(format!(
            "scope_history_test_{}.tmp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));

        let _ = fs::remove_file(&path);

        {
            let mut h = PersistHistory::new_with_path(path.clone());
            let _ = h.push("first");
            let _ = h.push("second");
            let _ = h.push("third");
        }

        assert!(path.exists(), "history file was not created");

        let loaded: Vec<String> = {
            let f = fs::File::open(&path).expect("cannot open history file");
            BufReader::new(f)
                .lines()
                .map_while(Result::ok)
                .filter(|l| !l.is_empty())
                .collect()
        };
        assert_eq!(loaded, vec!["first", "second", "third"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persist_history_loads_existing_entries_from_file() {
        let path = temp_path("load");
        fs::write(&path, "alpha\nbeta\ngamma\n").unwrap();

        let h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
        let entries = h.iter().collect::<Vec<_>>();
        assert_eq!(entries, vec!["alpha", "beta", "gamma"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persist_history_loads_empty_file_as_empty() {
        let path = temp_path("load_empty");
        fs::write(&path, "").unwrap();

        let h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
        assert!(h.is_empty());

        let _ = fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn persist_history_push_returns_err_on_readonly_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("readonly");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();

        let mut h = PersistHistory::new_with_path(path.clone());
        let result = h.push("cmd");
        assert!(
            result.is_err(),
            "push should fail when the file is read-only"
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persist_history_deduplicates_last_entry_across_restarts() {
        let path = temp_path("dedup_restart");

        // Session 1: push two distinct entries
        {
            let mut h = PersistHistory::new_with_path(path.clone());
            let _ = h.push("first");
            let _ = h.push("second");
        }

        // Session 2: reload, then push the same last entry — must be deduped
        {
            let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
            let added = h.push("second").unwrap();
            assert!(
                !added,
                "duplicate of last loaded entry should be deduplicated"
            );

            // File must not have gained an extra line
            let content = fs::read_to_string(&path).unwrap();
            assert_eq!(content, "first\nsecond\n");
        }

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persist_history_handles_consecutive_duplicates_in_file() {
        let path = temp_path("dup_in_file");
        // Simulate a file with consecutive duplicates (e.g. from manual editing)
        fs::write(&path, "alpha\nalpha\nbeta\n").unwrap();

        let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();
        // "beta" is the last line in the file; pushing it again must be deduped
        let added = h.push("beta").unwrap();
        assert!(
            !added,
            "should deduplicate against the last entry in the loaded file"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn persist_history_navigation_after_loading_from_disk() {
        let path = temp_path("nav_after_load");
        fs::write(&path, "cmd_a\ncmd_b\ncmd_c\n").unwrap();

        let mut h = PersistHistory::new_loaded_from_path(path.clone()).unwrap();

        // Navigate up: newest first
        assert!(matches!(
            h.navigate_up("draft"),
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

        // Navigate back down toward newest
        assert!(matches!(
            h.navigate_down(),
            HistoryNavResult::Entry("cmd_b")
        ));
        assert!(matches!(
            h.navigate_down(),
            HistoryNavResult::Entry("cmd_c")
        ));
        // Past newest restores the backup captured on the first navigate_up
        assert!(matches!(h.navigate_down(), HistoryNavResult::RestoreBackup));
        assert_eq!(h.backup(), "draft");

        let _ = fs::remove_file(&path);
    }
}
