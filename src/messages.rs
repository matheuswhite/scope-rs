use crate::rich_string::RichText;
use crate::text::ViewData;
use chrono::{DateTime, Local};
use ratatui::style::Color;

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
        content: Vec<u8>,
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
                RichText::new(content, Color::Reset, Color::Reset)
                    .decode_ansi_color()
                    .highlight_invisible()
                    .into_view_data(timestamp)
            }
            SerialRxData::TxData {
                timestamp,
                content,
                is_successful,
            } => {
                if is_successful {
                    RichText::from_string(content, Color::Black, Color::LightCyan)
                        .highlight_invisible()
                        .into_view_data(timestamp)
                } else {
                    RichText::from_string(
                        format!("Cannot send \"{}\"", content),
                        Color::White,
                        Color::LightRed,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                }
            }
            SerialRxData::Command {
                timestamp,
                command_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    RichText::from_string(
                        format!("</{}> {}", command_name, content),
                        Color::Black,
                        Color::LightGreen,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                } else {
                    RichText::from_string(
                        format!("Cannot send </{}>", command_name),
                        Color::White,
                        Color::LightRed,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                }
            }
            SerialRxData::HexString {
                timestamp,
                content,
                is_successful,
            } => {
                if is_successful {
                    RichText::from_string(format!("{:02x?}", &content), Color::Black, Color::Yellow)
                        .highlight_invisible()
                        .into_view_data(timestamp)
                } else {
                    RichText::from_string(
                        format!("Cannot send {:02x?}", &content),
                        Color::White,
                        Color::LightRed,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                }
            }
            SerialRxData::Plugin {
                timestamp,
                plugin_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    RichText::from_string(
                        format!(" [{plugin_name}] {content} "),
                        Color::Black,
                        Color::White,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                } else {
                    RichText::from_string(
                        format!(" [{plugin_name}] {content} "),
                        Color::White,
                        Color::Red,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                }
            }
            SerialRxData::PluginSerialTx {
                timestamp,
                plugin_name,
                content,
                is_successful,
            } => {
                if is_successful {
                    RichText::from_string(
                        format!(" [{plugin_name}] => {} ", String::from_utf8_lossy(&content)),
                        Color::Black,
                        Color::White,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                } else {
                    RichText::from_string(
                        format!(
                            " [{plugin_name}] => Fail to send {} ",
                            String::from_utf8_lossy(&content)
                        ),
                        Color::White,
                        Color::Red,
                    )
                    .highlight_invisible()
                    .into_view_data(timestamp)
                }
            }
        }
    }
}
