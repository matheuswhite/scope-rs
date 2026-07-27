//! End-to-end tests that drive the real `scope` TUI, the Rust equivalent of the
//! manual `test-tui` skill procedure.
//!
//! How it works (no `socat`/`tmux` required):
//!   * A virtual serial port is created with `openpty` (`VirtualSerial`); its
//!     slave path is handed to `scope serial <path> <baud>` so the app has a
//!     port to "connect" to.
//!   * The app is spawned inside a real PTY via `portable-pty`, which gives the
//!     controlling terminal that crossterm's raw mode needs.
//!   * Keystrokes are injected by writing to the PTY master.
//!   * A `vt100` parser consumes the PTY output and reconstructs the rendered
//!     screen — the equivalent of `tmux capture-pane -p`.
//!
//! These tests are Unix-only and spawn the built binary, so they are slower than
//! the unit tests. Run them with:
//!   cargo test --test tui_e2e
//! The serial-RX test runs on Linux but is `#[ignore]`d on macOS: macOS sets the
//! baud rate via the IOSSIOSPEED ioctl, which a PTY rejects with ENOTTY, so scope
//! can't open the virtual serial port there. On macOS run it explicitly (it will
//! fail to connect) with:
//!   cargo test --test tui_e2e -- --ignored

#![cfg(unix)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

const ROWS: u16 = 40;
const COLS: u16 = 160;
const READY: Duration = Duration::from_secs(20);
const SETTLE: Duration = Duration::from_secs(10);

/// A PTY pair acting as a virtual serial port. `path` is the slave device path
/// given to `scope`; `master` is the other end of the wire used by the test.
struct VirtualSerial {
    master: File,
    /// Kept open so the pts persists and the test never steals scope's RX bytes.
    _slave: OwnedFd,
    path: PathBuf,
}

impl VirtualSerial {
    fn new() -> Self {
        let pty = nix::pty::openpty(None, None).expect("openpty for virtual serial");
        let path = nix::unistd::ttyname(&pty.slave).expect("ttyname of serial slave");
        VirtualSerial {
            master: File::from(pty.master),
            _slave: pty.slave,
            path,
        }
    }
}

/// A running `scope` instance with its rendered screen observable.
struct Tui {
    _master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    serial: VirtualSerial,
    _tmp: tempfile::TempDir,
}

/// Options for [`Tui::start_with`]. Defaults spawn a plain TUI `scope serial`
/// with no tags, config, or pre-installed plugins.
#[derive(Default)]
struct StartOpts<'a> {
    /// Tag file entries (`name`, `value`).
    tags: &'a [(&'a str, &'a str)],
    /// Contents of `config.toml`, or `None` for no config file.
    config_toml: Option<&'a str>,
    /// Launch with the `--headless` global flag.
    headless: bool,
    /// Plugins to pre-install as `(name, lua_source)`: each is written as
    /// `<plugins_dir>/<name>.lua` and listed in a generated `installed.toml`.
    installed_plugins: &'a [(&'a str, &'a str)],
    /// Verbatim `installed.toml` contents, overriding the generated manifest.
    /// Use to seed a malformed manifest; the plugin `.lua` files (if any) still
    /// come from `installed_plugins`.
    raw_manifest: Option<&'a str>,
}

impl Tui {
    /// Launch `scope serial` connected to a fresh virtual serial port, with an
    /// optional tag file built from `tags`.
    fn start(tags: &[(&str, &str)]) -> Tui {
        Self::start_with(StartOpts {
            tags,
            ..Default::default()
        })
    }

