use chrono::Local;

/// Windows reserved device names (matched case-insensitively, with or without
/// an extension); also invalid as plain file names there.
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Normalize a user-supplied session name into a bare base name (no `.txt`
/// extension, no directory). Strips trailing whitespace and `.txt` extensions
/// repeatedly (stripping can expose more of either, e.g. `"foo .txt"` -> `foo`,
/// `foo.txt.txt` -> `foo`). The result is kept portable: empty names, path
/// separators, characters/patterns invalid on Windows, and reserved device
/// names are rejected so creating the file can't fail later with a confusing OS
/// error.
pub fn sanitize_name(raw: &str) -> Result<String, String> {
    let mut name = raw.trim();
    loop {
        let trimmed = name.trim_end();
        match trimmed.strip_suffix(".txt") {
            Some(rest) => name = rest,
            None => {
                name = trimmed;
                break;
            }
        }
    }

    if name.is_empty() {
        return Err("session name cannot be empty".to_string());
    }

    if name == "." || name == ".." {
        return Err(format!("invalid session name \"{}\"", raw));
    }

    if name.chars().any(|c| {
        matches!(c, '/' | '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*') || c.is_control()
    }) {
        return Err(format!("session name \"{}\" has invalid characters", raw));
    }

    if name.ends_with('.') {
        return Err(format!("session name \"{}\" cannot end with a dot", raw));
    }

    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        return Err(format!("session name \"{}\" is reserved", raw));
    }

    Ok(name.to_string())
}

/// Build the session record file name: `<name>.txt` for a (sanitized) name, or
/// a timestamped name (`%Y%m%d_%H%M%S.txt`) when none is given.
pub fn record_filename(name: Option<&str>) -> String {
    match name {
        Some(name) => format!("{name}.txt"),
        None => format!("{}.txt", Local::now().format("%Y%m%d_%H%M%S")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_trailing_txt() {
        assert_eq!(sanitize_name("capture.txt").unwrap(), "capture");
        assert_eq!(sanitize_name("capture").unwrap(), "capture");
        // Repeated `.txt` is fully stripped so the suffix isn't doubled later.
        assert_eq!(sanitize_name("capture.txt.txt").unwrap(), "capture");
    }

    #[test]
    fn sanitize_keeps_inner_dots() {
        assert_eq!(sanitize_name("foo.bar").unwrap(), "foo.bar");
        assert_eq!(sanitize_name("foo.bar.txt").unwrap(), "foo.bar");
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_name("  log  ").unwrap(), "log");
        // Stripping `.txt` can expose trailing whitespace, which is trimmed too.
        assert_eq!(sanitize_name("foo .txt").unwrap(), "foo");
    }

    #[test]
    fn sanitize_rejects_windows_invalid_chars_and_trailing_dot() {
        for bad in ["a:b", "a*b", "a?b", "a\"b", "a|b", "a<b", "a>b"] {
            assert!(sanitize_name(bad).is_err(), "{bad} should be rejected");
        }
        assert!(sanitize_name("trailing.").is_err());
    }

    #[test]
    fn sanitize_rejects_reserved_device_names() {
        assert!(sanitize_name("CON").is_err());
        assert!(sanitize_name("nul").is_err());
        assert!(sanitize_name("CON.txt").is_err());
        assert!(sanitize_name("com1").is_err());
        // Not reserved: the device word only as a later dotted segment.
        assert_eq!(sanitize_name("data.con").unwrap(), "data.con");
    }

    #[test]
    fn sanitize_rejects_empty_and_dotonly() {
        assert!(sanitize_name("").is_err());
        assert!(sanitize_name("   ").is_err());
        assert!(sanitize_name(".txt").is_err());
        assert!(sanitize_name(".").is_err());
        assert!(sanitize_name("..").is_err());
    }

    #[test]
    fn sanitize_rejects_path_separators() {
        assert!(sanitize_name("../escape").is_err());
        assert!(sanitize_name("dir/name").is_err());
        assert!(sanitize_name("dir\\name").is_err());
    }

    #[test]
    fn record_filename_named_and_timestamped() {
        assert_eq!(record_filename(Some("capture")), "capture.txt");

        let timestamped = record_filename(None);
        assert!(timestamped.ends_with(".txt"));
        assert_eq!(timestamped.len(), "YYYYMMDD_HHMMSS.txt".len());
    }
}
