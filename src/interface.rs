use chrono::{DateTime, Local};
use std::sync::mpsc::TryRecvError;
use tui::style::Color;

pub enum DataIn {
    Exit,
    Data(String),
    Command(String, String),
}

#[derive(Clone)]
pub enum DataOut {
    Data(DateTime<Local>, String),
    ConfirmData(DateTime<Local>, String),
    ConfirmCommand(DateTime<Local>, String, String),
    FailData(DateTime<Local>, String),
    FailCommand(DateTime<Local>, String, String),
}

#[allow(drop_bounds)]
pub trait Interface: Drop {
    fn is_connected(&self) -> bool;
    fn send(&self, data: DataIn);
    fn try_recv(&self) -> Result<DataOut, TryRecvError>;
    fn description(&self) -> String;
    fn color(&self) -> Color;
}
