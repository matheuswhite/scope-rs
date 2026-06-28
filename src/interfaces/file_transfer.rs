use crate::infra::ByteFormat;
use crate::infra::logger::{LogLevel, Logger};
use crate::{error, info, success, warning};
use std::path::Path;
use std::time::Instant;

/// Bytes offered to the wire per task-loop iteration during a transfer. The
/// write is non-blocking and may accept fewer (RTT ring-buffer backpressure, or
/// a full serial OS buffer); [`FileTransfer::advance`] takes the actual count,
/// so a partial — or zero — write simply retries the remainder next iteration
/// without blocking the loop.
pub const CHUNK_SIZE: usize = 1024;

/// Upper bound on a file accepted by [`FileTransfer::load`]. The file is read
/// fully into memory, so this guards against an accidental path to a huge or
/// device file ballooning RSS. Serial/RTT payloads are small in practice, so
/// the limit is generous rather than tight.
pub const MAX_FILE_SIZE: u64 = 256 * 1024 * 1024;

/// An in-progress binary file transfer to an interface. It is driven a chunk at
/// a time from the interface task loop so receiving stays responsive, and the
/// bytes are written straight to the wire — they never reach the `tx` display
/// bus, so the raw content can't pollute the history. Only progress is
/// surfaced, via the logger.
pub struct FileTransfer {
    /// Bare file name, for progress messages.
    name: String,
    data: Vec<u8>,
    /// Bytes already written to the wire.
    sent: usize,
    /// Last progress percentage already reported, to throttle log spam.
    last_reported: u8,
    started: Instant,
}

