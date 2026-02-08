use crate::{
    graphics::{Serialize, screen::ScreenDecoder},
    infra::LogLevel,
};
use chrono::{DateTime, Local};
use std::ops::AddAssign;

pub struct Buffer {
    lines: Vec<BufferLine<Vec<u8>>>,
    capacity: usize,
}

impl Buffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: Vec::new(),
            capacity,
        }
    }

    pub fn get_range(&self, start: usize, end: usize) -> &[BufferLine<Vec<u8>>] {
        let end = end.min(self.lines.len());
        let start = start.min(end);

        &self.lines[start..end]
    }

    pub fn iter(&self) -> impl Iterator<Item = &BufferLine<Vec<u8>>> {
        self.lines.iter()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }

    fn drop_oldest_if_needed(&mut self) {
        if self.lines.len() == self.capacity {
            self.lines.remove(0);
        }

        for (index, line) in self.lines.iter_mut().enumerate() {
            line.line = index;
        }
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
}

impl AddAssign<BufferLine<Vec<u8>>> for Buffer {
    fn add_assign(&mut self, mut rhs: BufferLine<Vec<u8>>) {
        self.drop_oldest_if_needed();

        rhs.line = self.lines.len();
        self.lines.push(rhs);
    }
}

impl AddAssign<Vec<BufferLine<Vec<u8>>>> for Buffer {
    fn add_assign(&mut self, mut rhs: Vec<BufferLine<Vec<u8>>>) {
        for line in rhs.drain(..) {
            *self += line;
        }
    }
}

pub struct BufferLine<T>
where
    T: AsRef<[u8]>,
{
    pub line: usize,
    pub timestamp: DateTime<Local>,
    pub level: Option<LogLevel>,
    pub message: T,
    pub is_tx: bool,
}

impl BufferLine<Vec<u8>> {
    pub fn decode(&self, decoder: ScreenDecoder) -> BufferLine<String> {
        BufferLine {
            line: self.line,
            timestamp: self.timestamp,
            level: self.level,
            message: decoder.decode(&self.message),
            is_tx: self.is_tx,
        }
    }

    pub fn new_rx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: None,
            message,
            is_tx: false,
        }
    }

    pub fn new_tx(timestamp: DateTime<Local>, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: None,
            message,
            is_tx: true,
        }
    }

    pub fn new_log(timestamp: DateTime<Local>, level: LogLevel, message: Vec<u8>) -> Self {
        Self {
            line: 0,
            timestamp,
            level: Some(level),
            message,
            is_tx: false,
        }
    }

    pub fn timestamp(&self) -> DateTime<Local> {
        self.timestamp
    }
}

impl Serialize for BufferLine<Vec<u8>> {
    fn serialize(&self) -> String {
        let message = ScreenDecoder::Ascii.decode(&self.message);

        if let Some(level) = self.level {
            let log_level = match level {
                LogLevel::Error => "ERR",
                LogLevel::Warning => "WRN",
                LogLevel::Success => " OK",
                LogLevel::Info => "INF",
                LogLevel::Debug => "DBG",
            };

            return format!(
                "[{}][{}] {}",
                timestamp_fmt(self.timestamp),
                log_level,
                message
            );
        }

        if self.is_tx {
            format!("[{}][ =>] {}", timestamp_fmt(self.timestamp), message)
        } else {
            format!("[{}][ <=] {}", timestamp_fmt(self.timestamp), message)
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct BufferPosition {
    pub line: usize,
    pub column: usize,
}

pub fn timestamp_fmt(timestamp: DateTime<Local>) -> String {
    timestamp.format("%H:%M:%S.%3f").to_string()
}
