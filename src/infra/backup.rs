use crate::error;
use crate::infra::logger::{LogLevel, Logger};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender, channel},
    thread::{self, JoinHandle},
    time::SystemTime,
};

/// How many `.bkp` files to keep in the backup directory. Once a new session's
/// backup is created and the count exceeds this, the oldest files are removed so
/// the directory doesn't grow without bound.
const MAX_BACKUP_FILES: usize = 10;

/// Directory the crash-recovery backups live in: the user's config directory
/// under `scope/backup` (e.g. `~/.config/scope/backup` on Linux). Keeping the
/// `.bkp` files here instead of the working directory avoids littering whatever
/// folder the user launched `scope` from. Falls back to a relative directory if
/// the platform config directory can't be resolved.
fn backup_dir() -> PathBuf {
    dirs::config_dir()
        .map(|dir| dir.join("scope").join("backup"))
        .unwrap_or_else(|| PathBuf::from("scope").join("backup"))
}

/// Resolve a backup file name (e.g. `session.txt.bkp`) to its full path inside
/// the backup directory. Pure — creating the directory happens lazily on the
/// worker thread right before the file is opened.
pub fn backup_path(file_name: &str) -> String {
    // Pin to the final path component. Session names are already sanitized
    // upstream (`session::sanitize_name` rejects separators, `..`, etc.), but
    // `PathBuf::join` would let an absolute or directory-bearing `file_name`
    // escape the backup directory entirely, so keep the helper safe on its own.
    let name = Path::new(file_name)
        .file_name()
        .unwrap_or_else(|| file_name.as_ref());
    backup_dir().join(name).to_string_lossy().into_owned()
}

/// Commands handed to the background backup-writer thread.
enum BackupCommand {
    Append(Vec<String>),
    Rename(String),
}

/// Mirrors every line the session accumulates into a `<session>.txt.bkp` file
/// as it arrives, so an accidental close or a crash doesn't lose the history
/// that hasn't been explicitly saved yet (the `TypeWriter` only hits disk on an
/// explicit save). The file lives in the user's config directory (see
/// [`backup_dir`]) rather than the working directory, and at most
/// [`MAX_BACKUP_FILES`] are kept. All disk writes happen on a dedicated thread,
/// so the graphics loop hands off the already-serialized lines and never blocks
/// on I/O.
pub struct Backup {
    // `Option` only so `Drop` can take the sender and join the worker.
    sender: Option<Sender<BackupCommand>>,
    handle: Option<JoinHandle<()>>,
}

impl Backup {
    pub fn new(filename: String, logger: Logger) -> Self {
        // The channel is intentionally unbounded: dropping batches would punch
        // gaps into the crash-recovery file, and a blocking bounded send would
        // reintroduce the draw-loop stall the worker thread exists to avoid.
        // Backlog can only grow while disk writes lag arrival, which sequential
        // text appends don't in practice; failed writes are dropped (not
        // re-queued), and the session's full unsaved history already lives
        // unbounded in the TypeWriter, so this isn't the binding memory cap.
        let (sender, receiver) = channel::<BackupCommand>();
        let handle = thread::Builder::new()
            .name("backup-writer".to_string())
            .spawn(move || Self::worker(filename, receiver, logger))
            .expect("Cannot spawn backup writer thread");

        Self {
            sender: Some(sender),
            handle: Some(handle),
        }
    }

    /// Queue already-serialized lines to be appended to the backup file. The
    /// caller shares the serialization with the typewriter/recorder; this just
    /// hands the owned strings to the writer thread without blocking.
    pub fn append(&self, contents: Vec<String>) {
        if contents.is_empty() {
            return;
        }

        if let Some(sender) = &self.sender {
            let _ = sender.send(BackupCommand::Append(contents));
        }
    }

