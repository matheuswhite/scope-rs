use std::{fs::File, io::Write, path::PathBuf};

use super::into_byte_format;

pub struct Recorder {
    base_filename: String,
    suffix: usize,
    file_handler: Option<File>,
    file_size: u128,
}

impl Recorder {
    pub fn new(filename: String) -> Result<Self, String> {
        let filename = PathBuf::from(filename)
            .with_extension("")
            .file_name()
            .ok_or("Cannot get filename to record")?
            .to_str()
            .ok_or("Cannot convert record filename to string")?
            .to_string();

        Ok(Self {
            base_filename: filename,
            suffix: 1,
            file_handler: None,
            file_size: 0,
        })
    }

    pub fn get_filename(&self) -> String {
        format!("{}_rec{}.txt", self.base_filename, self.suffix)
    }

    pub fn get_size(&self) -> String {
        into_byte_format(self.file_size)
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
