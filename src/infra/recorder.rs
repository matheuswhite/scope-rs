use std::{fs::File, io::Write, path::PathBuf};

pub struct Recorder {
    base_filename: String,
    suffix: usize,
    file_handler: Option<File>,
    file_size: u128,
}

impl Recorder {
    pub fn new(filename: String) -> Result<Self, String> {
        Ok(Self {
            base_filename: Self::base_name(&filename)?,
            suffix: 1,
            file_handler: None,
            file_size: 0,
        })
    }

    /// Strip any directory and extension to get the bare record base name.
    fn base_name(filename: &str) -> Result<String, String> {
        Ok(PathBuf::from(filename)
            .with_extension("")
            .file_name()
            .ok_or("Cannot get filename to record")?
            .to_str()
            .ok_or("Cannot convert record filename to string")?
            .to_string())
    }

    /// Change the base name used for subsequent recordings. An in-progress
    /// recording keeps writing to its already-open file under the old name.
    pub fn rename(&mut self, filename: &str) -> Result<(), String> {
        self.base_filename = Self::base_name(filename)?;
        Ok(())
    }

    pub fn get_filename(&self) -> String {
        format!("{}_rec{}.txt", self.base_filename, self.suffix)
    }

    pub fn get_size(&self) -> u128 {
        self.file_size
    }

    pub fn is_recording(&self) -> bool {
        self.file_handler.is_some()
    }

    pub fn start_record(&mut self) -> Result<(), String> {
        let file = File::create(self.get_filename()).map_err(|err| err.to_string())?;
        self.file_handler = Some(file);

        Ok(())
    }

    pub fn stop_record(&mut self) {
        if let Some(file) = self.file_handler.take() {
            drop(file);
        }

        self.suffix += 1;
        self.file_size = 0;
    }

    pub fn add_bulk_content(&mut self, contents: Vec<String>) -> Result<(), String> {
        let Some(file) = self.file_handler.as_mut() else {
            return Err("No file recording now".to_string());
        };

        for content in contents {
            let content = if !content.ends_with('\n') {
                content.to_string() + "\r\n"
            } else {
                content.to_string()
            };

            file.write_all(content.as_bytes())
                .map_err(|err| err.to_string())?;
            self.file_size += content.len() as u128;
        }
        Ok(())
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(file) = self.file_handler.take() {
            drop(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_strips_directory_and_extension() {
        let recorder = Recorder::new("logs/session.txt".to_string()).unwrap();
        assert_eq!(recorder.get_filename(), "session_rec1.txt");
    }

    #[test]
    fn rename_changes_base_for_future_records() {
        let mut recorder = Recorder::new("foo.txt".to_string()).unwrap();
        assert_eq!(recorder.get_filename(), "foo_rec1.txt");

        recorder.rename("bar").unwrap();
        assert_eq!(recorder.get_filename(), "bar_rec1.txt");

        // An extension on the new name is stripped just like in `new`.
        recorder.rename("baz.txt").unwrap();
        assert_eq!(recorder.get_filename(), "baz_rec1.txt");
    }
}
