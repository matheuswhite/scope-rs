use std::fs::OpenOptions;
use std::io::Write;
use std::ops::AddAssign;

pub struct Storage {
    contents: Vec<String>,
    filename: String,
    suffix: Option<usize>,
    file_size: u128,
}

impl Storage {
    pub fn new(filename: String) -> Self {
        Self {
            contents: vec![],
            filename,
            suffix: None,
            file_size: 0,
        }
    }

    #[allow(unused)]
    pub fn new_file(&mut self) {
        let _ = self.suffix.insert(self.suffix.unwrap_or(0) + 1);
        self.contents.clear();
        self.file_size = 0;
    }

    pub fn get_filename(&self) -> String {
        match self.suffix {
            None => self.filename.clone(),
            Some(suffix) => format!("{}_{}", self.filename, suffix),
        }
    }

    pub fn get_size(&self) -> String {
        let size = self.file_size;
        let (size, unit) = match size {
            x if x < 1024 => return format!("{} Bytes", size),
            x if x < 1024 * 1024 => (size as f32 / 1024.0, "KB"),
            x if x < 1024 * 1024 * 1024 => (size as f32 / (1024.0 * 1024.0), "MB"),
            _ => (size as f32 / (1024.0 * 1024.0 * 1024.0), "GB"),
        };

        format!("{:.1} {}", size, unit)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.get_filename())
            .map_err(|err| err.to_string())?;

        let content = self
            .contents
            .drain(..)
            .map(|line| {
                if !line.ends_with('\n') {
                    line + "\r\n"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("");

        file.write(content.as_bytes())
            .map_err(|err| err.to_string())?;

        Ok(())
    }
}

impl AddAssign<String> for Storage {
    fn add_assign(&mut self, rhs: String) {
        self.file_size += rhs.len() as u128;
        self.contents.push(rhs);
    }
}
