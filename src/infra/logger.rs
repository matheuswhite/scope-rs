use std::sync::mpsc::{channel, Receiver, Sender};

use chrono::{DateTime, Local};

#[derive(Clone)]
pub struct Logger {
    sender: Sender<LogMessage>,
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
    pub fn new() -> (Self, Receiver<LogMessage>) {
        let (sender, receiver) = channel();

        (Self { sender }, receiver)
    }

    pub fn write(
        &self,
        message: String,
        level: LogLevel,
    ) -> Result<(), std::sync::mpsc::SendError<LogMessage>> {
        self.sender.send(LogMessage {
            timestamp: Local::now(),
            message,
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
