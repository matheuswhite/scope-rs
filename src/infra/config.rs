//! Optional user configuration read from `config.toml` in the user's config
//! directory (`~/.config/scope/config.toml` on Linux, alongside the crash
//! backups). Every field is optional and overrides the built-in default, but is
//! itself overridden by an explicit CLI flag — so the precedence is:
//!
//! ```text
//! CLI flag  >  config.toml  >  built-in default
//! ```
//!
//! A missing file (or a missing field) simply falls through to the defaults; a
//! present-but-unreadable or malformed file is a hard error so a typo doesn't
//! silently do nothing.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.toml";

/// User-overridable settings. Field names match the corresponding CLI flags.
/// Unknown keys are rejected so a misspelled option is reported instead of
/// silently ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Scrollback capacity in lines (CLI: `-c/--capacity`).
    pub capacity: Option<usize>,
    /// Path to the tag file (CLI: `-t/--tag-file`). Used verbatim — no shell is
    /// involved, so `~` and `$VAR` are not expanded; use an absolute path.
    pub tag_file: Option<PathBuf>,
    /// Optional shortcut overrides: `action-name = "Key+Combo"` (e.g.
    /// `record = "Ctrl+G"`). There is no CLI flag, so precedence is
    /// config.toml > built-in default; an omitted action keeps its default.
    ///
    /// The action-name vocabulary and combo parsing live in
    /// [`crate::inputs::keymap`] — this module deliberately does not enumerate
    /// them. Unknown *top-level* keys are still rejected by
    /// `deny_unknown_fields`; unknown *action* names (and bad combos, reserved
    /// keys, duplicate bindings) are rejected by `Keymap::from_config`.
    #[serde(default)]
    pub shortcuts: Option<BTreeMap<String, String>>,
    /// How a byte with no text form is displayed (issue #239): `"\\xaa"` (the
    /// default), `"0xAA"`, `"0xaa"` or `"AA"`. Kept a plain string so the value
    /// vocabulary lives with the renderer ([`crate::graphics::screen::HexFormat`]),
    /// which rejects an unknown one. There is no CLI flag.
    pub hex_format: Option<String>,
    /// Optional `[history]` table switching off the files scope writes on its
    /// own, without the user asking (issue #247). There is no CLI flag, so
    /// precedence is config.toml > built-in default (everything saved).
    #[serde(default)]
    pub history: Option<HistoryConfig>,
}

/// The `[history]` table. Each key covers one file that is written
/// automatically; an explicit save (`Ctrl+S`) or `!record` is never affected.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryConfig {
    /// Persist the command-bar history to `<data_dir>/scope/.scope_history`,
    /// so `Up`/`Down` reach commands from earlier runs. When off, the history
    /// still works for the current run, it just lives in memory only.
    pub save_commands: Option<bool>,
    /// Mirror the session (RX, TX and logs) into the crash-recovery
    /// `<config_dir>/scope/backup/<session>.txt.bkp`.
    pub save_backup: Option<bool>,
}

/// What scope may write to disk on its own, resolved from `[history]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Autosave {
    pub commands: bool,
    pub backup: bool,
}

impl Autosave {
    /// Resolve the `[history]` table, an omitted table or key keeping the
    /// built-in default of saving.
    pub fn from_config(history: Option<&HistoryConfig>) -> Self {
        Self {
            commands: history.and_then(|h| h.save_commands).unwrap_or(true),
            backup: history.and_then(|h| h.save_backup).unwrap_or(true),
        }
    }
}

