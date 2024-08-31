use std::sync::mpsc::{channel, Receiver, Sender};

use chrono::{DateTime, Local};

#[derive(Clone)]
pub struct Logger {
    sender: Sender<LogMessage>,
    source: String,
    id: Option<String>,
}

pub struct LogMessage {
    pub timestamp: DateTime<Local>,
    pub message: String,
    pub level: LogLevel,
}

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    Error,
    Warning,
    Success,
    Info,
    Debug,
}

impl Logger {
    pub fn new(source: String) -> (Self, Receiver<LogMessage>) {
        let (sender, receiver) = channel();

        (
            Self {
                sender,
                source,
                id: None,
            },
            receiver,
        )
    }

    pub fn with_source(mut self, source: String) -> Self {
        self.source = source;
        self
    }

    pub fn with_id(mut self, id: String) -> Self {
        self.id = Some(id);
        self
    }

    pub fn write(
        &self,
        message: String,
        level: LogLevel,
    ) -> Result<(), std::sync::mpsc::SendError<LogMessage>> {
        self.sender.send(LogMessage {
            timestamp: Local::now(),
            message: format!(
                "[{}{}] {}",
                self.source,
                self.id
                    .as_ref()
                    .map(|id| ":".to_string() + id)
                    .unwrap_or("".to_string()),
                message
            ),
            level,
        })
    }

    pub fn write_with_source_id(
        &self,
        message: String,
        level: LogLevel,
        source: String,
        id: String,
    ) -> Result<(), std::sync::mpsc::SendError<LogMessage>> {
        self.sender.send(LogMessage {
            timestamp: Local::now(),
            message: format!("[{}:{}] {}", source, id, message),
            level,
        })
    }
}

#[macro_export]
macro_rules! debug {
    ($logger:expr, $($arg:tt)+) => {
        {let _ = $logger.write(format!($($arg)+), LogLevel::Debug);}
    };
}

#[macro_export]
macro_rules! info {
    ($logger:expr, $($arg:tt)+) => {
        {let _ = $logger.write(format!($($arg)+), LogLevel::Info);}
    };
}

#[macro_export]
macro_rules! success {
    ($logger:expr, $($arg:tt)+) => {
        {let _ = $logger.write(format!($($arg)+), LogLevel::Success);}
    };
}

#[macro_export]
macro_rules! warning {
    ($logger:expr, $($arg:tt)+) => {
        {let _ = $logger.write(format!($($arg)+), LogLevel::Warning);}
    };
}

#[macro_export]
macro_rules! error {
    ($logger:expr, $($arg:tt)+) => {
        {let _ = $logger.write(format!($($arg)+), LogLevel::Error);}
    };
}
