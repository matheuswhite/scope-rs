use std::{sync::Arc, time::Instant};

use mlua::{Table, Value};
use std::time::Duration;

use super::line_waiter::{WaitResult, WaitSpec};
use crate::{infra::LogLevel, interfaces::InterfaceType};

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
    Finish {
        fn_name: Arc<String>,
    },
    SerialInfo,
    SerialSend {
        message: Vec<u8>,
    },
    SerialRecv {
        timeout: Option<Instant>,
    },
    RttInfo,
    RttSend {
        message: Vec<u8>,
    },
    RttRecv {
        timeout: Option<Instant>,
    },
    RttRead {
        timeout: Option<Instant>,
        plugin_name: Arc<String>,
        method_id: u64,
        address: u64,
        size: usize,
    },
    SendAndWait {
        interface: InterfaceType,
        message: Vec<u8>,
        spec: WaitSpec,
    },
    Log {
        level: LogLevel,
        message: String,
        plugin_name: String,
        id: String,
    },
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
    SerialInfo {
        port: String,
        baudrate: u32,
    },
    SerialSend,
    SerialRecv {
        err: String,
        message: Vec<u8>,
    },
    RttInfo {
        target: String,
        channel: usize,
    },
    RttSend,
    RttRecv {
        err: String,
        message: Vec<u8>,
    },
    RttRead {
        err: String,
        data: Vec<u8>,
    },
    SendAndWait(WaitResult),
    /// The plugin instance that made the call is gone; the call must stop.
    Unloaded,
    SysSleep,
    ReLiteral {
        literal: String,
    },
    ReMatches {
        pattern: Option<String>,
    },
    ReMatch {
        is_match: bool,
    },
    ShellRun {
        stdout: String,
        stderr: String,
    },
    ShellExist {
        exist: bool,
    },
}

impl PluginRequest {
    fn deadline_from_timeout_ms(timeout_ms: Option<u64>) -> Option<Instant> {
        // None (or a very large sentinel) means “no timeout”.
        let timeout_ms = timeout_ms?;
        if timeout_ms == u64::MAX {
            return None;
        }

        Instant::now().checked_add(Duration::from_millis(timeout_ms))
    }

    fn bytes(value: mlua::Result<Value>, what: &str) -> Result<Vec<u8>, String> {
        match value.map_err(|err| format!("Cannot get {what}: {err}"))? {
            Value::String(string) => Ok(string.as_bytes().to_vec()),
            Value::Table(table) => table
                .sequence_values::<u8>()
                .collect::<mlua::Result<_>>()
                .map_err(|err| format!("{what} is not a list of bytes: {err}")),
            other => Err(format!(
                "{what} must be a string or a list of bytes, not {}",
                other.type_name()
            )),
        }
    }

    fn send_and_wait(interface: InterfaceType, value: Table, req_id: &str) -> Result<Self, String> {
        let opts: Table = value
            .get(2)
            .map_err(|_| format!("Cannot get the options table of {req_id}"))?;
        let field = |name: &str, err: mlua::Error| format!("{req_id}: invalid {name}: {err}");

        let message = Self::bytes(opts.get("message"), &format!("{req_id} message"))?;
        let expected: String = opts.get("expected").map_err(|err| field("expected", err))?;
        let timeout_ms: Option<u64> = opts
            .get("timeout_ms")
            .map_err(|err| field("timeout_ms", err))?;
        let error_pattern: Option<String> = opts
            .get("error_pattern")
            .map_err(|err| field("error_pattern", err))?;
        let ignore: Option<Vec<String>> = opts.get("ignore").map_err(|err| field("ignore", err))?;

        Ok(PluginRequest::External(
            PluginExternalRequest::SendAndWait {
                interface,
                message,
                spec: WaitSpec {
                    expected,
                    error_pattern,
                    ignore: ignore.unwrap_or_default(),
                    timeout: timeout_ms
                        .filter(|timeout_ms| *timeout_ms != u64::MAX)
                        .map(Duration::from_millis),
                },
            },
        ))
    }

    pub fn from_table<'lua>(
        value: Table<'lua>,
        plugin_name: String,
        id: String,
        method_id: u64,
    ) -> Result<Self, String> {
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
                    plugin_name,
                    id,
                })
            }
            ":log.info" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Info,
                    message,
                    plugin_name,
                    id,
                })
            }
            ":log.success" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Success,
                    message,
                    plugin_name,
                    id,
                })
            }
            ":log.warning" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Warning,
                    message,
                    plugin_name,
                    id,
                })
            }
            ":log.error" => {
                let message: String = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as String".to_string())?;

                PluginRequest::External(PluginExternalRequest::Log {
                    level: LogLevel::Error,
                    message,
                    plugin_name,
                    id,
                })
            }
            ":serial.info" => PluginRequest::External(PluginExternalRequest::SerialInfo),
            ":serial.send" => {
                let message = Self::bytes(value.get(2), ":serial.send message")?;

                PluginRequest::External(PluginExternalRequest::SerialSend { message })
            }
            ":serial.recv" => {
                let opts: Table = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as Table".to_string())?;

                let timeout_ms: Option<u64> = opts.get("timeout_ms").ok();

                PluginRequest::External(PluginExternalRequest::SerialRecv {
                    timeout: Self::deadline_from_timeout_ms(timeout_ms),
                })
            }
            ":rtt.info" => PluginRequest::External(PluginExternalRequest::RttInfo),
            ":rtt.send" => {
                let message = Self::bytes(value.get(2), ":rtt.send message")?;

                PluginRequest::External(PluginExternalRequest::RttSend { message })
            }
            ":serial.send_and_wait" => Self::send_and_wait(InterfaceType::Serial, value, &req_id)?,
            ":rtt.send_and_wait" => Self::send_and_wait(InterfaceType::Rtt, value, &req_id)?,
            ":rtt.recv" => {
                let opts: Table = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as Table".to_string())?;

                let timeout_ms: Option<u64> = opts.get("timeout_ms").ok();

                PluginRequest::External(PluginExternalRequest::RttRecv {
                    timeout: Self::deadline_from_timeout_ms(timeout_ms),
                })
            }
            ":rtt.read" => {
                let opts: Table = value
                    .get(2)
                    .map_err(|_| "Cannot get second table entry as Table".to_string())?;

                let address: u64 = opts
                    .get("address")
                    .map_err(|_| "Cannot get 'address' field as Number".to_string())?;
                let size: usize = opts
                    .get("size")
                    .map_err(|_| "Cannot get 'size' field as Number".to_string())?;

                if size > 1024 {
                    return Err(
                        "Cannot perform ':rtt.read': 'size' field exceeds maximum of 1024 bytes"
                            .to_string(),
                    );
                }

                PluginRequest::External(PluginExternalRequest::RttRead {
                    timeout: Instant::now().checked_add(Duration::from_millis(1000)),
                    plugin_name: Arc::new(plugin_name),
                    method_id,
                    address,
                    size,
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
                    .sequence_values::<Table<'_>>()
                    .filter_map(|res| res.ok())
                    .filter_map(|t| t.get::<_, String>(1).ok())
                    .collect();

                PluginRequest::Internal(PluginInternalRequest::ReMatches {
                    string,
                    pattern_table,
                })
            }
            ":re.match" => {
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
