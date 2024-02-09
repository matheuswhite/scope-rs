use crate::text::ViewData;
use chrono::{DateTime, Local};
use tui::style::Color;

pub enum UserTxData {
    Exit,
    Data {
        content: String,
    },
    Command {
        command_name: String,
        content: String,
    },
    HexString {
        content: Vec<u8>,
    },
    PluginSerialTx {
        plugin_name: String,
        content: Vec<u8>,
    },
}

#[derive(Clone)]
pub enum SerialRxData {
    RxData {
        timestamp: DateTime<Local>,
        content: String,
    },
    TxData {
        timestamp: DateTime<Local>,
        content: String,
        is_successful: bool,
    },
    Command {
        timestamp: DateTime<Local>,
        command_name: String,
        content: String,
        is_successful: bool,
    },
    HexString {
        timestamp: DateTime<Local>,
        content: Vec<u8>,
        is_successful: bool,
    },
    Plugin {
        timestamp: DateTime<Local>,
        plugin_name: String,
        content: String,
        is_successful: bool,
    },
    PluginSerialTx {
        timestamp: DateTime<Local>,
        plugin_name: String,
        content: Vec<u8>,
        is_successful: bool,
    },
}

impl SerialRxData {
    pub fn is_plugin_serial_tx(&self) -> bool {
        matches!(self, SerialRxData::PluginSerialTx { .. })
    }
}

#[allow(clippy::from_over_into)]
impl Into<ViewData> for SerialRxData {
    fn into(self) -> ViewData {
        match self {
            SerialRxData::RxData { timestamp, content } => {
                ViewData::new(timestamp, content, Color::Reset, Color::Reset)
            }
            SerialRxData::TxData {
                timestamp,
                content,
                is_successful,
            } => {
                if is_successful {
                    ViewData::new(timestamp, content, Color::Black, Color::LightCyan)
                } else {
                    ViewData::new(
                        timestamp,
                        format!("Cannot send \"{}\"", content),
                        Color::White,
                        Color::LightRed,
                    )
                }
            }
            SerialRxData::Command {
                timestamp,
                command_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    ViewData::new(
                        timestamp,
                        format!("</{}> {}", command_name, content),
                        Color::Black,
                        Color::LightGreen,
                    )
                } else {
                    ViewData::new(
                        timestamp,
                        format!("Cannot send </{}>", command_name),
                        Color::White,
                        Color::LightRed,
                    )
                }
            }
            SerialRxData::HexString {
                timestamp,
                content,
                is_successful,
            } => {
                if is_successful {
                    ViewData::new(
                        timestamp,
                        format!("{:02x?}", &content),
                        Color::Black,
                        Color::Yellow,
                    )
                } else {
                    ViewData::new(
                        timestamp,
                        format!("Cannot send {:02x?}", &content),
                        Color::White,
                        Color::LightRed,
                    )
                }
            }
            SerialRxData::Plugin {
                timestamp,
                plugin_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    ViewData::new(
                        timestamp,
                        format!(" [{plugin_name}] {content} "),
                        Color::Black,
                        Color::White,
                    )
                } else {
                    ViewData::new(
                        timestamp,
                        format!(" [{plugin_name}] {content} "),
                        Color::White,
                        Color::Red,
                    )
                }
            }
            SerialRxData::PluginSerialTx {
                timestamp,
                plugin_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    ViewData::new(
                        timestamp,
                        format!(" [{plugin_name}] => {:02x?} ", content),
                        Color::Black,
                        Color::White,
                    )
                } else {
                    ViewData::new(
                        timestamp,
                        format!(" [{plugin_name}] => Fail to send {:02x?} ", content),
                        Color::White,
                        Color::Red,
                    )
                }
            }
        }
    }
}