    /// Point the backup at a new file name (following a session `!rename`). The
    /// request is always forwarded and serialized with pending writes on the
    /// worker thread, which owns the authoritative file name — so it can no-op a
    /// same-name request without a stale local copy blocking a later retry.
    pub fn rename(&self, filename: String) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(BackupCommand::Rename(filename));
        }
    }

    fn worker(mut filename: String, receiver: Receiver<BackupCommand>, logger: Logger) {
        // Opened lazily on the first write so an idle session leaves no stray
        // `.bkp` behind, mirroring how the `TypeWriter` creates its file.
        let mut file: Option<File> = None;
        // Report an I/O failure only on the transition into the error state so a
        // persistently unwritable backup doesn't flood the log every batch.
        let mut reported_error = false;

        while let Ok(cmd) = receiver.recv() {
            match cmd {
                BackupCommand::Rename(new_name) => {
                    if new_name == filename {
                        continue;
                    }
                    // Drop the handle first so the move targets a closed file;
                    // the next append reopens under the (possibly new) name.
                    file = None;
                    if Path::new(&new_name).exists() {
                        // Never clobber an existing backup — it may be an
                        // unrecovered crash file from another session. Keep
                        // writing under the current name instead (mirrors the
                        // TypeWriter's refusal to overwrite on rename).
                        error!(
                            logger,
                            "Cannot move backup to \"{}\": already exists", new_name
                        );
                    } else if Path::new(&filename).exists() {
                        match std::fs::rename(&filename, &new_name) {
                            Ok(()) => filename = new_name,
                            Err(err) => {
                                error!(logger, "Cannot rename backup to \"{}\": {}", new_name, err)
                            }
                        }
                    } else {
                        // Nothing written yet; just adopt the new name.
                        filename = new_name;
                    }
                }
                BackupCommand::Append(contents) => {
                    if file.is_none() {
                        // Create the backup directory lazily so an idle session
                        // leaves nothing behind, and only pay the syscall when we
                        // actually have something to write.
                        if let Some(parent) = Path::new(&filename).parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match OpenOptions::new().append(true).create(true).open(&filename) {
                            Ok(f) => {
                                file = Some(f);
                                reported_error = false;
                                // A fresh backup just landed in the directory;
                                // evict the oldest ones if we're over the cap.
                                prune_old_backups(Path::new(&filename), MAX_BACKUP_FILES, &logger);
                            }
                            Err(err) => {
                                if !reported_error {
                                    error!(
                                        logger,
                                        "Cannot open backup file \"{}\": {}", filename, err
                                    );
                                    reported_error = true;
                                }
                                continue;
                            }
                        }
                    }

                    let Some(f) = file.as_mut() else {
                        continue;
                    };

                    for content in contents {
                        // Match the line-ending normalization the typewriter and
                        // recorder apply so the `.bkp` is byte-identical to the
                        // saved `.txt`.
                        let content = if !content.ends_with('\n') {
                            content + "\r\n"
                        } else {
                            content
                        };

                        if let Err(err) = f.write_all(content.as_bytes()) {
                            if !reported_error {
                                error!(
                                    logger,
                                    "Cannot write to backup file \"{}\": {}", filename, err
                                );
                                reported_error = true;
                            }
                            // Force a reopen on the next batch in case the handle
                            // went bad (e.g. the file was removed underneath us).
                            file = None;
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Keep at most `keep` `.bkp` files in `current`'s directory, deleting the
/// oldest (by modification time) beyond that. `current` is the file being
/// actively written; it always counts toward the cap and is never removed even
/// if the clock makes it look old.
fn prune_old_backups(current: &Path, keep: usize, logger: &Logger) {
    let Some(dir) = current.parent() else {
        return;
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut others: Vec<(PathBuf, SystemTime)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.as_path() == current || !path.extension().is_some_and(|ext| ext == "bkp") {
                return None;
            }
            // One stat, straight off the directory entry. Skip anything that
            // isn't a regular file so we never try to `remove_file` a directory
            // named `*.bkp`; an unreadable mtime falls back to the epoch so the
            // entry still counts toward the cap and stays a prune candidate
            // rather than silently dodging it.
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            Some((path, modified))
        })
        .collect();

    // `current` occupies one slot, so at most `keep - 1` other files may remain.
    let max_others = keep.saturating_sub(1);
    if others.len() <= max_others {
        return;
    }

    others.sort_by_key(|(_, modified)| *modified); // oldest first
    let remove_count = others.len() - max_others;
    for (path, _) in others.into_iter().take(remove_count) {
        if let Err(err) = std::fs::remove_file(&path) {
            error!(
                logger,
                "Cannot remove old backup \"{}\": {}",
                path.display(),
                err
            );
        }
    }
}

impl Drop for Backup {
    fn drop(&mut self) {
        // Closing the channel makes the worker drain its remaining writes and
        // exit; joining it guarantees a clean shutdown flushes pending lines.
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Each caller gets its own directory so the worker's `prune_old_backups`
    // scan stays contained to that test's files instead of the shared OS temp
    // root (where it could touch unrelated `.bkp` files).
    fn temp_path(suffix: &str) -> String {
        let dir = std::env::temp_dir().join(format!("scope_bkp_{}_{}", std::process::id(), suffix));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(suffix).to_str().unwrap().to_string()
    }

    fn test_logger() -> Logger {
        // `error!` swallows send errors (`let _ = ...`), so a dropped receiver
        // is fine here; the log output is irrelevant to these tests.
        Logger::new("test".to_string()).0
    }

    #[test]
    fn append_writes_lines_with_crlf() {
        let path = temp_path("append.bkp");
        let _ = std::fs::remove_file(&path);

        {
            let backup = Backup::new(path.clone(), test_logger());
            backup.append(vec!["hello".to_string()]);
            backup.append(vec!["world".to_string()]);
            // Dropping joins the worker, flushing every queued write.
        }

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "hello\r\nworld\r\n");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn append_keeps_existing_trailing_newline() {
        let path = temp_path("newline.bkp");
        let _ = std::fs::remove_file(&path);

        {
            let backup = Backup::new(path.clone(), test_logger());
            backup.append(vec!["already\n".to_string()]);
        }

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "already\n");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn rename_moves_the_backup_file() {
        let old = temp_path("rename_old.bkp");
        let new = temp_path("rename_new.bkp");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&new);

        {
            let backup = Backup::new(old.clone(), test_logger());
            backup.append(vec!["before".to_string()]);
            backup.rename(new.clone());
            backup.append(vec!["after".to_string()]);
        }

        assert!(!Path::new(&old).exists(), "old backup should be moved");
        let content = std::fs::read_to_string(&new).unwrap();
        assert_eq!(content, "before\r\nafter\r\n");

        let _ = std::fs::remove_file(&new);
    }

    #[test]
    fn rename_does_not_clobber_existing_destination() {
        let old = temp_path("clobber_old.bkp");
        let dest = temp_path("clobber_dest.bkp");
        let _ = std::fs::remove_file(&old);
        // A pre-existing backup at the destination (e.g. an unrecovered crash
        // file from another session) must survive the rename untouched.
        std::fs::write(&dest, "previous-session").unwrap();

        {
            let backup = Backup::new(old.clone(), test_logger());
            backup.append(vec!["before".to_string()]);
            backup.rename(dest.clone());
            backup.append(vec!["after".to_string()]);
        }

        // Destination kept its original content; the session kept writing to
        // the original file rather than overwriting the destination.
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "previous-session");
        assert_eq!(
            std::fs::read_to_string(&old).unwrap(),
            "before\r\nafter\r\n"
        );

        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn backup_path_lives_under_the_config_dir() {
        let path = backup_path("session.txt.bkp");
        let path = Path::new(&path);
        assert!(path.ends_with(Path::new("scope").join("backup").join("session.txt.bkp")));
    }

    #[test]
    fn backup_path_cannot_escape_the_backup_dir() {
        // An absolute or directory-bearing name must collapse to its final
        // component so it can't write outside the backup directory.
        let base = backup_dir();
        for name in ["../../etc/evil.bkp", "/tmp/evil.bkp", "sub/dir/evil.bkp"] {
            let path = backup_path(name);
            assert_eq!(Path::new(&path), base.join("evil.bkp"));
        }
    }

    #[test]
    fn prune_removes_oldest_beyond_cap() {
        use std::time::{Duration, UNIX_EPOCH};

        let dir = std::env::temp_dir().join(format!("scope_bkp_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 12 backups with strictly increasing modification times (oldest first).
        let mut paths = vec![];
        for i in 0..12 {
            let path = dir.join(format!("session_{i:02}.txt.bkp"));
            let f = File::create(&path).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(1_000 + i))
                .unwrap();
            paths.push(path);
        }
        // An unrelated file must be left untouched.
        let keep_me = dir.join("notes.txt");
        std::fs::write(&keep_me, "x").unwrap();

        // The newest file is the one being actively written.
        let current = paths.last().unwrap().clone();
        prune_old_backups(&current, MAX_BACKUP_FILES, &test_logger());

        // The two oldest .bkp were removed; the rest and the .txt survive.
        assert!(!paths[0].exists(), "oldest backup should be pruned");
        assert!(!paths[1].exists(), "second-oldest backup should be pruned");
        for path in &paths[2..] {
            assert!(path.exists(), "recent backup pruned: {}", path.display());
        }
        assert!(keep_me.exists(), "non-.bkp files must be untouched");
        assert!(current.exists(), "the active backup is never removed");

        let remaining = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "bkp"))
            .count();
        assert_eq!(remaining, MAX_BACKUP_FILES);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_is_a_noop_under_the_cap() {
        let dir = std::env::temp_dir().join(format!("scope_bkp_noprune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut paths = vec![];
        for i in 0..MAX_BACKUP_FILES {
            let path = dir.join(format!("session_{i:02}.txt.bkp"));
            File::create(&path).unwrap();
            paths.push(path);
        }

        prune_old_backups(&paths[0], MAX_BACKUP_FILES, &test_logger());

        for path in &paths {
            assert!(path.exists(), "nothing should be pruned at the cap");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_append_creates_no_file() {
        let path = temp_path("empty.bkp");
        let _ = std::fs::remove_file(&path);

        {
            let backup = Backup::new(path.clone(), test_logger());
            backup.append(vec![]);
        }

        assert!(!Path::new(&path).exists(), "no file for an empty append");
    }
}