    /// Launch `scope` under [`StartOpts`]. The config directory is always
    /// isolated to the temp tree (via `HOME`/`XDG_CONFIG_HOME`), so a real user
    /// config can never affect a test; `config_toml` (when set) is written to the
    /// location `dirs::config_dir()` resolves on each platform, and any
    /// pre-installed plugins / manifest are seeded into the plugins directory so
    /// the engine auto-loads them at start-up (issue #36).
    fn start_with(opts: StartOpts) -> Tui {
        let StartOpts {
            tags,
            config_toml,
            headless,
            installed_plugins,
            raw_manifest,
        } = opts;

        let serial = VirtualSerial::new();
        let tmp = tempfile::tempdir().expect("tempdir");

        let tags_path = tmp.path().join("tags.yml");
        // Always write valid YAML: an empty document deserializes to null and
        // would make TagList::new fail to build a map, so use `{}` when empty.
        let tags_yaml: String = if tags.is_empty() {
            "{}\n".to_string()
        } else {
            tags.iter().map(|(k, v)| format!("{k}: {v}\n")).collect()
        };
        std::fs::write(&tags_path, tags_yaml).expect("write tag file");

        // Config isolation. On Linux `dirs::config_dir()` honors
        // `XDG_CONFIG_HOME`; on macOS it uses `$HOME/Library/Application
        // Support` and ignores XDG. Point both at the temp tree, and (when a
        // config is given) write it to both candidate paths so whichever the
        // platform picks is present.
        let xdg = tmp.path().join("xdg");
        let mac_cfg = tmp.path().join("Library").join("Application Support");
        if let Some(cfg) = config_toml {
            for base in [xdg.join("scope"), mac_cfg.join("scope")] {
                std::fs::create_dir_all(&base).expect("create config dir");
                std::fs::write(base.join("config.toml"), cfg).expect("write config");
            }
        }

        // Pre-install plugins: write each `<name>.lua` into the plugins dir and
        // a manifest (either the verbatim `raw_manifest`, or one generated from
        // the plugin names), under both candidate config roots, so the engine
        // sees them at start-up regardless of platform.
        if !installed_plugins.is_empty() || raw_manifest.is_some() {
            let manifest = raw_manifest.map(str::to_string).unwrap_or_else(|| {
                format!(
                    "plugins = [{}]\n",
                    installed_plugins
                        .iter()
                        .map(|(name, _)| format!("\"{name}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });
            for base in [xdg.join("scope"), mac_cfg.join("scope")] {
                let plugins = base.join("plugins");
                std::fs::create_dir_all(&plugins).expect("create plugins dir");
                for (name, source) in installed_plugins {
                    std::fs::write(plugins.join(format!("{name}.lua")), source)
                        .expect("write plugin");
                }
                std::fs::write(plugins.join("installed.toml"), &manifest).expect("write manifest");
            }
        }

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("openpty for app");

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_scope"));
        cmd.args(["-t", tags_path.to_str().unwrap()]);
        if headless {
            cmd.arg("--headless"); // global flag, before the subcommand
        }
        cmd.arg("serial");
        cmd.arg(serial.path.to_str().unwrap());
        cmd.arg("115200");
        cmd.cwd(tmp.path()); // session log + .scope_history land here, cleaned with tmp
        cmd.env("TERM", "xterm-256color");
        cmd.env("HOME", tmp.path());
        cmd.env("XDG_CONFIG_HOME", &xdg);

        let child = pair.slave.spawn_command(cmd).expect("spawn scope");
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer = pair.master.take_writer().expect("take writer");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
        {
            let parser = parser.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    parser.lock().unwrap().process(&buf[..n]);
                }
            });
        }

        Tui {
            _master: pair.master,
            writer,
            parser,
            child,
            serial,
            _tmp: tmp,
        }
    }

    /// The staged-plugins directory the running app uses (`<config_dir>/scope/
    /// plugins`), resolved to the platform-correct root under the isolated temp
    /// config tree. Used to seed or inspect the install manifest.
    fn plugins_dir(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        let base = self._tmp.path().join("Library").join("Application Support");
        #[cfg(not(target_os = "macos"))]
        let base = self._tmp.path().join("xdg");
        base.join("scope").join("plugins")
    }

    /// The currently rendered screen as plain text (like `tmux capture-pane -p`).
    fn screen(&self) -> String {
        self.parser.lock().unwrap().screen().contents()
    }

    /// Block until the rendered screen contains `needle`, returning the screen.
    /// Panics (with the last screen) on timeout.
    fn wait_for(&self, needle: &str, timeout: Duration) -> String {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            if screen.contains(needle) {
                return screen;
            }
            if start.elapsed() > timeout {
                panic!(
                    "timed out waiting for {needle:?}.\n--- screen ---\n{screen}\n--------------"
                );
            }
            thread::sleep(Duration::from_millis(80));
        }
    }

    /// Simulate the terminal emulator clearing its own grid (what Cmd+K does in
    /// Zed): wipe the parser screen directly, as the app receives no event for it.
    /// Returns the (blank) screen captured while holding the lock, so the reader
    /// thread can't refill it before the caller inspects it.
    fn simulate_external_clear(&self) -> String {
        let mut parser = self.parser.lock().unwrap();
        parser.process(b"\x1b[3J\x1b[2J\x1b[H");
        parser.screen().contents()
    }

    /// Type text into the command bar (raw bytes to the PTY).
    fn type_text(&mut self, text: &str) {
        self.writer
            .write_all(text.as_bytes())
            .expect("write keystrokes");
        self.writer.flush().expect("flush keystrokes");
    }

    /// Press Enter (carriage return, as a terminal sends it).
    fn press_enter(&mut self) {
        self.type_text("\r");
    }

    /// Block until the TUI has finished its first render — the precondition for
    /// injecting keystrokes — by waiting for the configured baud in the status bar.
    ///
    /// We deliberately do NOT wait for the "Connected at ..." serial log,
    /// because whether it ever appears is platform-dependent: a PTY-backed port
    /// connects on Linux but not on macOS, where setting the baud via the
    /// IOSSIOSPEED ioctl fails with ENOTTY (the same limitation that gates the
    /// macOS-only ignore on the RX test). The status bar is a portable,
    /// connection-independent render signal. A live connection isn't needed here
    /// anyway — the command bar parses and echoes input regardless of link
    /// state, and the PTY buffers keystrokes so none are lost even if written
    /// before crossterm starts reading.
    fn wait_until_ready(&self) {
        self.wait_for("115200bps", READY);
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn hex_with_multiple_dollars_sends_correct_bytes() {
    // Regression for issue #178: `$01 $02` must render as \x01\x02, not \x01$02.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("$01 $02");
    tui.press_enter();

    tui.wait_for("\\x01\\x02\\r\\n", SETTLE);
}

#[test]
fn hex_dollars_no_separator_sends_correct_bytes() {
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("$01$02");
    tui.press_enter();

    tui.wait_for("\\x01\\x02\\r\\n", SETTLE);
}

#[test]
fn hex_mixed_with_plain_text_renders_correctly() {
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("$01 $02 hello");
    tui.press_enter();

    tui.wait_for("\\x01\\x02hello\\r\\n", SETTLE);
}

#[test]
fn adjacent_tags_both_resolve() {
    // Regression for the tag half of issue #178: `@tag1@tag2` must resolve both.
    let mut tui = Tui::start(&[("tag1", "hello"), ("tag2", "world")]);
    tui.wait_until_ready();

    tui.type_text("@tag1@tag2");
    tui.press_enter();

    tui.wait_for("helloworld\\r\\n", SETTLE);
}

#[test]
fn tag_autocomplete_lists_only_matching_tags() {
    let mut tui = Tui::start(&[("tag1", "hello"), ("tag2", "world"), ("temperature", "25")]);
    tui.wait_until_ready();

    tui.type_text("@ta");

    let screen = tui.wait_for("@tag1", SETTLE);
    assert!(
        screen.contains("@tag2"),
        "expected @tag2 in popup.\n{screen}"
    );
    assert!(
        !screen.contains("temperature"),
        "non-matching tag should be filtered out.\n{screen}"
    );
}

#[test]
fn regex_search_toggles_with_ctrl_e_and_matches_each_line() {
    // Issue #209: Ctrl+E toggles regex mode in search. As a literal string
    // `err \d+` matches nothing; as a regex it matches the digit run on each
    // injected line.
    //
    // The pattern is anchored to `err ` on purpose: on Linux the PTY serial
    // port connects and adds a "Connected at .../dev/pts/N with 115200bps" log
    // line whose digits a bare `\d+` would also match (4 hits, not 2), while on
    // macOS the port never connects (2 hits). Requiring `err ` excludes that
    // connection line, so the match count is a deterministic 2 on both.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    // Two TX lines with digit runs give the regex something to match.
    tui.type_text("err 12");
    tui.press_enter();
    tui.wait_for("err 12\\r\\n", SETTLE);
    tui.type_text("err 345");
    tui.press_enter();
    tui.wait_for("err 345\\r\\n", SETTLE);

    // Enter search mode (Ctrl+F = 0x06) and type the pattern.
    tui.type_text("\x06");
    tui.type_text("err \\d+");
    // Wait until the pattern shows in the search bar (keystrokes landed). Regex
    // is off by default, so the literal `err \d+` matches nothing.
    let plain = tui.wait_for("err \\d+", SETTLE);
    assert!(
        plain.contains("[  ]"),
        "regex should start disabled ([  ]).\n{plain}"
    );
    assert!(
        plain.contains("[--/--]"),
        "literal `err \\d+` should match nothing.\n{plain}"
    );

    // Toggle regex on (Ctrl+E = 0x05): the two `err <digits>` lines now match.
    tui.type_text("\x05");
    let regex = tui.wait_for("[1/2]", SETTLE);
    assert!(
        regex.contains("[.*]"),
        "regex box should read [.*] once enabled.\n{regex}"
    );
}

#[test]
fn tag_autocomplete_down_arrow_then_tab_completes_selected_tag() {
    // Issue #177: the arrows move the highlight inside the pop-up and Tab
    // completes the *selected* entry, not just the first one. With the list
    // sorted [tag1, tag2], one Down selects tag2, so Tab must yield "@tag2"
    // (which resolves to tag2's value on Enter), never "@tag1".
    let mut tui = Tui::start(&[("tag1", "hello"), ("tag2", "world")]);
    tui.wait_until_ready();

    tui.type_text("@ta");
    tui.wait_for("@tag2", SETTLE);

    tui.type_text("\x1b[B"); // Down arrow: highlight tag2
    tui.type_text("\t"); // Tab: complete the highlighted entry
    tui.press_enter();

    tui.wait_for("world\\r\\n", SETTLE);
}

#[test]
fn bracketed_paste_inserts_into_command_bar() {
    // A terminal delivers a paste wrapped in the bracketed-paste markers
    // (ESC[200~ ... ESC[201~). It must land in the command bar and be sendable.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("\x1b[200~hello world\x1b[201~");
    tui.press_enter();

    tui.wait_for("hello world\\r\\n", SETTLE);
}

#[test]
fn bracketed_paste_strips_control_chars() {
    // A multi-line paste must not inject newlines into the single-line command
    // bar or submit on its own: the control chars are dropped, so "ab\ncd"
    // becomes "abcd" and is only sent on the explicit Enter.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("\x1b[200~ab\ncd\x1b[201~");
    tui.press_enter();

    tui.wait_for("abcd\\r\\n", SETTLE);
}

#[test]
fn screen_recovers_after_external_clear() {
    // Regression for issue #166: an external terminal clear (e.g. Cmd+K in Zed's
    // terminal) wipes the grid without notifying the app, leaving ratatui's diff
    // buffer stale so only changed cells repaint. The periodic full repaint must
    // restore the screen on its own.
    let tui = Tui::start(&[]);
    tui.wait_until_ready();

    let blanked = tui.simulate_external_clear();
    assert!(
        !blanked.contains("115200bps"),
        "screen should be blank right after the external clear.\n{blanked}"
    );

    // The periodic full repaint should redraw the whole status bar within a few
    // seconds (the period is 1s) without any input from the app's user.
    tui.wait_for("115200bps", Duration::from_secs(5));
}

#[test]
fn scrollbar_appears_only_when_buffer_overflows_viewport() {
    // Issue #134: a vertical scrollbar indicates scroll position. It must stay
    // hidden while the content fits and appear once the buffer overflows the
    // viewport. The ▲/▼ arrow heads are unique to the scrollbar on screen.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("first line");
    tui.press_enter();
    let screen = tui.wait_for("first line", SETTLE);
    assert!(
        !screen.contains('▲') && !screen.contains('▼'),
        "scrollbar must be hidden while content fits.\n{screen}"
    );

    // Overflow the viewport: ROWS lines always exceed the visible height, which
    // is ROWS minus the command bar and borders.
    for i in 1..=ROWS {
        tui.type_text(&format!("filler {i}"));
        tui.press_enter();
    }
    let screen = tui.wait_for(&format!("filler {ROWS}"), SETTLE);
    assert!(
        screen.contains('▲') && screen.contains('▼'),
        "scrollbar arrows must appear once content overflows.\n{screen}"
    );
}

#[test]
fn scrollbar_thumb_reaches_both_ends() {
    // Issue #134: the thumb must span the whole track, not stop partway. At the
    // bottom (auto-scroll) the thumb's last cell sits just above the ▼ arrow; at
    // the top its first cell sits just below ▲. The rightmost column of each
    // scrollbar row holds an arrow (▲/▼), the thumb (█) or the track, so we can
    // read the glyphs straight off the end of each rendered line.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    for i in 1..=ROWS {
        tui.type_text(&format!("row {i}"));
        tui.press_enter();
    }

    let rows_ending_with = |screen: &str, glyph: char| -> Vec<usize> {
        screen
            .lines()
            .enumerate()
            .filter(|(_, line)| line.chars().last() == Some(glyph))
            .map(|(i, _)| i)
            .collect()
    };

    // Auto-scroll pins us to the bottom: the thumb reaches the bottom of the track.
    let bottom = tui.wait_for(&format!("row {ROWS}\\r\\n"), SETTLE);
    assert!(
        !bottom.contains("row 1\\r\\n"),
        "oldest line should be off-screen at the bottom.\n{bottom}"
    );
    let down_arrow = rows_ending_with(&bottom, '▼');
    let thumb = rows_ending_with(&bottom, '█');
    let (Some(&arrow), Some(&last_thumb)) = (down_arrow.first(), thumb.last()) else {
        panic!("expected a thumb and a ▼ arrow at the bottom.\n{bottom}");
    };
    assert_eq!(
        last_thumb,
        arrow - 1,
        "thumb must reach the bottom of the track (just above ▼).\n{bottom}"
    );

    // PageUp scrolls a full page to the top: the thumb reaches the top of the track.
    tui.type_text("\x1b[5~");
    let top = tui.wait_for("row 1\\r\\n", SETTLE);
    let up_arrow = rows_ending_with(&top, '▲');
    let thumb = rows_ending_with(&top, '█');
    let (Some(&arrow), Some(&first_thumb)) = (up_arrow.first(), thumb.first()) else {
        panic!("expected a thumb and a ▲ arrow at the top.\n{top}");
    };
    assert_eq!(
        first_thumb,
        arrow + 1,
        "thumb must reach the top of the track (just below ▲).\n{top}"
    );
}

#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "macOS sets baud via the IOSSIOSPEED ioctl, which a PTY rejects with ENOTTY, so scope can't open the virtual serial port; Linux sets baud via termios and works"
)]
fn received_bytes_are_displayed() {
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.serial
        .master
        .write_all(b"ping\r\n")
        .expect("write to wire");
    tui.serial.master.flush().expect("flush wire");

    tui.wait_for("ping", SETTLE);
}

