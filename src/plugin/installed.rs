//! The persistent list of *installed* plugins (issue #36).
//!
//! Loading a plugin with `!plugin load` is per-session; **installing** one with
//! `!plugin install` records its name in a small manifest so every subsequent
//! `scope` session auto-loads it at start-up. The manifest is a TOML file,
//! `installed.toml`, kept in the same directory scope stages loaded plugins
//! into (`<config_dir>/scope/plugins/`), next to the copied `.lua` files and the
//! bundled standard library.
//!
//! ```toml
//! plugins = ["analytics", "auto_test"]
//! ```
//!
//! This is program-managed state (not the user-owned `config.toml`), so a
//! missing manifest is simply "nothing installed" and never an error. A
//! malformed manifest *is* reported, but the caller decides whether that is
//! fatal (at start-up it is logged and skipped so a corrupt file can't brick the
//! app; an explicit `!plugin install` surfaces it).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const MANIFEST_FILE: &str = "installed.toml";

/// The set of installed plugin names, persisted to `installed.toml`.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Installed {
    /// Installed plugin names (without the `.lua` extension), in install order.
    #[serde(default)]
    plugins: Vec<String>,
}

impl Installed {
    /// The manifest path inside the plugins directory `dir`.
    fn path(dir: &Path) -> PathBuf {
        dir.join(MANIFEST_FILE)
    }

    /// Read the manifest from `dir`. A missing file yields an empty list; an
    /// unreadable or malformed file is an error.
    pub fn load(dir: &Path) -> Result<Installed, String> {
        let path = Self::path(dir);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Installed::default());
            }
            Err(err) => {
                return Err(format!(
                    "Cannot read plugin manifest at {:?}: {}",
                    path, err
                ));
            }
        };

        toml::from_str(&contents)
            .map_err(|err| format!("Cannot parse plugin manifest at {:?}: {}", path, err))
    }

    /// The installed plugin names, in install order.
    pub fn names(&self) -> &[String] {
        &self.plugins
    }

    /// Whether `name` is already installed.
    pub fn contains(&self, name: &str) -> bool {
        self.plugins.iter().any(|p| p == name)
    }

    /// Record `name` as installed. Returns `true` if it was newly added, or
    /// `false` if it was already present (no duplicate is created).
    pub fn add(&mut self, name: &str) -> bool {
        if self.contains(name) {
            return false;
        }
        self.plugins.push(name.to_string());
        true
    }

    /// Remove `name` from the installed set. Returns `true` if it was present
    /// (and thus removed), or `false` if it wasn't installed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|p| p != name);
        self.plugins.len() != before
    }

    /// Persist the manifest to `dir`, creating the directory if needed.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("Cannot create plugins directory {:?}: {}", dir, err))?;
        let path = Self::path(dir);
        let contents = toml::to_string(self)
            .map_err(|err| format!("Cannot serialize plugin manifest: {}", err))?;
        std::fs::write(&path, contents)
            .map_err(|err| format!("Cannot write plugin manifest at {:?}: {}", path, err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "scope-installed-test-{}-{}",
                std::process::id(),
                tag
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_manifest_is_empty() {
        let tmp = TempDir::new("missing");
        let installed = Installed::load(tmp.path()).unwrap();
        assert!(installed.names().is_empty());
    }

    #[test]
    fn add_dedupes_and_reports_novelty() {
        let mut installed = Installed::default();
        assert!(installed.add("analytics"));
        assert!(!installed.add("analytics"));
        assert!(installed.add("auto_test"));
        assert_eq!(installed.names(), ["analytics", "auto_test"]);
    }

    #[test]
    fn remove_reports_presence_and_deletes() {
        let mut installed = Installed::default();
        installed.add("analytics");
        installed.add("auto_test");
        assert!(installed.remove("analytics"));
        assert!(!installed.remove("analytics")); // already gone
        assert!(!installed.remove("never_there"));
        assert_eq!(installed.names(), ["auto_test"]);
    }

    #[test]
    fn save_then_load_round_trips() {
        let tmp = TempDir::new("roundtrip");
        let mut installed = Installed::default();
        installed.add("analytics");
        installed.add("auto_test");
        installed.save(tmp.path()).unwrap();

        let reloaded = Installed::load(tmp.path()).unwrap();
        assert_eq!(reloaded.names(), ["analytics", "auto_test"]);
        assert!(reloaded.contains("analytics"));
        assert!(!reloaded.contains("nope"));
    }

    #[test]
    fn save_creates_missing_directory() {
        let tmp = TempDir::new("mkdir");
        let nested = tmp.path().join("a").join("b");
        let mut installed = Installed::default();
        installed.add("analytics");
        installed.save(&nested).unwrap();
        assert_eq!(Installed::load(&nested).unwrap().names(), ["analytics"]);
    }

    #[test]
    fn malformed_manifest_is_error() {
        let tmp = TempDir::new("malformed");
        std::fs::write(Installed::path(tmp.path()), "plugins = not_a_list\n").unwrap();
        let err = Installed::load(tmp.path()).unwrap_err();
        assert!(err.contains("Cannot parse plugin manifest"), "{err}");
    }
}
