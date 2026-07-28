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
- **Icon mode** (`src/selector.rs`, issues #229/#230): when `serial`/`rtt` is launched **without** its positional args (`scope serial`, `scope --headless rtt`, …), `main` runs an interactive ratatui picker *before* spawning any task, to choose port+baud (serial: a list from `list::usb_ports`) or target+channel (rtt: text fields, since a chip name isn't enumerable). `resolve_serial`/`resolve_rtt` in `main.rs` gate it: only when the missing arg *and* an interactive terminal (`stdin/stdout.is_terminal()`) are present — piped/scripted runs fall through to the old "start disconnected" behaviour, as does the picker's `Skip` (`s`). `Quit` (`q`/`Esc`) exits before the app starts. The picker owns its own crossterm session (raw mode + alternate screen) and restores the terminal via `Tui`'s `Drop`. Pure state logic (`move_selection`, `parse_baud`, `parse_channel`, `initial_baud_index`) is unit-tested; the render/loop is exercised via the `test-tui` skill.
- **Headless mode** (`--headless`): no TUI — a raw terminal↔wire bridge. A `graphics/headless.rs` task takes the graphics slot (same `GraphicsCommand` channel + tx/rx/logger consumers) and just writes RX bytes to stdout (logs colored via ANSI, no timestamps/scrollback/persistence). The Inputs task carries a `raw: bool` overlay on `InputsShared` (not a new `InputMode`): raw keys are encoded to VT bytes (`inputs/key_encode.rs`) and sent straight to `tx`; `Ctrl+K` drops into the existing `Normal` command bar (blinking `> ` prompt rendered by the headless task), Enter runs the command and returns to raw, Esc quits. The interface tasks forward RX immediately (per-byte / per-chunk) instead of `\n`-framing when `headless` is set.
- Optional config file (`infra/config.rs`): `<config_dir>/scope/config.toml` (e.g. `~/.config/scope/config.toml`, alongside the crash backups). Supports `capacity`, `tag_file`, and an optional `[shortcuts]` table (see below). Resolution precedence is **CLI flag > config.toml > built-in default** (`Config::load` is folded into `main`'s single fatal-error flow). A missing file/field falls through to defaults; a malformed file or unknown key is a fatal error (`deny_unknown_fields`). Path values (`tag_file`) are used verbatim — there is no shell involved, so `~` and `$VAR` are **not** expanded; use an absolute path.
- **Custom shortcuts** (`inputs/keymap.rs`, issue #211): the optional `[shortcuts]` config table (`action = "Key+Combo"`) remaps the 15 action/navigation keys. `Action`/`KeyBinding`/`Keymap` own all key knowledge; `config.rs` stays a `BTreeMap<String,String>` and never enumerates the actions. Shortcuts have no CLI flag, so precedence is config.toml > default; `Keymap::from_config(config.shortcuts)` is built in `main` and threaded (by value, no lock) through `app_serial`/`app_rtt` into `InputsConnections`. Config modifier names are **logical** and lowered to the real per-platform crossterm event in `keymap::resolve` — this replaces the old `CTRL_MODIFIER`/`ACTION_MODIFIER` consts in `handle_key_input` (only `ACTION_MODIFIER`, for the fixed `Alt+Enter` arm, remains). `handle_key_input` resolves a remappable action via `private.keymap.action_for()` **before** the intrinsic key match (`try_run_action`); a `Tab`-bound `next_bookmark` falls through to the `@tag` autocomplete arm while the pop-up is up. Text-editing/intrinsic keys (typing, Enter, Esc, arrows, Home/End, Backspace/Delete, headless `Ctrl+K`/`Ctrl+Q`) stay hardcoded and are rejected as override targets (`reserved_reason`). Unknown actions, bad key strings, reserved keys and duplicate bindings are fatal config errors. An unbound `Ctrl`/`Alt`+letter is now swallowed rather than typed literally.
- `Ble` is declared as a subcommand but is not implemented (returns an error).
- **Release & installers** (issue #229): distribution is driven by [cargo-dist](https://opensource.axo.dev/cargo-dist/) — config in `dist-workspace.toml` (`[dist]`), the `[profile.dist]` profile and `[package.metadata.wix]` GUIDs in `Cargo.toml`. `.github/workflows/release.yml` is **dist-generated** (do not hand-edit; regenerate with `dist init && dist generate`) and fires on a `vX.Y.Z` tag, building tarballs/zip + shell/powershell installers + a Windows `.msi`. Publishing to crates.io is *not* done by dist — `.github/workflows/publish-crates.yml` handles that on the same tag (OIDC). Maintainer release flow: bump the version, push the `vX.Y.Z` tag. `pr-run-mode = "upload"` makes PRs build the same artifacts (installers included) and attach them to the Actions run without publishing — that's how the `.msi` is tested before a tag (`gh run download <run-id>`). `wix/main.wxs` is hand-edited (dist `allow-dirty = ["msi"]`) to add the four per-command Start-Menu shortcuts + their icons; `libudev-dev` is installed on the Linux runner via `[dist.dependencies.apt]`.
- **Release security** (defense-in-depth so no PR/non-owner can cut a release): a release only fires on a `vX.Y.Z` tag; the `release-tags` repo ruleset restricts creating tags to admins; `publish-crates.yml` is owner-guarded (`github.actor == github.repository_owner`), goes through the reviewer-gated `crates` environment, and checks tag==Cargo.toml version; `main` requires code-owner review (`.github/CODEOWNERS`) for release-critical paths. `tests/release_security.rs` pins these workflow invariants and fails CI if they regress — keep it green and do not weaken it. Repo-settings pieces (rulesets, environment reviewers) live in GitHub, not the tree.
- **Windows icons** (issue #230): `installer/icons/*.ico` (one base + four per-command variants) are generated by the standalone helper `installer/gen-icons/` (its own workspace, so `image`/`resvg`/`ico` stay out of the scope build) from `imgs/scope-logo.png` + Font Awesome glyphs. `build.rs` embeds `scope.ico` into `scope.exe` on Windows (`winresource`, a `cfg(windows)` build-dep; no-op elsewhere).

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

- **Installed plugins** (`plugin/installed.rs`, issues #36/#37): `!plugin install <file>` loads a plugin *and* records its name in a TOML manifest `<config_dir>/scope/plugins/installed.toml` (next to the staged `.lua` files); `!plugin uninstall <name>` removes it from the manifest and deletes the staged copy; `!plugin list` prints the set. The engine owns the manifest: `PluginEngineCommand::{InstallPlugin,UninstallPlugin,ListInstalledPlugins}` handle the commands, and `load_installed_plugins` (called once at the top of `task_async`, before the command loop) auto-loads each installed plugin from its staged copy, logging per plugin like `!plugin load`. Install persists only after a successful load, dedupes, and treats a missing/malformed manifest or a broken entry as logged-and-skipped (never fatal — it's program-managed state, not the user's `config.toml`). Uninstall errors if the plugin isn't installed and is orthogonal to `unload` — it does **not** stop a running instance (that's `!plugin unload`); the name is normalized via `get_plugin_name` so `foo`/`foo.lua` both work, and it never deletes a bundled `STDLIB` file (`scope.lua`/`shell.lua`) even if a hand-edited manifest lists a reserved name.

## Logging

`infra/logger.rs` provides a channel-based logger; each task gets a clone tagged with its source name. Use the `error!`, `warning!`, `success!`, `info!` macros — messages fan in to the Graphics task for display.

## Manually testing the TUI

There is a `test-tui` skill (`.claude/skills/test-tui/`) that drives the running TUI end-to-end without hardware: a virtual serial port via `socat`, the app inside `tmux`, keystroke injection with `tmux send-keys`, and screen/colour inspection via `tmux capture-pane`. Use it to verify send/receive behavior and visual layout.
