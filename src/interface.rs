use chrono::{DateTime, Local};
use std::sync::mpsc::TryRecvError;
use tui::style::Color;

pub enum DataIn {
    Exit,
    Data(String),
    Command(String, String),
    HexString(Vec<u8>),
}

#[derive(Clone)]
pub enum DataOut {
    Data(DateTime<Local>, String),
    ConfirmData(DateTime<Local>, String),
    ConfirmCommand(DateTime<Local>, String, String),
    ConfirmHexString(DateTime<Local>, Vec<u8>),
    FailData(DateTime<Local>, String),
    FailCommand(DateTime<Local>, String, String),
    FailHexString(DateTime<Local>, Vec<u8>),
}

#[allow(drop_bounds)]
pub trait Interface: Drop {
    fn is_connected(&self) -> bool;
    fn send(&self, data: DataIn);
    fn try_recv(&self) -> Result<DataOut, TryRecvError>;
    fn description(&self) -> String;
    fn color(&self) -> Color;
    fn set_port(&mut self, _port : String);
    fn set_baudrate(&mut self, _baudarate: u32);
}