#[test]
fn custom_shortcut_from_config_remaps_action() {
    // Issue #211: a `[shortcuts]` override both moves an action to a new key and
    // disables the built-in one. Move `record` from Ctrl+R to Ctrl+G. (Ctrl+G is
    // 0x07; we avoid Ctrl+J=0x0a / Ctrl+I=0x09, which terminals send as
    // Enter/Tab.)
    let mut tui = Tui::start_with(StartOpts {
        config_toml: Some("[shortcuts]\nrecord = \"Ctrl+G\"\n"),
        ..Default::default()
    });
    tui.wait_until_ready();

    // The old key is now unbound: Ctrl+R (0x12) must NOT start a recording. Use
    // a sent-text sentinel as an ordering barrier — the single input pipeline
    // guarantees Ctrl+R was processed by the time the sentinel is rendered.
    tui.type_text("\x12"); // Ctrl+R
    tui.type_text("probe");
    tui.press_enter();
    let screen = tui.wait_for("probe", SETTLE);
    assert!(
        !screen.contains("Recording content on"),
        "Ctrl+R must be unbound after remapping record to Ctrl+G.\n{screen}"
    );

    // The new key works: Ctrl+G (0x07) starts a recording.
    tui.type_text("\x07"); // Ctrl+G
    tui.wait_for("Recording content on", SETTLE);
}

