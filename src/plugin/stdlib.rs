//! The Lua standard library bundled with scope (`require("scope")`,
//! `require("shell")`, `require("codec.at")`).

use std::path::Path;

/// Each module as `(path relative to the plugins directory, contents)`.
pub const STDLIB: &[(&str, &str)] = &[
    ("scope.lua", include_str!("../../plugins/scope.lua")),
    ("shell.lua", include_str!("../../plugins/shell.lua")),
];

/// Writes every module into `dir` whose copy there is missing or differs, so
/// the plugins always run against the library of this scope version.
pub fn provision(dir: &Path) -> Result<(), String> {
    for (name, contents) in STDLIB {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|current| current == contents.as_bytes()) {
            continue;
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Cannot create {:?}: {}", parent, err))?;
        }
        std::fs::write(&path, contents)
            .map_err(|err| format!("Cannot write {:?}: {}", path, err))?;
    }

    Ok(())
}
