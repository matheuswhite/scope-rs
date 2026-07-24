//! Configurable keyboard shortcuts (issue #211).
//!
//! All key/action knowledge lives here so `handle_key_input` can stay a thin
//! dispatcher and `config.rs` never has to enumerate the action vocabulary.
//!
//! - [`Action`] is the closed set of *remappable* actions (the single source of
//!   truth for their names and built-in bindings). Text-editing and control
//!   keys (typing, Enter, Esc, arrows, Home/End, Backspace/Delete, and the
//!   headless `Ctrl+K`/`Ctrl+Q` chords) are **not** here — they stay hardcoded.
//! - [`KeyBinding`] is a parsed, *resolved* `(KeyCode, KeyModifiers)` pair. Users
//!   write **logical** modifier names (`Ctrl`/`Alt`/`Shift`); [`parse`](KeyBinding::parse)
//!   lowers them to the physical event crossterm actually delivers on the build
//!   target (see [`resolve`]), so matching is a plain equality check and a config
//!   file stays portable across platforms.
//! - [`Keymap`] is the effective action→binding table: built-in [`Default`]
//!   overridden by the optional `[shortcuts]` config section via
//!   [`from_config`](Keymap::from_config), with unknown actions, bad key
//!   strings, reserved keys and duplicate bindings all reported as fatal errors.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::str::FromStr;

/// A remappable action. The enum is the single source of truth for the action
/// vocabulary: its name (config key), its built-in default binding, and — via
/// the dispatch in `inputs_task.rs` — its behavior.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Copy,
    Clear,
    Save,
    Record,
    SearchToggle,
    ToggleCase,
    ToggleRegex,
    PageUp,
    PageDown,
    JumpStart,
    JumpEnd,
    WordLeft,
    WordRight,
    NextBookmark,
    PrevBookmark,
}

// `jump_*` is the only per-platform *default*: on Windows the action modifier is
// Ctrl (not Alt). This is a deliberately different binding, not a delivery quirk,
// so it is encoded here rather than in `resolve`. The macOS `Ctrl+arrow` quirk
// for `word_*` is a delivery detail handled inside `resolve`, so those defaults
// stay uniform strings.
#[cfg(windows)]
const JUMP_START: &str = "Ctrl+PageUp";
#[cfg(not(windows))]
const JUMP_START: &str = "Alt+PageUp";
#[cfg(windows)]
const JUMP_END: &str = "Ctrl+PageDown";
#[cfg(not(windows))]
const JUMP_END: &str = "Alt+PageDown";

impl Action {
    /// Every action, in a stable order (used for defaults, lookups and the
    /// error listing).
    pub const ALL: [Action; 15] = [
        Action::Copy,
        Action::Clear,
        Action::Save,
        Action::Record,
        Action::SearchToggle,
        Action::ToggleCase,
        Action::ToggleRegex,
        Action::PageUp,
        Action::PageDown,
        Action::JumpStart,
        Action::JumpEnd,
        Action::WordLeft,
        Action::WordRight,
        Action::NextBookmark,
        Action::PrevBookmark,
    ];