#[test]
fn headless_ctrl_f_is_swallowed_in_command_bar() {
    // `search_toggle` (Ctrl+F) must be a no-op in headless mode: it must neither
    // flip into a Search mode the headless bridge cannot render, nor leave a
    // literal 'f' in the command bar. Enter the command bar with Ctrl+K, press
    // Ctrl+F, then type a sentinel 'Z'. The blinking prompt mirrors the command
    // line as `> <text>`, so a correct swallow shows `> Z`; a leaked 'f' would
    // show `> fZ` (and `> Z` would never appear).
    let mut tui = Tui::start_with(StartOpts {
        headless: true,
        ..Default::default()
    });

    tui.type_text("\x0b"); // Ctrl+K -> command bar
    tui.type_text("\x06"); // Ctrl+F -> must be swallowed
    tui.type_text("Z"); // sentinel

    let screen = tui.wait_for("> Z", SETTLE);
    assert!(
        !screen.contains("fZ"),
        "Ctrl+F must not be typed into the headless command bar.\n{screen}"
    );
}

#[test]
fn plugin_install_persists_to_manifest() {
    // Issue #36: `!plugin install <file>` loads the plugin and records it in the
    // manifest so future sessions auto-load it. Install a trivial plugin from an
    // absolute path (independent of the app's cwd), then assert the success log
    // and that `installed.toml` now lists it.
    let plugin_home = tempfile::tempdir().expect("plugin tempdir");
    let plugin_path = plugin_home.path().join("e2e_installed.lua");
    std::fs::write(&plugin_path, "local M = {}\nreturn M\n").expect("write plugin");

    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text(&format!(
        "!plugin install {}",
        plugin_path.to_str().unwrap()
    ));
    tui.press_enter();

    tui.wait_for("installed", SETTLE);

    let manifest = std::fs::read_to_string(tui.plugins_dir().join("installed.toml"))
        .expect("manifest written after install");
    assert!(
        manifest.contains("e2e_installed"),
        "manifest must list the installed plugin.\n{manifest}"
    );
}

