use crate::rich_string::RichText;
use crate::text::ViewData;
use chrono::{DateTime, Local};
use ratatui::style::Color;
use std::borrow::Cow;
use std::str::Utf8Chunks;

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

    fn from_utf8_print_invalid(v: &[u8]) -> Cow<'_, str> {
        let mut iter = Utf8Chunks::new(v);

        let chunk = if let Some(chunk) = iter.next() {
            let valid = chunk.valid();
            if chunk.invalid().is_empty() {
                debug_assert_eq!(valid.len(), v.len());
                return Cow::Borrowed(valid);
            }
            chunk
        } else {
            return Cow::Borrowed("");
        };

        let mut res = String::with_capacity(v.len());
        res.push_str(chunk.valid());
        res.extend(chunk.invalid().iter().map(|ch| format!("\\x{:02x}", ch)));

        for chunk in iter {
            res.push_str(chunk.valid());
            res.extend(chunk.invalid().iter().map(|ch| format!("\\x{:02x}", ch)));
        }

        Cow::Owned(res)
    }

    pub fn serialize(&self) -> String {
        let success = " OK";
        let fail = "ERR";

        match self {
            SerialRxData::RxData { timestamp, content } => {
                format!(
                    "[{}|<=| OK]{}",
                    timestamp.format("%H:%M:%S.%3f"),
                    Self::from_utf8_print_invalid(content)
                )
            }
            SerialRxData::TxData {
                timestamp,
                content,
                is_successful,
            } => {
                format!(
                    "[{}|=>|{}]{}",
                    timestamp.format("%H:%M:%S.%3f"),
                    if *is_successful { success } else { fail },
                    content
                )
            }
            SerialRxData::Command {
                timestamp,
                command_name,
                content,
                is_successful,
            } => {
                format!(
                    "[{}|=>|{}|/{}]{}",
                    timestamp.format("%H:%M:%S.%3f"),
                    if *is_successful { success } else { fail },
                    command_name,
                    content
                )
            }
            SerialRxData::HexString {
                timestamp,
                content,
                is_successful,
            } => {
                format!(
                    "[{}|=>|{}]{:?}",
                    timestamp.format("%H:%M:%S.%3f"),
                    if *is_successful { success } else { fail },
                    content
                )
            }
            SerialRxData::Plugin {
                timestamp,
                content,
                is_successful,
                plugin_name,
            } => {
                format!(
                    "[{}| P|{}|!{}]{}",
                    timestamp.format("%H:%M:%S.%3f"),
                    if *is_successful { success } else { fail },
                    plugin_name,
                    content
                )
            }
            SerialRxData::PluginSerialTx {
                timestamp,
                content,
                is_successful,
                plugin_name,
            } => {
                format!(
                    "[{}|=>|{}|!{}]{}",
                    timestamp.format("%H:%M:%S.%3f"),
                    if *is_successful { success } else { fail },
                    plugin_name,
                    Self::from_utf8_print_invalid(content)
                )
            }
        }
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