    /// The `[shortcuts]` config key for this action.
    fn name(self) -> &'static str {
        match self {
            Action::Copy => "copy",
            Action::Clear => "clear",
            Action::Save => "save",
            Action::Record => "record",
            Action::SearchToggle => "search_toggle",
            Action::ToggleCase => "toggle_case",
            Action::ToggleRegex => "toggle_regex",
            Action::PageUp => "page_up",
            Action::PageDown => "page_down",
            Action::JumpStart => "jump_start",
            Action::JumpEnd => "jump_end",
            Action::WordLeft => "word_left",
            Action::WordRight => "word_right",
            Action::NextBookmark => "next_bookmark",
            Action::PrevBookmark => "prev_bookmark",
        }
    }

    /// The built-in (logical) key combo for this action.
    fn default_spec(self) -> &'static str {
        match self {
            Action::Copy => "Ctrl+C",
            Action::Clear => "Ctrl+L",
            Action::Save => "Ctrl+S",
            Action::Record => "Ctrl+R",
            Action::SearchToggle => "Ctrl+F",
            Action::ToggleCase => "Ctrl+W",
            Action::ToggleRegex => "Ctrl+E",
            Action::PageUp => "PageUp",
            Action::PageDown => "PageDown",
            Action::JumpStart => JUMP_START,
            Action::JumpEnd => JUMP_END,
            Action::WordLeft => "Ctrl+Left",
            Action::WordRight => "Ctrl+Right",
            Action::NextBookmark => "Tab",
            Action::PrevBookmark => "Shift+Tab",
        }
    }

    fn from_name(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == s)
    }

    /// Comma-separated list of every valid action name, for error messages.
    fn names_csv() -> String {
        Action::ALL
            .iter()
            .map(|a| a.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The logical modifiers a user wrote in the config string, before they are
/// resolved to the physical crossterm modifiers of the build target.
#[derive(Clone, Copy, Default)]
struct LogicalMods {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

/// A parsed, platform-resolved key binding. Compared against incoming events by
/// plain equality (after canonicalizing both sides).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct KeyBinding {
    /// Canonical code: `Char` lowercased; `Shift+Tab` stored as `BackTab`.
    code: KeyCode,
    /// Physical modifiers crossterm delivers on THIS target (post-resolve),
    /// masked to `CONTROL | ALT | SHIFT`.
    mods: KeyModifiers,
}

impl FromStr for KeyBinding {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, String> {
        let mut tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
        if tokens.iter().any(|t| t.is_empty()) {
            return Err(format!("empty token in \"{spec}\""));
        }
        // `split('+')` always yields at least one element.
        let key_tok = tokens.pop().unwrap();

        let mut m = LogicalMods::default();
        for t in &tokens {
            match t.to_ascii_lowercase().as_str() {
                "ctrl" if !m.ctrl => m.ctrl = true,
                "alt" if !m.alt => m.alt = true,
                "shift" if !m.shift => m.shift = true,
                "ctrl" | "alt" | "shift" => {
                    return Err(format!("duplicate modifier \"{t}\""));
                }
                other => return Err(format!("unknown modifier \"{other}\"")),
            }
        }

        let code = parse_key(key_tok).ok_or_else(|| format!("unknown key \"{key_tok}\""))?;
        // Canonicalize a character to lowercase (crossterm delivers `Ctrl+C` as
        // `Char('c')`), so bindings match regardless of the written case.
        let code = match code {
            KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
            other => other,
        };

        if m.shift {
            if let KeyCode::Char(_) = code {
                return Err("Shift cannot be combined with a character key".into());
            }
            // `Shift+Tab` is delivered as `BackTab` with no Shift bit.
            if code == KeyCode::Tab {
                let mut m2 = m;
                m2.shift = false;
                return Ok(KeyBinding {
                    code: KeyCode::BackTab,
                    mods: resolve(m2, KeyCode::BackTab),
                });
            }
        }

        Ok(KeyBinding {
            mods: resolve(m, code),
            code,
        })
    }
}

impl KeyBinding {
    fn parse(spec: &str) -> Result<KeyBinding, String> {
        spec.parse()
    }

    /// Whether an incoming key event triggers this binding.
    fn matches(&self, ev: &KeyEvent) -> bool {
        let (code, mods) = canonicalize_event(ev);
        code == self.code && mods == self.mods
    }

    /// `Some(reason)` when this (resolved, physical) binding lands on a key that
    /// is reserved for a fixed, non-remappable behavior.
    fn reserved_reason(&self) -> Option<&'static str> {
        let bare = self.mods == KeyModifiers::NONE;
        let shift = self.mods == KeyModifiers::SHIFT;
        match (self.code, self.mods) {
            (KeyCode::Enter, _) => Some("Enter is reserved (send / run / next match)"),
            (KeyCode::Esc, _) => Some("Esc is reserved (quit / leave search)"),
            (KeyCode::Backspace, _) | (KeyCode::Delete, _) => Some("editing keys are reserved"),
            (KeyCode::Up, _) | (KeyCode::Down, _) => {
                Some("Up/Down are reserved (history / search navigation / tag pop-up)")
            }
            (KeyCode::Char('k'), KeyModifiers::CONTROL) => {
                Some("Ctrl+K is reserved (headless command bar)")
            }
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                Some("Ctrl+Q is reserved (headless quit)")
            }
            (KeyCode::Char(_), _) if bare || shift => {
                Some("plain character keys are reserved for typing")
            }
            (KeyCode::Left, _) | (KeyCode::Right, _) | (KeyCode::Home, _) | (KeyCode::End, _)
                if bare =>
            {
                Some("cursor movement keys are reserved")
            }
            _ => None,
        }
    }
}