#[test]
fn installed_plugin_autoloads_at_startup() {
    // A plugin recorded in the manifest is auto-loaded at start-up (the whole
    // point of install): no `!plugin load` is typed, yet the load log appears.
    let tui = Tui::start_with(StartOpts {
        installed_plugins: &[("autoloaded_plugin", "local M = {}\nreturn M\n")],
        ..Default::default()
    });

    tui.wait_for("autoloaded_plugin", SETTLE);
}

#[test]
fn plugin_install_failure_does_not_persist() {
    // Safety invariant: a plugin that fails to load must NOT be recorded in the
    // manifest (else every future session would try to auto-load a broken
    // plugin). Install a path that doesn't exist and assert the error is logged
    // and the manifest never gains the name.
    let plugin_home = tempfile::tempdir().expect("plugin tempdir");
    let ghost = plugin_home.path().join("ghost_plugin.lua"); // deliberately not created

    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text(&format!("!plugin install {}", ghost.to_str().unwrap()));
    tui.press_enter();

    tui.wait_for("doesn't exist", SETTLE);

    let recorded =
        std::fs::read_to_string(tui.plugins_dir().join("installed.toml")).unwrap_or_default();
    assert!(
        !recorded.contains("ghost_plugin"),
        "a failed install must not persist to the manifest.\n{recorded}"
    );
}

