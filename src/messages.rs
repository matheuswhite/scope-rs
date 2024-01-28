use crate::text::ViewData;
use chrono::{DateTime, Local};
use tui::style::Color;

pub enum UserTxData {
    Exit,
    Data(String),
    Command(String, String),
    HexString(Vec<u8>),
    PluginSerialTx(String, Vec<u8>),
    #[allow(unused)]
    File(usize, usize, String, String),
}

#[derive(Clone)]
pub enum SerialRxData {
    Data(DateTime<Local>, String),
    ConfirmData(DateTime<Local>, String),
    ConfirmCommand(DateTime<Local>, String, String),
    ConfirmHexString(DateTime<Local>, Vec<u8>),
    ConfirmFile(DateTime<Local>, usize, usize, String, String),
    Plugin(DateTime<Local>, String, String),
    ConfirmPluginSerialTx(DateTime<Local>, String, Vec<u8>),
    FailPlugin(DateTime<Local>, String, String),
    FailData(DateTime<Local>, String),
    FailCommand(DateTime<Local>, String, String),
    FailHexString(DateTime<Local>, Vec<u8>),
    FailFile(DateTime<Local>, usize, usize, String, String),
    FailPluginSerialTx(DateTime<Local>, String, Vec<u8>),
}

impl SerialRxData {
    pub fn is_plugin_serial_tx(&self) -> bool {
        matches!(
            self,
            SerialRxData::ConfirmPluginSerialTx(..) | SerialRxData::FailPluginSerialTx(..)
        )
    }
}

#[allow(clippy::from_over_into)]
impl<'a> Into<ViewData<'a>> for SerialRxData {
    fn into(self) -> ViewData<'a> {
        match self {
            SerialRxData::Data(timestamp, content) => {
                ViewData::new(timestamp, content, Color::Reset, Color::Reset)
            }
            SerialRxData::ConfirmData(timestamp, content) => {
                ViewData::new(timestamp, content, Color::Black, Color::LightCyan)
            }
            SerialRxData::ConfirmCommand(timestamp, cmd_name, content) => ViewData::new(
                timestamp,
                format!("</{}> {}", cmd_name, content),
                Color::Black,
                Color::LightGreen,
            ),
            SerialRxData::ConfirmHexString(timestamp, bytes) => ViewData::new(
                timestamp,
                format!("{:02x?}", &bytes),
                Color::Black,
                Color::Yellow,
            ),
            SerialRxData::ConfirmFile(timestamp, idx, total, filename, content) => ViewData::new(
                timestamp,
                format!("{}[{}/{}]: <{}>", filename, idx, total, content),
                Color::Black,
                Color::LightMagenta,
            ),
            SerialRxData::Plugin(timestamp, plugin_name, message) => ViewData::new(
                timestamp,
                format!(" [{plugin_name}] {message} "),
                Color::Black,
                Color::White,
            ),
            SerialRxData::ConfirmPluginSerialTx(timestamp, plugin_name, message) => ViewData::new(
                timestamp,
                format!(" [{plugin_name}] => {:02x?} ", message),
                Color::Black,
                Color::White,
            ),
            SerialRxData::FailData(timestamp, content) => ViewData::new(
                timestamp,
                format!("Cannot send \"{}\"", content),
                Color::White,
                Color::LightRed,
            ),
            SerialRxData::FailCommand(timestamp, cmd_name, _content) => ViewData::new(
                timestamp,
                format!("Cannot send </{}>", cmd_name),
                Color::White,
                Color::LightRed,
            ),
            SerialRxData::FailHexString(timestamp, bytes) => ViewData::new(
                timestamp,
                format!("Cannot send {:02x?}", &bytes),
                Color::White,
                Color::LightRed,
            ),
            SerialRxData::FailFile(timestamp, idx, total, filename, content) => ViewData::new(
                timestamp,
                format!("Cannot send {}[{}/{}]: <{}>", filename, idx, total, content),
                Color::White,
                Color::LightRed,
            ),
            SerialRxData::FailPlugin(timestamp, plugin_name, message) => ViewData::new(
                timestamp,
                format!(" [{plugin_name}] {message} "),
                Color::White,
                Color::Red,
            ),
            SerialRxData::FailPluginSerialTx(timestamp, pluging_name, message) => ViewData::new(
                timestamp,
                format!(" [{pluging_name}] => Fail to send {:02x?} ", message),
                Color::White,
                Color::Red,
            ),
        }
    }
}