impl Config {
    /// The config file location: `<config_dir>/scope/config.toml`, or `None`
    /// when the platform config directory can't be resolved.
    fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("scope").join(CONFIG_FILE_NAME))
    }

    /// Load the user config, falling back to all-default when the file is absent
    /// or the platform config dir is unknown.
    pub fn load() -> Result<Self, String> {
        match Self::path() {
            Some(path) => Self::load_from(&path),
            None => Ok(Self::default()),
        }
    }

    /// Read and parse the config at `path`. A missing file yields the defaults;
    /// an unreadable or malformed file is an error.
    fn load_from(path: &Path) -> Result<Self, String> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(err) => {
                return Err(format!(
                    "Cannot read config file at {}: {}",
                    path.display(),
                    err
                ));
            }
        };

        toml::from_str(&contents)
            .map_err(|err| format!("Cannot parse config file at {}: {}", path.display(), err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(suffix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scope_cfg_{}_{}", std::process::id(), suffix));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(CONFIG_FILE_NAME)
    }

    #[test]
    fn parses_all_fields() {
        let path = temp_path("all");
        std::fs::write(&path, "capacity = 5000\ntag_file = \"custom.yml\"\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.capacity, Some(5000));
        assert_eq!(config.tag_file, Some(PathBuf::from("custom.yml")));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn missing_fields_stay_none() {
        let path = temp_path("partial");
        std::fs::write(&path, "capacity = 100\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.capacity, Some(100));
        assert_eq!(config.tag_file, None);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn empty_file_is_all_defaults() {
        let path = temp_path("empty");
        std::fs::write(&path, "").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.capacity, None);
        assert_eq!(config.tag_file, None);
        assert!(config.shortcuts.is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_shortcuts_table() {
        let path = temp_path("shortcuts");
        std::fs::write(
            &path,
            "capacity = 100\n\n[shortcuts]\nrecord = \"Ctrl+G\"\nnext_bookmark = \"F2\"\n",
        )
        .unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.capacity, Some(100));
        let shortcuts = config.shortcuts.expect("shortcuts table present");
        assert_eq!(shortcuts.get("record").map(String::as_str), Some("Ctrl+G"));
        assert_eq!(
            shortcuts.get("next_bookmark").map(String::as_str),
            Some("F2")
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn parses_hex_format() {
        let path = temp_path("hex_format");
        std::fs::write(&path, "hex_format = \"0xAA\"\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.hex_format.as_deref(), Some("0xAA"));

        // The documented spelling of the default: a literal string, where the
        // backslash needs no escaping.
        std::fs::write(&path, "hex_format = '\\xaa'\n").unwrap();
        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.hex_format.as_deref(), Some("\\xaa"));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn history_defaults_to_saving_everything() {
        let path = temp_path("history_default");
        std::fs::write(&path, "capacity = 100\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        assert!(config.history.is_none());
        assert_eq!(
            Autosave::from_config(config.history.as_ref()),
            Autosave {
                commands: true,
                backup: true,
            }
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn history_keys_switch_off_each_file() {
        let path = temp_path("history_off");
        std::fs::write(&path, "[history]\nsave_commands = false\n").unwrap();

        let config = Config::load_from(&path).unwrap();
        // An omitted key keeps its default, independently of the other one.
        assert_eq!(
            Autosave::from_config(config.history.as_ref()),
            Autosave {
                commands: false,
                backup: true,
            }
        );

        std::fs::write(
            &path,
            "[history]\nsave_commands = false\nsave_backup = false\n",
        )
        .unwrap();
        let config = Config::load_from(&path).unwrap();
        assert_eq!(
            Autosave::from_config(config.history.as_ref()),
            Autosave {
                commands: false,
                backup: false,
            }
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn unknown_history_key_is_rejected() {
        let path = temp_path("history_unknown");
        std::fs::write(&path, "[history]\nlog_read = false\n").unwrap();

        let err = Config::load_from(&path).unwrap_err();
        assert!(err.contains("Cannot parse config file"), "got: {err}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn absent_file_is_all_defaults() {
        let path = temp_path("absent");
        let _ = std::fs::remove_file(&path);

        let config = Config::load_from(&path).unwrap();
        assert_eq!(config.capacity, None);
        assert_eq!(config.tag_file, None);
    }

    #[test]
    fn unknown_key_is_rejected() {
        let path = temp_path("unknown");
        std::fs::write(&path, "capcity = 5000\n").unwrap();

        let err = Config::load_from(&path).unwrap_err();
        assert!(err.contains("Cannot parse config file"), "got: {err}");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn malformed_toml_is_rejected() {
        let path = temp_path("malformed");
        std::fs::write(&path, "capacity = \n").unwrap();

        let err = Config::load_from(&path).unwrap_err();
        assert!(err.contains("Cannot parse config file"), "got: {err}");

        let _ = std::fs::remove_file(&path);
    }
}
