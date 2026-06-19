use std::{fs::OpenOptions, io::Write, ops::AddAssign, path::Path};

pub struct TypeWriter {
    contents: Vec<String>,
    filename: String,
    file_size: u128,
}

impl TypeWriter {
    pub fn new(filename: String) -> Self {
        Self {
            contents: vec![],
            filename,
            file_size: 0,
        }
    }

    pub fn get_filename(&self) -> String {
        self.filename.to_string()
    }

    /// Point the session record at `filename`. If the previous file already
    /// exists on disk (it's created lazily on the first flush), it is moved so
    /// the accumulated session isn't left behind under the old name. As a
    /// best-effort guard against clobbering an unrelated file, it errors when
    /// the destination already exists — note this check isn't atomic with the
    /// move, so a concurrent creator could still race it.
    pub fn rename(&mut self, filename: String) -> Result<(), String> {
        if filename == self.filename {
            return Ok(());
        }

        if Path::new(&filename).exists() {
            return Err(format!("\"{}\" already exists", filename));
        }

        if Path::new(&self.filename).exists() {
            std::fs::rename(&self.filename, &filename).map_err(|err| err.to_string())?;
        }

        self.filename = filename;
        Ok(())
    }

    pub fn get_size(&self) -> u128 {
        self.file_size
    }

    pub fn flush(&mut self) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.get_filename())
            .map_err(|err| err.to_string())?;

        let content = self.contents.drain(..).collect::<Vec<_>>().join("");

        file.write_all(content.as_bytes())
            .map_err(|err| err.to_string())?;

        Ok(())
    }
}

impl AddAssign<Vec<String>> for TypeWriter {
    fn add_assign(&mut self, rhs: Vec<String>) {
        for content in rhs {
            let content = if !content.ends_with('\n') {
                content.to_string() + "\r\n"
            } else {
                content.to_string()
            };

            self.file_size += content.len() as u128;
            self.contents.push(content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(suffix: &str) -> String {
        std::env::temp_dir()
            .join(format!("scope_tw_{}_{}", std::process::id(), suffix))
            .to_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn rename_moves_existing_file_and_appends_after() {
        let old = temp_path("old.txt");
        let new = temp_path("new.txt");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&new);

        let mut tw = TypeWriter::new(old.clone());
        tw += vec!["hello".to_string()];
        tw.flush().expect("flush creates the file");
        assert!(Path::new(&old).exists());

        tw.rename(new.clone()).expect("rename");
        assert!(!Path::new(&old).exists(), "old file should be moved");
        assert!(Path::new(&new).exists(), "new file should exist");
        assert_eq!(tw.get_filename(), new);

        tw += vec!["world".to_string()];
        tw.flush().expect("flush appends to the new file");
        let content = std::fs::read_to_string(&new).unwrap();
        assert!(content.contains("hello") && content.contains("world"));

        let _ = std::fs::remove_file(&new);
    }

    #[test]
    fn rename_refuses_to_overwrite_existing_destination() {
        let old = temp_path("clobber_old.txt");
        let dest = temp_path("clobber_dest.txt");
        let _ = std::fs::remove_file(&old);

        let mut tw = TypeWriter::new(old.clone());
        tw += vec!["data".to_string()];
        tw.flush().expect("flush creates the file");
        std::fs::write(&dest, "existing").expect("seed destination");

        assert!(tw.rename(dest.clone()).is_err(), "must not clobber dest");
        assert_eq!(tw.get_filename(), old, "name unchanged on failure");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "existing");
        assert!(Path::new(&old).exists(), "source kept on failure");

        let _ = std::fs::remove_file(&old);
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn rename_before_any_flush_only_updates_name() {
        let mut tw = TypeWriter::new(temp_path("nf_old.txt"));
        let new = temp_path("nf_new.txt");
        tw.rename(new.clone())
            .expect("rename without an existing file");
        assert_eq!(tw.get_filename(), new);
        assert!(!Path::new(&new).exists());
    }
}