#[test]
fn malformed_manifest_at_startup_is_non_fatal() {
    // A corrupt manifest must be logged and skipped, never abort start-up (it is
    // program-managed state, not the user's config.toml). The app must still
    // become interactive; the parse error being logged proves the engine handled
    // it gracefully (rather than the process aborting or the engine dying mute).
    let tui = Tui::start_with(StartOpts {
        raw_manifest: Some("plugins = not_a_list\n"),
        ..Default::default()
    });

    tui.wait_until_ready();
    tui.wait_for("Cannot parse plugin manifest", SETTLE);
}

#[test]
fn plugin_list_shows_installed_set() {
    // `!plugin list` reports the empty set and, after an install, the names.
    let plugin_home = tempfile::tempdir().expect("plugin tempdir");
    let plugin_path = plugin_home.path().join("listed_plugin.lua");
    std::fs::write(&plugin_path, "local M = {}\nreturn M\n").expect("write plugin");

    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("!plugin list");
    tui.press_enter();
    tui.wait_for("No plugins installed", SETTLE);

    tui.type_text(&format!(
        "!plugin install {}",
        plugin_path.to_str().unwrap()
    ));
    tui.press_enter();
    tui.wait_for("installed", SETTLE);

    tui.type_text("!plugin list");
    tui.press_enter();
    // The "Installed plugins:" prefix only comes from the list command's
    // non-empty branch (the name alone is already on screen from the install).
    tui.wait_for("Installed plugins:", SETTLE);
}