/// Lower a set of logical modifiers on `code` to the physical modifiers
/// crossterm delivers on this build target. This replaces the old
/// `ACTION_MODIFIER` / `CTRL_MODIFIER` consts for the remappable keys.
fn resolve(m: LogicalMods, code: KeyCode) -> KeyModifiers {
    let mut out = KeyModifiers::NONE;
    // `Shift+Tab` already collapsed to `BackTab` (no Shift bit) by the caller.
    if m.shift && code != KeyCode::BackTab {
        out |= KeyModifiers::SHIFT;
    }
    if m.alt {
        out |= KeyModifiers::ALT;
    }
    if m.ctrl {
        out |= ctrl_physical(code);
    }
    out
}

/// The physical modifier a logical `Ctrl` becomes. On macOS, terminals deliver
/// `Ctrl+<arrow>` as an ESC-prefixed (i.e. Alt) sequence, so a logical `Ctrl`
/// on an arrow key is physically `Alt` there.
#[cfg(target_os = "macos")]
fn ctrl_physical(code: KeyCode) -> KeyModifiers {
    if matches!(
        code,
        KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down
    ) {
        KeyModifiers::ALT
    } else {
        KeyModifiers::CONTROL
    }
}

#[cfg(not(target_os = "macos"))]
fn ctrl_physical(_code: KeyCode) -> KeyModifiers {
    KeyModifiers::CONTROL
}

/// Canonicalize an incoming event so it compares equal to a stored binding:
/// lowercase `Char`, drop `Shift` on `BackTab`, and mask to the three real
/// modifiers (ignoring e.g. `KEYPAD`/`NONE`-adjacent flags crossterm may set).
fn canonicalize_event(ev: &KeyEvent) -> (KeyCode, KeyModifiers) {
    let mut mods = ev.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT);
    let code = match ev.code {
        KeyCode::Char(c) => KeyCode::Char(c.to_ascii_lowercase()),
        KeyCode::BackTab => {
            mods.remove(KeyModifiers::SHIFT);
            KeyCode::BackTab
        }
        other => other,
    };
    (code, mods)
}

