use std::ops::AddAssign;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

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

    pub async fn flush(&mut self) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.get_filename())
            .await
            .map_err(|err| err.to_string())?;

        let content = self.contents.drain(..).collect::<Vec<_>>().join("");

        file.write_all(content.as_bytes())
            .await
            .map_err(|err| err.to_string())?;

        Ok(())
    }
}

impl AddAssign<String> for TypeWriter {
    fn add_assign(&mut self, rhs: String) {
        let rhs = if !rhs.ends_with('\n') {
            rhs + "\r\n"
        } else {
            rhs
        };

        self.file_size += rhs.len() as u128;
        self.contents.push(rhs);
    }
}