#[test]
fn plugin_install_rejects_reserved_name() {
    // `scope`/`shell` are the bundled stdlib names; installing one must error and
    // not be persisted.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("!plugin install scope");
    tui.press_enter();
    tui.wait_for("reserved plugin name", SETTLE);

    let recorded =
        std::fs::read_to_string(tui.plugins_dir().join("installed.toml")).unwrap_or_default();
    assert!(
        !recorded.contains("scope"),
        "a reserved name must not be installed.\n{recorded}"
    );
}

#[test]
fn plugin_uninstall_removes_from_manifest_and_deletes_file() {
    // Issue #37: `!plugin uninstall <name>` drops the plugin from the manifest
    // (so it no longer auto-loads) and deletes its staged copy. Pre-install one,
    // uninstall it, and assert both are gone.
    let tui = {
        let mut tui = Tui::start_with(StartOpts {
            installed_plugins: &[("uninst_plugin", "local M = {}\nreturn M\n")],
            ..Default::default()
        });
        tui.wait_until_ready();

        tui.type_text("!plugin uninstall uninst_plugin");
        tui.press_enter();
        tui.wait_for("uninstalled", SETTLE);
        tui
    };

    let manifest =
        std::fs::read_to_string(tui.plugins_dir().join("installed.toml")).unwrap_or_default();
    assert!(
        !manifest.contains("uninst_plugin"),
        "manifest must no longer list the uninstalled plugin.\n{manifest}"
    );
    assert!(
        !tui.plugins_dir().join("uninst_plugin.lua").exists(),
        "the staged plugin file must be deleted on uninstall"
    );
}

#[test]
fn plugin_uninstall_of_not_installed_reports_error() {
    // Uninstalling something that was never installed is a clear error, not a
    // silent no-op.
    let mut tui = Tui::start(&[]);
    tui.wait_until_ready();

    tui.type_text("!plugin uninstall ghost");
    tui.press_enter();
    tui.wait_for("is not installed", SETTLE);
}

#[test]
fn uninstall_never_deletes_bundled_stdlib() {
    // Defense-in-depth: a hand-corrupted manifest that lists a reserved stdlib
    // name must never cause its file to be deleted. Seed a real plugin (so the
    // stdlib is provisioned at start-up) alongside a bogus `scope` manifest
    // entry, then uninstall `scope`: the manifest entry is cleaned but the
    // bundled `scope.lua` survives.
    let tui = {
        let mut tui = Tui::start_with(StartOpts {
            installed_plugins: &[("realplug", "local M = {}\nreturn M\n")],
            raw_manifest: Some("plugins = [\"realplug\", \"scope\"]\n"),
            ..Default::default()
        });
        tui.wait_until_ready();
        tui.wait_for("realplug", SETTLE); // real plugin auto-loaded -> stdlib staged

        tui.type_text("!plugin uninstall scope");
        tui.press_enter();
        tui.wait_for("uninstalled", SETTLE);
        tui
    };

    assert!(
        tui.plugins_dir().join("scope.lua").exists(),
        "uninstall must never delete the bundled scope.lua"
    );
    let manifest =
        std::fs::read_to_string(tui.plugins_dir().join("installed.toml")).unwrap_or_default();
    assert!(
        !manifest.contains("scope"),
        "the bogus scope entry should be cleaned from the manifest.\n{manifest}"
    );
    assert!(
        manifest.contains("realplug"),
        "a legitimately-installed plugin must stay installed.\n{manifest}"
    );
}
