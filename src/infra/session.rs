use chrono::Local;

/// Normalize a user-supplied session name into a bare base name (no `.txt`
/// extension, no directory). Trims whitespace, strips a trailing `.txt` so it
/// isn't doubled later, and rejects empty names or ones containing path
/// components — a session must stay a single file in the working directory.
pub fn sanitize_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    let name = name.strip_suffix(".txt").unwrap_or(name);

    if name.is_empty() {
        return Err("session name cannot be empty".to_string());
    }

    if name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err(format!("invalid session name \"{}\"", raw));
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
    }

    #[test]
    fn sanitize_keeps_inner_dots() {
        assert_eq!(sanitize_name("foo.bar").unwrap(), "foo.bar");
        assert_eq!(sanitize_name("foo.bar.txt").unwrap(), "foo.bar");
    }

    #[test]
    fn sanitize_trims_whitespace() {
        assert_eq!(sanitize_name("  log  ").unwrap(), "log");
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
