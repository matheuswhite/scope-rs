use chrono::{DateTime, Local};

#[derive(Default)]
pub struct TimedBytes {
    pub timestamp: DateTime<Local>,
    pub message: Vec<u8>,
}