impl FileTransfer {
    /// Read `path` and arm a transfer, logging the kickoff. Returns `None`
    /// (after logging the reason) when the file can't be read or is empty. The
    /// bytes are read raw, so any file is streamed verbatim as binary.
    pub fn load(path: &str, logger: &Logger) -> Option<Self> {
        // Cap the size before slurping the whole file into memory. A failed
        // metadata lookup falls through to `read`, which surfaces the real error.
        if let Ok(meta) = std::fs::metadata(path)
            && meta.len() > MAX_FILE_SIZE
        {
            error!(
                logger,
                "Cannot send \"{}\": file is too large ({}, max {})",
                path,
                ByteFormat::from(meta.len() as u128).0,
                ByteFormat::from(MAX_FILE_SIZE as u128).0
            );
            return None;
        }

        let data = match std::fs::read(path) {
            Ok(data) => data,
            Err(err) => {
                error!(logger, "Cannot read \"{}\": {}", path, err);
                return None;
            }
        };
        if data.is_empty() {
            warning!(logger, "File \"{}\" is empty; nothing to send", path);
            return None;
        }

        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        info!(
            logger,
            "Sending \"{}\" ({})...",
            name,
            ByteFormat::from(data.len() as u128).0
        );

        Some(Self {
            name,
            data,
            sent: 0,
            last_reported: 0,
            started: Instant::now(),
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The next slice to write, up to `max` bytes. Never empty while the
    /// transfer is unfinished.
    pub fn next_chunk(&self, max: usize) -> &[u8] {
        let end = (self.sent + max).min(self.data.len());
        &self.data[self.sent..end]
    }

    /// Record `written` bytes as sent, reporting progress at each new 10% step
    /// and a final success line on completion. Returns `true` once the whole
    /// file has been sent — the caller should then drop the transfer. A
    /// `written` of 0 (e.g. a full channel buffer) is a no-op that keeps the
    /// transfer alive for the next iteration.
    pub fn advance(&mut self, written: usize, logger: &Logger) -> bool {
        self.sent = (self.sent + written).min(self.data.len());
        let total = self.data.len();
        // Use 64-bit math: `sent * 100` would overflow a 32-bit usize for files
        // larger than ~42 MB on 32-bit targets.
        let percent = (self.sent as u64 * 100 / total as u64) as u8;

        if self.sent >= total {
            success!(
                logger,
                "\"{}\" sent ({}) in {:.1}s",
                self.name,
                ByteFormat::from(total as u128).0,
                self.started.elapsed().as_secs_f32()
            );
            return true;
        }

        if percent / 10 > self.last_reported / 10 {
            self.last_reported = percent;
            info!(logger, "Sending \"{}\": {}%", self.name, percent);
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::logger::LogMessage;
    use std::sync::mpsc::Receiver;

    fn test_logger() -> (Logger, Receiver<LogMessage>) {
        Logger::new("test".to_string())
    }

    fn drain(rx: &Receiver<LogMessage>) -> Vec<(LogLevel, String)> {
        let mut out = vec![];
        while let Ok(msg) = rx.try_recv() {
            out.push((msg.level, msg.message));
        }
        out
    }

    fn temp_file(suffix: &str, content: &[u8]) -> String {
        let path = std::env::temp_dir()
            .join(format!("scope_ft_{}_{}", std::process::id(), suffix))
            .to_str()
            .unwrap()
            .to_string();
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn load_reads_file_and_announces_it() {
        let (logger, rx) = test_logger();
        let path = temp_file("load.bin", b"abcdef");

        let t = FileTransfer::load(&path, &logger).expect("loads existing file");
        assert_eq!(
            t.name(),
            format!("scope_ft_{}_load.bin", std::process::id())
        );
        assert_eq!(t.data, b"abcdef");

        let logs = drain(&rx);
        assert_eq!(logs.len(), 1);
        assert!(matches!(logs[0].0, LogLevel::Info));
        assert!(logs[0].1.contains("Sending"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_rejects_empty_file() {
        let (logger, rx) = test_logger();
        let path = temp_file("empty.bin", b"");

        assert!(FileTransfer::load(&path, &logger).is_none());
        assert!(matches!(drain(&rx)[0].0, LogLevel::Warning));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_reports_missing_file() {
        let (logger, rx) = test_logger();
        let missing = "/nonexistent/scope/does-not-exist.bin";

        assert!(FileTransfer::load(missing, &logger).is_none());
        assert!(matches!(drain(&rx)[0].0, LogLevel::Error));
    }

    #[test]
    fn next_chunk_is_bounded_and_advances() {
        let (logger, _rx) = test_logger();
        let path = temp_file("chunk.bin", &[0u8; CHUNK_SIZE * 2 + 5]);
        let mut t = FileTransfer::load(&path, &logger).unwrap();

        assert_eq!(t.next_chunk(CHUNK_SIZE).len(), CHUNK_SIZE);
        t.advance(CHUNK_SIZE, &logger);
        assert_eq!(t.next_chunk(CHUNK_SIZE).len(), CHUNK_SIZE);
        t.advance(CHUNK_SIZE, &logger);
        // Only the 5-byte tail remains.
        assert_eq!(t.next_chunk(CHUNK_SIZE).len(), 5);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn advance_completes_and_reports_success() {
        let (logger, rx) = test_logger();
        let path = temp_file("done.bin", b"hello");
        let mut t = FileTransfer::load(&path, &logger).unwrap();
        let _ = drain(&rx); // discard the kickoff line

        let chunk = t.next_chunk(CHUNK_SIZE).len();
        assert!(t.advance(chunk, &logger), "small file done in one step");

        let logs = drain(&rx);
        assert_eq!(logs.len(), 1);
        assert!(matches!(logs[0].0, LogLevel::Success));
        assert!(logs[0].1.contains("sent"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn advance_throttles_progress_to_ten_percent_steps() {
        let (logger, rx) = test_logger();
        // 100 bytes so each byte is exactly 1%.
        let path = temp_file("progress.bin", &[7u8; 100]);
        let mut t = FileTransfer::load(&path, &logger).unwrap();
        let _ = drain(&rx);

        // Advance 1% at a time up to 95%: should emit one line per 10% step.
        for _ in 0..95 {
            t.advance(1, &logger);
        }
        let progress = drain(&rx);
        // Steps crossed: 10,20,30,40,50,60,70,80,90 -> 9 lines, all info.
        assert_eq!(progress.len(), 9);
        assert!(
            progress
                .iter()
                .all(|(lvl, _)| matches!(lvl, LogLevel::Info))
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn advance_handles_partial_writes() {
        let (logger, rx) = test_logger();
        let path = temp_file("partial.bin", b"abcd");
        let mut t = FileTransfer::load(&path, &logger).unwrap();
        let _ = drain(&rx);

        // A zero-byte write (full channel buffer) keeps the transfer alive.
        assert!(!t.advance(0, &logger));
        // Dribble the 4 bytes out one at a time.
        assert!(!t.advance(1, &logger));
        assert!(!t.advance(2, &logger));
        assert!(t.advance(1, &logger), "final byte completes the transfer");

        let _ = std::fs::remove_file(&path);
    }
}
