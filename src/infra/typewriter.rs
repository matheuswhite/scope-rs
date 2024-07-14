use std::{fs::OpenOptions, io::Write, ops::AddAssign};

use super::into_byte_format;

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

    pub fn get_size(&self) -> String {
        into_byte_format(self.file_size)
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