/// Parse the key token (case-insensitive for named keys; single characters kept
/// as-is and lowercased by the caller). Returns `None` for anything unknown.
fn parse_key(k: &str) -> Option<KeyCode> {
    let lower = k.to_ascii_lowercase();
    Some(match lower.as_str() {
        "tab" => KeyCode::Tab,
        "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "insert" => KeyCode::Insert,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        fk if fk.starts_with('f')
            && fk[1..]
                .parse::<u8>()
                .map(|n| (1..=12).contains(&n))
                .unwrap_or(false) =>
        {
            KeyCode::F(fk[1..].parse().unwrap())
        }
        _ => {
            let mut it = k.chars();
            match (it.next(), it.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    })
}

/// The effective action→binding table.
#[derive(Debug)]
pub struct Keymap {
    entries: Vec<(Action, KeyBinding)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Keymap {
            entries: Action::ALL
                .into_iter()
                // Every `default_spec()` is a compile-time constant proven to
                // parse and to be non-reserved/collision-free by the unit tests
                // below, so the `expect` can never fire.
                .map(|a| {
                    (
                        a,
                        KeyBinding::parse(a.default_spec()).expect("valid default binding"),
                    )
                })
                .collect(),
        }
    }
}

impl Keymap {
    /// Build the effective keymap: built-in defaults overridden by the optional
    /// `[shortcuts]` config table, fully validated. Any unknown action name,
    /// unparseable combo, reserved-key override or duplicate binding is a fatal
    /// error (matching the crate's "malformed config is fatal" philosophy).
    pub fn from_config(overrides: Option<&BTreeMap<String, String>>) -> Result<Keymap, String> {
        let mut km = Keymap::default();

        if let Some(map) = overrides {
            for (name, spec) in map {
                let action = Action::from_name(name).ok_or_else(|| {
                    format!(
                        "unknown shortcut action \"{name}\" in [shortcuts]; valid actions are: {}",
                        Action::names_csv()
                    )
                })?;
                let binding = KeyBinding::parse(spec).map_err(|e| {
                    format!("invalid key binding \"{spec}\" for shortcut \"{name}\": {e}")
                })?;
                if let Some(reason) = binding.reserved_reason() {
                    return Err(format!(
                        "shortcut \"{name}\" cannot be bound to \"{spec}\": {reason}"
                    ));
                }
                km.set(action, binding);
            }
        }

        km.check_duplicates()?;
        Ok(km)
    }

    fn set(&mut self, action: Action, binding: KeyBinding) {
        // `action` is always present (the table is seeded from `Action::ALL`).
        self.entries
            .iter_mut()
            .find(|(a, _)| *a == action)
            .expect("every action is present in the keymap")
            .1 = binding;
    }

    /// Resolve an incoming key event to the action it triggers, if any.
    /// Duplicate bindings are impossible after validation, so first-match is
    /// unambiguous.
    pub fn action_for(&self, ev: &KeyEvent) -> Option<Action> {
        self.entries
            .iter()
            .find(|(_, b)| b.matches(ev))
            .map(|(a, _)| *a)
    }

    fn check_duplicates(&self) -> Result<(), String> {
        for i in 0..self.entries.len() {
            for j in (i + 1)..self.entries.len() {
                if self.entries[i].1 == self.entries[j].1 {
                    return Err(format!(
                        "shortcuts \"{}\" and \"{}\" are both bound to the same key",
                        self.entries[i].0.name(),
                        self.entries[j].0.name()
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_all_parse_and_are_legal() {
        // Guards the `expect` in `Default` and the collision-free invariant.
        for a in Action::ALL {
            let b = KeyBinding::parse(a.default_spec())
                .unwrap_or_else(|e| panic!("default for {} failed to parse: {e}", a.name()));
            assert!(
                b.reserved_reason().is_none(),
                "default for {} is reserved",
                a.name()
            );
        }
        Keymap::default()
            .check_duplicates()
            .expect("default bindings must be collision-free");
    }

    #[test]
    fn parser_letters_and_case() {
        let expected = KeyBinding {
            code: KeyCode::Char('c'),
            mods: KeyModifiers::CONTROL,
        };
        for s in ["Ctrl+C", "ctrl+c", "CTRL+C", "Ctrl + c"] {
            assert_eq!(KeyBinding::parse(s).unwrap(), expected, "{s}");
        }
    }

    #[test]
    fn parser_named_keys() {
        assert_eq!(
            KeyBinding::parse("PageUp").unwrap(),
            KeyBinding {
                code: KeyCode::PageUp,
                mods: KeyModifiers::NONE
            }
        );
        assert_eq!(KeyBinding::parse("pageup").unwrap().code, KeyCode::PageUp);
        assert_eq!(
            KeyBinding::parse("F5").unwrap(),
            KeyBinding {
                code: KeyCode::F(5),
                mods: KeyModifiers::NONE
            }
        );
        assert_eq!(KeyBinding::parse("f12").unwrap().code, KeyCode::F(12));
    }

    #[test]
    fn parser_shift_tab_is_backtab() {
        let b = KeyBinding::parse("Shift+Tab").unwrap();
        assert_eq!(b.code, KeyCode::BackTab);
        assert_eq!(b.mods, KeyModifiers::NONE);
    }

    #[test]
    fn parser_folds_multiple_modifiers() {
        let b = KeyBinding::parse("Ctrl+Alt+End").unwrap();
        assert_eq!(b.code, KeyCode::End);
        assert_eq!(b.mods, KeyModifiers::CONTROL | KeyModifiers::ALT);
    }

    #[test]
    fn parser_rejections() {
        for s in [
            "",
            "Ctrl+",
            "+A",
            "Ctrl++A",
            "Ctrl+Ctrl+A",
            "Shift+a",
            "F0",
            "F13",
            "Meta+A",
            "Control+A",
            "PgUp",
            "ab",
        ] {
            assert!(
                KeyBinding::parse(s).is_err(),
                "expected {s:?} to be rejected"
            );
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn resolve_ctrl_arrow_is_control_off_macos() {
        assert_eq!(
            KeyBinding::parse("Ctrl+Left").unwrap().mods,
            KeyModifiers::CONTROL
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn resolve_ctrl_arrow_is_alt_on_macos() {
        // Arrows swap to Alt on macOS, but Ctrl+letter does not.
        assert_eq!(
            KeyBinding::parse("Ctrl+Left").unwrap().mods,
            KeyModifiers::ALT
        );
        assert_eq!(
            KeyBinding::parse("Ctrl+C").unwrap().mods,
            KeyModifiers::CONTROL
        );
    }

    #[test]
    #[cfg(windows)]
    fn resolve_jump_default_is_ctrl_on_windows() {
        assert_eq!(
            KeyBinding::parse(Action::JumpStart.default_spec())
                .unwrap()
                .mods,
            KeyModifiers::CONTROL
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn resolve_jump_default_is_alt_off_windows() {
        assert_eq!(
            KeyBinding::parse(Action::JumpStart.default_spec())
                .unwrap()
                .mods,
            KeyModifiers::ALT
        );
    }

    #[test]
    fn matcher_exact_modifiers_and_case_fold() {
        let ctrl_c = KeyBinding::parse("Ctrl+C").unwrap();
        assert!(ctrl_c.matches(&ev(KeyCode::Char('c'), KeyModifiers::CONTROL)));
        // Uppercase (no shift bit) still matches: case is folded.
        assert!(ctrl_c.matches(&ev(KeyCode::Char('C'), KeyModifiers::CONTROL)));
        // A stray SHIFT bit means it does NOT match (exact modifiers, as today).
        assert!(!ctrl_c.matches(&ev(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        )));
    }

    #[test]
    fn matcher_backtab_with_or_without_shift() {
        let b = KeyBinding::parse("Shift+Tab").unwrap();
        assert!(b.matches(&ev(KeyCode::BackTab, KeyModifiers::NONE)));
        assert!(b.matches(&ev(KeyCode::BackTab, KeyModifiers::SHIFT)));
    }

    #[test]
    fn default_matches_current_arms() {
        // Anti-regression: the default keymap resolves the same events the old
        // hardcoded arms matched. Modifier consts mirror `handle_key_input`.
        #[cfg(target_os = "macos")]
        let ctrl_arrow = KeyModifiers::ALT;
        #[cfg(not(target_os = "macos"))]
        let ctrl_arrow = KeyModifiers::CONTROL;

        #[cfg(windows)]
        let action_mod = KeyModifiers::CONTROL;
        #[cfg(not(windows))]
        let action_mod = KeyModifiers::ALT;

        let km = Keymap::default();
        let cases = [
            (ev(KeyCode::Char('c'), KeyModifiers::CONTROL), Action::Copy),
            (ev(KeyCode::Char('l'), KeyModifiers::CONTROL), Action::Clear),
            (ev(KeyCode::Char('s'), KeyModifiers::CONTROL), Action::Save),
            (
                ev(KeyCode::Char('r'), KeyModifiers::CONTROL),
                Action::Record,
            ),
            (
                ev(KeyCode::Char('f'), KeyModifiers::CONTROL),
                Action::SearchToggle,
            ),
            (
                ev(KeyCode::Char('w'), KeyModifiers::CONTROL),
                Action::ToggleCase,
            ),
            (
                ev(KeyCode::Char('e'), KeyModifiers::CONTROL),
                Action::ToggleRegex,
            ),
            (ev(KeyCode::PageUp, KeyModifiers::NONE), Action::PageUp),
            (ev(KeyCode::PageDown, KeyModifiers::NONE), Action::PageDown),
            (ev(KeyCode::PageUp, action_mod), Action::JumpStart),
            (ev(KeyCode::PageDown, action_mod), Action::JumpEnd),
            (ev(KeyCode::Left, ctrl_arrow), Action::WordLeft),
            (ev(KeyCode::Right, ctrl_arrow), Action::WordRight),
            (ev(KeyCode::Tab, KeyModifiers::NONE), Action::NextBookmark),
            (
                ev(KeyCode::BackTab, KeyModifiers::NONE),
                Action::PrevBookmark,
            ),
        ];
        for (event, action) in cases {
            assert_eq!(km.action_for(&event), Some(action), "event {event:?}");
        }

        // Bare cursor keys stay unbound (fall through to the intrinsic arms).
        assert_eq!(km.action_for(&ev(KeyCode::Left, KeyModifiers::NONE)), None);
        assert_eq!(km.action_for(&ev(KeyCode::Right, KeyModifiers::NONE)), None);
    }

    #[test]
    fn from_config_override_replaces_default() {
        let km = Keymap::from_config(Some(&map(&[("record", "Ctrl+G")]))).unwrap();
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('g'), KeyModifiers::CONTROL)),
            Some(Action::Record)
        );
        // The old default no longer triggers record...
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            None
        );
        // ...but every other default is untouched.
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Action::Copy)
        );
    }

    #[test]
    fn from_config_none_is_defaults() {
        let km = Keymap::from_config(None).unwrap();
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(Action::Record)
        );
    }

    #[test]
    fn from_config_full_swap_is_allowed() {
        let km =
            Keymap::from_config(Some(&map(&[("record", "Ctrl+S"), ("save", "Ctrl+R")]))).unwrap();
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            Some(Action::Record)
        );
        assert_eq!(
            km.action_for(&ev(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(Action::Save)
        );
    }

    #[test]
    fn from_config_unknown_action_is_error() {
        let err = Keymap::from_config(Some(&map(&[("recrod", "Ctrl+J")]))).unwrap_err();
        assert!(err.contains("unknown shortcut action"), "{err}");
        assert!(err.contains("record"), "should list valid actions: {err}");
    }

    #[test]
    fn from_config_bad_binding_is_error() {
        let err = Keymap::from_config(Some(&map(&[("copy", "Cmd+C")]))).unwrap_err();
        assert!(err.contains("invalid key binding"), "{err}");
    }

    #[test]
    fn from_config_reserved_key_is_error() {
        for (action, spec) in [("copy", "Enter"), ("record", "a"), ("clear", "Ctrl+K")] {
            let err = Keymap::from_config(Some(&map(&[(action, spec)]))).unwrap_err();
            assert!(err.contains("cannot be bound to"), "{action}={spec}: {err}");
        }
    }

    #[test]
    fn from_config_duplicate_binding_is_error() {
        // Moving `save` onto `record`'s default without moving `record` collides.
        let err = Keymap::from_config(Some(&map(&[("save", "Ctrl+R")]))).unwrap_err();
        assert!(err.contains("both bound to the same key"), "{err}");
    }
}
