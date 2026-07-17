//! Headless display task.
//!
//! This is the drop-in replacement for [`GraphicsTask`](super::graphics_task)
//! when `--headless` is set. It occupies the same "graphics" slot in the actor
//! system — same `GraphicsCommand` channel (so the Inputs task's `Exit`
//! broadcast reaches it) and the same `tx`/`rx`/logger consumers — but instead
//! of a ratatui TUI it is a plain stdout bridge:
//!
//! - received bytes are written straight to stdout, verbatim (no timestamp, no
//!   scrollback);
//! - log messages are printed inline with a background color (the same
//!   `LogLevel`→color mapping the TUI uses, emitted as raw ANSI SGR);
//! - transmitted bytes are drained and discarded (no local echo);
//! - while the Inputs task is in `Ctrl+K` command mode (`InputsShared::raw`
//!   is `false`), incoming output is held back and a `> ` prompt mirrors the
//!   command being typed, blinking between black-on-yellow and plain yellow
//!   text; on return to raw the held output is flushed.
//!
//! It owns the terminal's raw mode (no alternate screen / mouse / bracketed
//! paste) and is the sole writer of stdout, so there is no rendering race with
//! the Inputs task (which only ever produces bytes onto the `tx` bus).

use crate::graphics::graphics_task::GraphicsCommand;
use crate::infra::{
    logger::{LogLevel, LogMessage},
    messages::TimedBytes,
    mpmc::Consumer,
    task::{Shared, Task},
    timer::Timer,
};
use crate::inputs::inputs_task::InputsShared;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, Write};
use std::sync::{
    Arc, RwLock,
    mpsc::{Receiver, Sender},
};
use std::thread::{sleep, yield_now};
use std::time::Duration;

pub type HeadlessTask = Task<(), GraphicsCommand>;

pub struct HeadlessConnections {
    logger_receiver: Receiver<LogMessage>,
    system_log_level: LogLevel,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    inputs_shared: Shared<InputsShared>,
    latency: u64,
}

impl HeadlessConnections {
    pub fn new(
        logger_receiver: Receiver<LogMessage>,
        tx: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
        inputs_shared: Shared<InputsShared>,
        latency: u64,
    ) -> Self {
        Self {
            logger_receiver,
            // Matches the TUI default (`GraphicsConnections::new`): show every
            // level until `!log system <level>` narrows it.
            system_log_level: LogLevel::Debug,
            tx,
            rx,
            inputs_shared,
            latency,
        }
    }
}

pub fn spawn_headless_task(
    connections: HeadlessConnections,
    cmd_sender: Sender<GraphicsCommand>,
    cmd_receiver: Receiver<GraphicsCommand>,
) -> HeadlessTask {
    Task::new((), connections, run_headless, cmd_sender, cmd_receiver)
}

/// The blink half-period for the command-mode prompt.
const BLINK_PERIOD: Duration = Duration::from_millis(500);

