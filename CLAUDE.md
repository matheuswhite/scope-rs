# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`scope` (crate `scope-monitor`) is a cross-platform serial-monitor TUI built with `ratatui` + `crossterm`. It connects to a serial port (or an RTT target), shows received/sent data with timestamps and color, and is extensible via Lua plugins. Edition 2024, MSRV 1.92.0.

## Commands

```bash
cargo build                         # debug build -> target/debug/scope
cargo build --release               # what CI builds on Linux/Windows/macOS

cargo run --bin scope -- list       # list available serial ports
cargo run --bin scope -- serial <PORT> <BAUD>   # e.g. serial /dev/ttyUSB0 115200
cargo run --bin scope -- rtt <TARGET> <CHANNEL>  # RTT via probe-rs

cargo test --bin scope              # run unit tests
cargo test --bin scope <substr>     # run a single test, e.g. cargo test --bin scope test_rhs
cargo test --test tui_e2e           # run the end-to-end TUI tests (Unix only)
```

- This is a **binary-only crate** (no lib target). Use `cargo test --bin scope` — `cargo test --lib` fails with "no library targets". Unit tests live in `#[cfg(test)] mod tests` blocks inside the source files they cover.
- **End-to-end TUI tests** are in `tests/tui_e2e.rs` (Unix only): they spawn the real binary in a PTY (`portable-pty`), connect it to a virtual serial port (`openpty`), inject keystrokes, and assert on the screen reconstructed by a `vt100` parser. The serial-RX test is `#[ignore]`d because byte transport over a PTY-backed serial port is platform dependent (`serialport` can't set baud via ioctl on a macOS PTY); run it with `cargo test --test tui_e2e -- --ignored`.
- `src/main.rs` has `#![deny(warnings)]`, so any compiler warning fails the build. Keep the tree warning-clean.
- Global CLI options (before the subcommand): `-c/--capacity` (scrollback lines, default 2000), `-t/--tag-file` (default `tags.yml`), `-l/--latency` (ms, clamped 0..=100000, default 100), `-n/--name` (session record base name, default a timestamp), `--headless` (see below). The session can also be renamed at runtime with `!rename <name>` in the command bar.
- **Headless mode** (`--headless`): no TUI — a raw terminal↔wire bridge. A `graphics/headless.rs` task takes the graphics slot (same `GraphicsCommand` channel + tx/rx/logger consumers) and just writes RX bytes to stdout (logs colored via ANSI, no timestamps/scrollback/persistence). The Inputs task carries a `raw: bool` overlay on `InputsShared` (not a new `InputMode`): raw keys are encoded to VT bytes (`inputs/key_encode.rs`) and sent straight to `tx`; `Ctrl+K` drops into the existing `Normal` command bar (blinking `> ` prompt rendered by the headless task), Enter runs the command and returns to raw, Esc quits. The interface tasks forward RX immediately (per-byte / per-chunk) instead of `\n`-framing when `headless` is set.
- Optional config file (`infra/config.rs`): `<config_dir>/scope/config.toml` (e.g. `~/.config/scope/config.toml`, alongside the crash backups). Currently supports `capacity` and `tag_file`. Resolution precedence is **CLI flag > config.toml > built-in default** (`Config::load` is folded into `main`'s single fatal-error flow). A missing file/field falls through to defaults; a malformed file or unknown key is a fatal error (`deny_unknown_fields`). Path values (`tag_file`) are used verbatim — there is no shell involved, so `~` and `$VAR` are **not** expanded; use an absolute path.
- `Ble` is declared as a subcommand but is not implemented (returns an error).

## Architecture

The app is a **multi-threaded actor system**. `main.rs` (`app_serial` / `app_rtt`) wires everything up, spawns four long-lived tasks on their own OS threads, and `join`s them. The two app functions are near-duplicates differing only in which interface (serial vs RTT) they spawn.

### Tasks and shared state (`infra/task.rs`)

Every subsystem is a `Task<S, M>`: it owns shared state `S` behind an `Arc<RwLock<S>>` and receives `M` commands over an `std::sync::mpsc` channel. Other tasks get a **read-only** `Shared<S>` handle (`task.shared_ref()`) to observe state, and a `Sender<M>` to drive it. The four tasks:

- **Interface** (`interfaces/`) — owns the serial port or RTT connection. Enum-dispatched: `InterfaceTask` / `InterfaceCommand` / `InterfaceShared` / `InterfaceType` select between `serial_if.rs` and `rtt_if.rs`.
- **Inputs** (`inputs/inputs_task.rs`) — the command bar. Parses keystrokes, manages input history (`inputs/history.rs`), and has two `InputMode`s: `Normal` and `Search` (plus a `raw` passthrough flag used only in headless mode).
- **Graphics** (`graphics/graphics_task.rs`) — renders the TUI, owns the scrollback buffer, handles selection/scrolling, persists the session to a timestamped `.txt` file, and is the sink for log messages. In headless mode it is replaced by `graphics/headless.rs` (same task slot, plain stdout, no TUI). Line-pinned features (bookmarks: right-click to toggle, `Tab`/`Shift+Tab` to jump, yellow timestamp) live in `graphics/screen.rs` and key off the stable per-line `BufferLine::id` so they survive scrollback rotation and filter changes.
- **PluginEngine** (`plugin/engine.rs`) — runs a Tokio runtime hosting Lua plugins.

### Data buses (`infra/mpmc.rs`)

Two custom fan-out MPMC channels carry `Arc<TimedBytes>` payloads. A `Producer::produce` clones the payload to **every** registered `Consumer` (with optional loopback exclusion by consumer id):

- **`tx_channel`** — bytes to transmit. Consumers: interface (writes to wire), plugin (so `on_*_send` hooks see it), graphics (so it's displayed).
- **`rx_channel`** — bytes received from the wire. Consumers: plugin (`on_*_recv` hooks), graphics (display).

Consumer/producer counts are fixed in `main.rs` (`tx_channel` has 3 consumers, `rx_channel` has 2); adding a consumer means updating those counts.

### Command-bar syntax (parsed in `inputs/inputs_task.rs`)

What the user types is transformed before being sent:

- **`$..` hex sequences** — `replace_hex_sequence` turns `$01 02`, `$0102`, `$01$02` into raw bytes. `,`, `_`, `-`, `.`, space and `$` act as separators between bytes within a sequence.
- **`@tag` tags** — `replace_tag_sequence` + `infra/tags.rs` resolve `@name` to a value from the tag file (default `tags.yml`, a YAML `name: value` map). `@` and whitespace delimit a tag name.
- **`!plugin args`** — invokes a Lua plugin command.

Special-character rendering for the display lives in `graphics/special_char.rs` (the `to_special_char` iterator that splits text into `Plain`/`Special` runs for highlighting). Both the tag filter and this iterator share the `SpecialCharPosition` type.

### Plugins (`plugin/`)

Plugins are Lua scripts (mlua, `lua54` vendored) returning a table `M`. The engine calls lifecycle/event hooks by name: `on_load`, `on_unload`, `on_serial_connect`/`on_serial_disconnect`/`on_serial_send`/`on_serial_recv` (and `on_rtt_*` equivalents), plus any `M.<name>` the user calls via `!plugin <name>`. Plugins reach back into the app through the `bridge`/`method_call` gates. See `plugins/README.md` for the plugin developer guide.

## Logging

`infra/logger.rs` provides a channel-based logger; each task gets a clone tagged with its source name. Use the `error!`, `warning!`, `success!`, `info!` macros — messages fan in to the Graphics task for display.

## Manually testing the TUI

There is a `test-tui` skill (`.claude/skills/test-tui/`) that drives the running TUI end-to-end without hardware: a virtual serial port via `socat`, the app inside `tmux`, keystroke injection with `tmux send-keys`, and screen/colour inspection via `tmux capture-pane`. Use it to verify send/receive behavior and visual layout.
