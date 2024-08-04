use std::{sync::Arc, time::Instant};

use mlua::{Table, Value};
use std::time::Duration;

use crate::infra::LogLevel;

#[derive(Clone)]
pub struct PluginMethodMessage<T: Clone> {
    pub plugin_name: Arc<String>,
    pub method_id: u64,
    pub data: T,
}

#[derive(Debug)]
pub enum PluginRequest {
    Internal(PluginInternalRequest),
    External(PluginExternalRequest),
}

#[derive(Clone, Debug)]
pub enum PluginExternalRequest {
    Finish { fn_name: Arc<String> },
    SerialInfo,
    SerialSend { message: Vec<u8> },
    SerialRecv { timeout: Instant },
    Log { level: LogLevel, message: String },
}

#[derive(Debug)]
pub enum PluginInternalRequest {
    SysSleep {
        time: Duration,
    },
    ReLiteral {
        string: String,
    },
    ReMatches {
        string: String,
        pattern_table: Vec<String>,
    },
    ReMatch {
        string: String,
        pattern: String,
    },
    ShellRun {
        cmd: String,
    },
    ShellExist {
        program: String,
    },
}

#[derive(Clone, Debug)]
pub enum PluginResponse {
    Log,
    SerialInfo { port: String, baudrate: u32 },
    SerialSend,
    SerialRecv { err: String, message: Vec<u8> },
    SysSleep,
    ReLiteral { literal: String },
    ReMatches { pattern: Option<String> },
    ReMatch { is_match: bool },
    ShellRun { stdout: String, stderr: String },
    ShellExist { exist: bool },
}

impl<'lua> TryFrom<Table<'lua>> for PluginRequest {
    type Error = String;

    fn try_from(value: Table<'lua>) -> Result<Self, Self::Error> {
        let req_id: String = value
            .get(1)
            .map_err(|_| "Cannot get first table entry as String".to_string())?;

        let req = match req_id.as_str() {
            ":log.debug" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Debug,
                    message,
                })
            }
            ":log.info" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Info,
                    message,
                })
            }
            ":log.success" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Success,
                    message,
                })
            }
            ":log.warning" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Warning,
                    message,
                })
            }
            ":log.error" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Error,
                    message,
                })
            }
            ":serial.info" => PluginRequest::External(PluginExternalRequest::SerialInfo),
            ":serial.send" => {
                let message: Vec<u8> = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::SerialSend { message })
            }
            ":serial.recv" => {
                let opts: Table = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as Table".to_string())?;

                let timeout_ms = opts.get("timeout_ms").unwrap_or(u64::MAX);

                PluginRequest::External(PluginExternalRequest::SerialRecv {
                    timeout: Instant::now() + Duration::from_millis(timeout_ms),
                })
            }
            ":sys.sleep" => {
                let time: u64 = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as Number".to_string())?;

                PluginRequest::Internal(PluginInternalRequest::SysSleep {
                    time: Duration::from_millis(time),
                })
            }
            ":shell.run" => {
                let cmd: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::Internal(PluginInternalRequest::ShellRun { cmd })
            }
            ":shell.exist" => {
                let program: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::Internal(PluginInternalRequest::ShellExist { program })
            }
            ":re.literal" => {
                let string: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::Internal(PluginInternalRequest::ReLiteral { string })
            }
            ":re.matches" => {
                let string: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;
                let pattern_table: Table<'_> = value
                    .get(3)
                    .map_err(|_| "Cannot get third table entry as String".to_string())?;
                let pattern_table = pattern_table
                    .pairs()
                    .into_iter()
                    .filter_map(|res| res.ok())
                    .map(|(k, _v): (String, Value<'_>)| k)
                    .collect();

                PluginRequest::Internal(PluginInternalRequest::ReMatches {
                    string,
                    pattern_table,
                })
            }
            "re.match" => {
                let string: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;
                let pattern: String = value
                    .get(3)
                    .map_err(|_| "Cannot get third table entry as String".to_string())?;

                PluginRequest::Internal(PluginInternalRequest::ReMatch { string, pattern })
            }
            _ => return Err("Invalid Plugin Request ID".to_string()),
        };

        Ok(req)
    }
}
