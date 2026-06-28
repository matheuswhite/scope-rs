use crate::error;
use crate::infra::logger::{LogLevel, Logger};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::mpsc::{Receiver, Sender, channel},
    thread::{self, JoinHandle},
};

/// Commands handed to the background backup-writer thread.
enum BackupCommand {
    Append(Vec<String>),
    Rename(String),
}

/// Mirrors every line the session accumulates into a `<session>.txt.bkp` file
/// as it arrives, so an accidental close or a crash doesn't lose the history
/// that hasn't been explicitly saved yet (the `TypeWriter` only hits disk on an
/// explicit save). All disk writes happen on a dedicated thread, so the graphics
/// loop hands off the already-serialized lines and never blocks on I/O.
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
                        match OpenOptions::new().append(true).create(true).open(&filename) {
                            Ok(f) => {
                                file = Some(f);
                                reported_error = false;
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

    fn temp_path(suffix: &str) -> String {
        std::env::temp_dir()
            .join(format!("scope_bkp_{}_{}", std::process::id(), suffix))
            .to_str()
            .unwrap()
            .to_string()
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