fn run_headless(
    _shared: Arc<RwLock<()>>,
    mut private: HeadlessConnections,
    cmd_receiver: Receiver<GraphicsCommand>,
) {
    enable_raw_mode().expect("Cannot enable terminal raw mode");
    let stdout = io::stdout();
    let mut out = stdout.lock();

    let mut blink_timer = Timer::new(BLINK_PERIOD);
    let mut prev_raw = true; // headless starts in raw passthrough
    let mut held: Vec<u8> = Vec::new();
    let mut inverted = true;
    let mut last_cmd = String::new();
    let mut prompt_dirty = false;

    'draw_loop: loop {
        // Control commands. Only Exit and log-level matter here; the rest are
        // TUI-only state the headless bridge has no concept of.
        while let Ok(cmd) = cmd_receiver.try_recv() {
            match cmd {
                GraphicsCommand::Exit => break 'draw_loop,
                GraphicsCommand::SetLogLevel(level) => private.system_log_level = level,
                _ => {}
            }
        }

        // No local echo: drain the TX bus so its queue can't grow unbounded.
        while private.tx.try_recv().is_ok() {}

        let (raw, command_line, cursor) = {
            let sr = private
                .inputs_shared
                .read()
                .expect("Cannot get input lock for read");
            (sr.raw, sr.command_line.clone(), sr.cursor)
        };

        if raw {
            // On return from command mode, wipe the prompt line and flush the
            // output that was held back while it was up.
            if !prev_raw {
                let _ = write!(out, "\r\x1b[2K");
                if !held.is_empty() {
                    let _ = out.write_all(&held);
                    held.clear();
                }
                let _ = out.flush();
            }

            let mut wrote = false;
            while let Ok(msg) = private.rx.try_recv() {
                let _ = out.write_all(&msg.message);
                wrote = true;
            }
            while let Ok(log) = private.logger_receiver.try_recv() {
                if level_visible(log.level, private.system_log_level) {
                    write_log(&mut out, &log);
                    wrote = true;
                }
            }
            if wrote {
                let _ = out.flush();
            }
        } else {
            // Command mode: hold incoming output and show the blinking prompt.
            if prev_raw {
                inverted = true;
                blink_timer.start();
                prompt_dirty = true;
            }

            while let Ok(msg) = private.rx.try_recv() {
                held.extend_from_slice(&msg.message);
            }
            while let Ok(log) = private.logger_receiver.try_recv() {
                if level_visible(log.level, private.system_log_level) {
                    hold_log(&mut held, &log);
                }
            }

            if blink_timer.tick() {
                inverted = !inverted;
                blink_timer.start();
                prompt_dirty = true;
            }
            if command_line != last_cmd {
                prompt_dirty = true;
            }

            if prompt_dirty {
                // `> ` occupies columns 1-2 (1-based), so the first typed
                // character sits at column 3; place the cursor at `3 + cursor`.
                // The prompt never blanks — it blinks between inverted (black on
                // yellow, same SGR as the `Warning` log) and plain yellow text
                // on the default background, so it stays readable while drawing
                // the eye and standing out from raw device output.
                let sgr = if inverted { "\x1b[30;43m" } else { "\x1b[33m" };
                let _ = write!(
                    out,
                    "\r\x1b[2K{}> {}\x1b[0m\r\x1b[{}G",
                    sgr,
                    command_line,
                    3 + cursor
                );
                let _ = out.flush();
                last_cmd = command_line;
                prompt_dirty = false;
            }
        }

        prev_raw = raw;

        if private.latency > 0 {
            sleep(Duration::from_micros(private.latency));
        } else {
            yield_now();
        }
    }

    // Teardown. If we were mid-prompt (a `Ctrl+K` `Ctrl+Q` quit), clear it and
    // flush anything held; then leave the cursor on a fresh line and restore
    // cooked mode.
    if !prev_raw {
        let _ = write!(out, "\r\x1b[2K");
        if !held.is_empty() {
            let _ = out.write_all(&held);
        }
    }
    let _ = write!(out, "\r\n");
    let _ = out.flush();
    disable_raw_mode().expect("Cannot disable terminal raw mode");
}

/// Whether a log at `level` clears the current threshold, mirroring
/// `Screen::draw`'s discriminant-order comparison (Error=0 … Debug=4).
fn level_visible(level: LogLevel, threshold: LogLevel) -> bool {
    (level as u32) <= (threshold as u32)
}

/// The ANSI SGR prefix reproducing the TUI's `LogLevel`→background mapping
/// (`screen.rs::log_line` + `Palette::fg`), reset with `\x1b[0m`.
fn log_sgr(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "\x1b[97;41m",    // white on red
        LogLevel::Warning => "\x1b[30;43m",  // black on yellow
        LogLevel::Success => "\x1b[30;102m", // black on bright green
        LogLevel::Info => "\x1b[30;46m",     // black on cyan
        LogLevel::Debug => "\x1b[30;100m",   // black on bright black
    }
}

fn write_log(out: &mut impl Write, log: &LogMessage) {
    let _ = write!(out, "{}", log_sgr(log.level));
    let _ = out.write_all(log.message.as_bytes());
    let _ = write!(out, "\x1b[0m\r\n");
}

fn hold_log(held: &mut Vec<u8>, log: &LogMessage) {
    held.extend_from_slice(log_sgr(log.level).as_bytes());
    held.extend_from_slice(log.message.as_bytes());
    held.extend_from_slice(b"\x1b[0m\r\n");
}
