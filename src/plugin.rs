use crate::messages::SerialRxData;
use crate::text::TextView;
use anyhow::Result;
use chrono::Local;
use homedir::get_my_home;
use mlua::{Function, Lua, RegistryKey, Table, Thread};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Plugin {
    name: String,
    code: String,
}

#[derive(Debug, PartialEq)]
pub enum PluginRequest {
    Println {
        msg: String,
    },
    Eprintln {
        msg: String,
    },
    Connect {
        port: Option<String>,
        baud_rate: Option<u32>,
    },
    Disconnect,
    SerialTx {
        msg: Vec<u8>,
    },
    Sleep {
        time: Duration,
    },
    Exec {
        cmd: String,
        quiet: bool,
    },
    Info,
}

pub enum PluginRequestResult {
    Exec {
        stdout: Vec<String>,
        stderr: Vec<String>,
    },
    Info {
        serial: SerialInfoResult,
    },
}

pub struct SerialInfoResult {
    port: String,
    baudrate: u32,
    is_connected: bool,
}

impl SerialInfoResult {
    pub fn new(port: String, baudrate: u32, is_connected: bool) -> Self {
        Self {
            port,
            baudrate,
            is_connected,
        }
    }
}

pub struct SerialRxCall {
    lua: Lua,
    thread: RegistryKey,
    msg: Vec<u8>,
    req_result: Option<PluginRequestResult>,
}

pub struct UserCommandCall {
    lua: Lua,
    thread: RegistryKey,
    arg_list: Vec<String>,
    req_result: Option<PluginRequestResult>,
}

pub trait PluginRequestResultHolder {
    fn attach_request_result(&mut self, request_result: PluginRequestResult);
    fn take_request_result(&mut self) -> Option<PluginRequestResult>;
}

impl PluginRequestResultHolder for UserCommandCall {
    fn attach_request_result(&mut self, request_result: PluginRequestResult) {
        self.req_result = Some(request_result);
    }

    fn take_request_result(&mut self) -> Option<PluginRequestResult> {
        self.req_result.take()
    }
}

impl PluginRequestResultHolder for SerialRxCall {
    fn attach_request_result(&mut self, request_result: PluginRequestResult) {
        self.req_result = Some(request_result);
    }

    fn take_request_result(&mut self) -> Option<PluginRequestResult> {
        self.req_result.take()
    }
}

impl<'a> TryFrom<Table<'a>> for PluginRequest {
    type Error = String;

    fn try_from(value: Table) -> std::result::Result<Self, Self::Error> {
        let id: String = value.get(1).map_err(|err| err.to_string())?;

        match id.as_str() {
            ":println" => Ok(PluginRequest::Println {
                msg: value.get(2).map_err(|err| err.to_string())?,
            }),
            ":eprintln" => Ok(PluginRequest::Eprintln {
                msg: value.get(2).map_err(|err| err.to_string())?,
            }),
            ":connect" => {
                let Some(first_arg): Option<String> = value.get(2).ok() else {
                    return Ok(PluginRequest::Connect {
                        port: None,
                        baud_rate: None,
                    });
                };

                let (port, baud_rate) = if first_arg.chars().all(|x| x.is_ascii_digit()) {
                    (
                        None,
                        Some(
                            first_arg
                                .parse::<u32>()
                                .map_err(|_| "Cannot parse baud rate".to_string())?,
                        ),
                    )
                } else {
                    (Some(first_arg), None)
                };

                let Some(second_arg): Option<String> = value.get(3).ok() else {
                    return Ok(PluginRequest::Connect { port, baud_rate });
                };

                let (port, baud_rate) = if port.is_some() {
                    (
                        port,
                        Some(
                            second_arg
                                .parse::<u32>()
                                .map_err(|_| "Cannot parse baud rate".to_string())?,
                        ),
                    )
                } else {
                    (Some(second_arg), baud_rate)
                };

                Ok(PluginRequest::Connect { port, baud_rate })
            }
            ":disconnect" => Ok(PluginRequest::Disconnect),
            ":serial_tx" => Ok(PluginRequest::SerialTx {
                msg: value.get(2).map_err(|err| err.to_string())?,
            }),
            ":sleep" => {
                let time: i32 = value.get(2).map_err(|err| err.to_string())?;
                Ok(PluginRequest::Sleep {
                    time: Duration::from_millis(time as u64),
                })
            }
            ":exec" => Ok(PluginRequest::Exec {
                cmd: value.get(2).map_err(|err| err.to_string())?,
                quiet: value.get(3).map_err(|err| err.to_string())?,
            }),
            ":info" => Ok(PluginRequest::Info),
            _ => Err("Unknown function".to_string()),
        }
    }
}

impl Plugin {
    pub fn new(filepath: PathBuf) -> Result<Plugin, String> {
        let name = filepath
            .with_extension("")
            .file_name()
            .ok_or("Cannot get filename of plugin".to_string())?
            .to_str()
            .ok_or("Cannot convert plugin name to string".to_string())?
            .to_string();
        let code = std::fs::read_to_string(filepath).map_err(|_| "Cannot read plugin file")?;

        Self::from_string(name, code)
    }

    pub fn from_string(name: String, code: String) -> Result<Plugin, String> {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, mlua::LuaOptions::default())
            .map_err(|_| "Cannot create Lua obj".to_string())?;

        Plugin::check_integrity(&lua, &code)?;

        Ok(Plugin { name, code })
    }

    pub async fn println(text_view: Arc<Mutex<TextView>>, plugin_name: String, content: String) {
        let mut text_view = text_view.lock().await;
        text_view
            .add_data_out(SerialRxData::Plugin {
                timestamp: Local::now(),
                plugin_name,
                content,
                is_successful: true,
            })
            .await;
    }

    pub async fn eprintln(text_view: Arc<Mutex<TextView>>, plugin_name: String, content: String) {
        let mut text_view = text_view.lock().await;
        text_view
            .add_data_out(SerialRxData::Plugin {
                timestamp: Local::now(),
                plugin_name,
                content,
                is_successful: false,
            })
            .await;
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    fn create_lua_thread(
        lua: &Lua,
        code: &str,
        coroutine_name: &str,
    ) -> Result<RegistryKey, String> {
        Plugin::append_plugins_dir(lua)?;

        lua.load(code)
            .exec()
            .map_err(|_| "Fail to load Lua code".to_string())?;

        let serial_rx: Thread = lua
            .load(format!("coroutine.create({})", coroutine_name))
            .eval()
            .map_err(|_| format!("Fail to create coroutine for {}", coroutine_name))?;
        let reg = lua
            .create_registry_value(serial_rx)
            .map_err(|_| format!("Fail to create register for {} coroutines", coroutine_name))?;
        Ok(reg)
    }

    pub fn serial_rx_call(&self, msg: Vec<u8>) -> SerialRxCall {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, mlua::LuaOptions::default())
            .expect("Cannot create Lua obj");

        let serial_rx_reg = Self::create_lua_thread(&lua, self.code.as_str(), "serial_rx");

        SerialRxCall {
            lua,
            thread: serial_rx_reg.expect("Cannot get serial_rx register"),
            msg,
            req_result: None,
        }
    }

    pub fn user_command_call(&self, arg_list: Vec<String>) -> UserCommandCall {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, mlua::LuaOptions::default())
            .expect("Cannot create Lua obj");

        let user_command_reg = Self::create_lua_thread(&lua, self.code.as_str(), "user_command");

        UserCommandCall {
            lua,
            thread: user_command_reg.expect("Cannot get user_command register"),
            arg_list,
            req_result: None,
        }
    }

    fn append_plugins_dir(lua: &Lua) -> Result<(), String> {
        let home_dir = get_my_home()
            .expect("Cannot get home directory")
            .expect("Cannot get home directory")
            .to_str()
            .expect("Cannot get home directory")
            .to_string();

        if lua
            .load(
                format!(
                    "package.path = package.path .. ';{}/.config/scope/plugins/?.lua'",
                    home_dir.replace('\\', "/")
                )
                .as_str(),
            )
            .exec()
            .is_err()
        {
            return Err("Cannot get default plugin path".to_string());
        }

        Ok(())
    }

    fn check_integrity(lua: &Lua, code: &str) -> Result<(), String> {
        let globals = lua.globals();

        Plugin::append_plugins_dir(lua)?;

        lua.load(code)
            .exec()
            .map_err(|_| "Fail to load Lua code".to_string())?;

        globals
            .get::<_, Function>("serial_rx")
            .map_err(|_| "serial_rx function not found in Lua code")?;
        globals
            .get::<_, Function>("user_command")
            .map_err(|_| "user_command function not found in Lua code")?;

        Ok(())
    }
}

fn resume_lua_thread<T>(thread: &Thread, data: T) -> Option<PluginRequest>
where
    T: for<'a> mlua::IntoLuaMulti<'a>,
{
    match thread.resume::<_, Table>(data) {
        Ok(req) => {
            let req: PluginRequest = match req.try_into() {
                Ok(req) => req,
                Err(msg) => return Some(PluginRequest::Eprintln { msg }),
            };
            Some(req)
        }
        Err(_) => None,
    }
}

impl Iterator for SerialRxCall {
    type Item = PluginRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let req_result = self.take_request_result();
        let thread = &self.thread;
        let msg = self.msg.clone();

        let serial_rx: Thread = self
            .lua
            .registry_value(thread)
            .expect("Cannot get serial_rx register");

        let Some(req_result) = req_result else {
            return resume_lua_thread(&serial_rx, msg);
        };

        match req_result {
            PluginRequestResult::Exec { stdout, stderr } => {
                match serial_rx.resume::<_, Table>((msg, stdout, stderr)) {
                    Ok(req) => {
                        let req: PluginRequest = match req.try_into() {
                            Ok(req) => req,
                            Err(msg) => return Some(PluginRequest::Eprintln { msg }),
                        };
                        Some(req)
                    }
                    Err(_) => None,
                }
            }
            PluginRequestResult::Info {
                serial:
                    SerialInfoResult {
                        port,
                        baudrate,
                        is_connected,
                    },
            } => {
                let serial = self
                    .lua
                    .create_table()
                    .expect("Cannot create serial lua table");
                serial.set("port", port).expect("Cannot add port");
                serial
                    .set("baudrate", baudrate)
                    .expect("Cannot add baudrate");
                serial
                    .set("is_connected", is_connected)
                    .expect("Cannot add baudrate");

                let table = self.lua.create_table().expect("Cannot create a lua table");
                table.set("serial", serial).expect("Cannot add serial");

                match serial_rx.resume::<_, Table>((msg, table)) {
                    Ok(req) => {
                        let req: PluginRequest = match req.try_into() {
                            Ok(req) => req,
                            Err(msg) => return Some(PluginRequest::Eprintln { msg }),
                        };
                        Some(req)
                    }
                    Err(_) => None,
                }
            }
        }
    }
}

impl Iterator for UserCommandCall {
    type Item = PluginRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let req_result = self.take_request_result();
        let thread = &self.thread;
        let arg_list = self.arg_list.clone();

        let user_command: Thread = self
            .lua
            .registry_value(thread)
            .expect("Cannot get user_command register");

        let Some(req_result) = req_result else {
            return resume_lua_thread(&user_command, arg_list);
        };

        match req_result {
            PluginRequestResult::Exec { stdout, stderr } => {
                match user_command.resume::<_, Table>((arg_list, stdout, stderr)) {
                    Ok(req) => {
                        let req: PluginRequest = match req.try_into() {
                            Ok(req) => req,
                            Err(msg) => return Some(PluginRequest::Eprintln { msg }),
                        };
                        Some(req)
                    }
                    Err(_) => None,
                }
            }
            PluginRequestResult::Info {
                serial:
                    SerialInfoResult {
                        port,
                        baudrate,
                        is_connected,
                    },
            } => {
                let serial = self
                    .lua
                    .create_table()
                    .expect("Cannot create serial lua table");
                serial.set("port", port).expect("Cannot add port");
                serial
                    .set("baudrate", baudrate)
                    .expect("Cannot add baudrate");
                serial
                    .set("is_connected", is_connected)
                    .expect("Cannot add baudrate");

                let table = self.lua.create_table().expect("Cannot create a lua table");
                table.set("serial", serial).expect("Cannot add serial");

                match user_command.resume::<_, Table>((arg_list, table)) {
                    Ok(req) => {
                        let req: PluginRequest = match req.try_into() {
                            Ok(req) => req,
                            Err(msg) => return Some(PluginRequest::Eprintln { msg }),
                        };
                        Some(req)
                    }
                    Err(_) => None,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin::{Plugin, PluginRequest};
    use crate::plugin_installer::PluginInstaller;
    use std::path::PathBuf;

    #[test]
    fn test_echo() -> Result<(), String> {
        PluginInstaller.post()?;
        Plugin::new(PathBuf::from("plugins/echo.lua"))?;

        Ok(())
    }

    #[test]
    fn test_get_name() -> Result<(), String> {
        PluginInstaller.post()?;
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua"))?;
        let expected = "test";

        assert_eq!(plugin.name(), expected);

        Ok(())
    }

    #[test]
    fn test_serial_rx_iter() -> Result<(), String> {
        PluginInstaller.post()?;
        let msg = "Hello, World!";
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua"))?;
        let serial_rx_call = plugin.serial_rx_call(msg.as_bytes().to_vec());
        let expected = [
            PluginRequest::Connect {
                port: Some("/dev/ttyACM0".to_string()),
                baud_rate: Some(115200),
            },
            PluginRequest::Disconnect,
            PluginRequest::SerialTx {
                msg: msg.as_bytes().to_vec(),
            },
            PluginRequest::Println {
                msg: format!("Sent {}", msg),
            },
            PluginRequest::Eprintln {
                msg: "Timeout".to_string(),
            },
        ];

        for (i, req) in serial_rx_call.enumerate() {
            assert_eq!(req, expected[i]);
        }

        Ok(())
    }

    #[test]
    fn test_2_serial_rx_iter() -> Result<(), String> {
        PluginInstaller.post()?;
        let msg = ["Hello, World!", "Other Message"];
        let plugin = [
            Plugin::new(PathBuf::from("plugins/test.lua"))?,
            Plugin::new(PathBuf::from("plugins/test.lua"))?,
        ];
        let mut serial_rx_call = [
            plugin[0].serial_rx_call(msg[0].as_bytes().to_vec()),
            plugin[1].serial_rx_call(msg[1].as_bytes().to_vec()),
        ];
        let expected = vec![
            (
                PluginRequest::Connect {
                    port: Some("/dev/ttyACM0".to_string()),
                    baud_rate: Some(115200),
                },
                PluginRequest::Connect {
                    port: Some("/dev/ttyACM0".to_string()),
                    baud_rate: Some(115200),
                },
            ),
            (PluginRequest::Disconnect, PluginRequest::Disconnect),
            (
                PluginRequest::SerialTx {
                    msg: msg[0].as_bytes().to_vec(),
                },
                PluginRequest::SerialTx {
                    msg: msg[1].as_bytes().to_vec(),
                },
            ),
            (
                PluginRequest::Println {
                    msg: format!("Sent {}", msg[0]),
                },
                PluginRequest::Println {
                    msg: format!("Sent {}", msg[1]),
                },
            ),
            (
                PluginRequest::Eprintln {
                    msg: "Timeout".to_string(),
                },
                PluginRequest::Eprintln {
                    msg: "Timeout".to_string(),
                },
            ),
        ];

        for (exp1, exp2) in expected {
            let req1 = serial_rx_call[0].next();
            let req2 = serial_rx_call[1].next();

            assert_eq!(req1, Some(exp1));
            assert_eq!(req2, Some(exp2));
        }

        Ok(())
    }

    #[test]
    fn test_user_command_iter() -> Result<(), String> {
        PluginInstaller.post()?;
        let arg_list = vec!["Hello", "World!"]
            .into_iter()
            .map(|arg| arg.to_string())
            .collect();
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua"))?;
        let user_command_call = plugin.user_command_call(arg_list);
        let expected = [
            PluginRequest::Connect {
                port: Some("/dev/ttyACM0".to_string()),
                baud_rate: Some(115200),
            },
            PluginRequest::Disconnect,
            PluginRequest::SerialTx {
                msg: "Hello".as_bytes().to_vec(),
            },
            PluginRequest::Println {
                msg: "Sent World!".to_string(),
            },
            PluginRequest::Eprintln {
                msg: "Timeout".to_string(),
            },
        ];

        for (i, req) in user_command_call.enumerate() {
            assert_eq!(req, expected[i]);
        }

        Ok(())
    }

    #[test]
    fn test_2_user_command_iter() -> Result<(), String> {
        PluginInstaller.post()?;
        let arg_list = [vec!["Hello", "World!"], vec!["Other", "Message"]]
            .into_iter()
            .map(|arg_list| {
                arg_list
                    .into_iter()
                    .map(|arg| arg.to_string())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let plugin = [
            Plugin::new(PathBuf::from("plugins/test.lua"))?,
            Plugin::new(PathBuf::from("plugins/test.lua"))?,
        ];
        let mut user_command_call = [
            plugin[0].user_command_call(arg_list[0].clone()),
            plugin[1].user_command_call(arg_list[1].clone()),
        ];
        let expected = vec![
            (
                PluginRequest::Connect {
                    port: Some("/dev/ttyACM0".to_string()),
                    baud_rate: Some(115200),
                },
                PluginRequest::Connect {
                    port: Some("/dev/ttyACM0".to_string()),
                    baud_rate: Some(115200),
                },
            ),
            (PluginRequest::Disconnect, PluginRequest::Disconnect),
            (
                PluginRequest::SerialTx {
                    msg: arg_list[0][0].as_bytes().to_vec(),
                },
                PluginRequest::SerialTx {
                    msg: arg_list[1][0].as_bytes().to_vec(),
                },
            ),
            (
                PluginRequest::Println {
                    msg: format!("Sent {}", arg_list[0][1]),
                },
                PluginRequest::Println {
                    msg: format!("Sent {}", arg_list[1][1]),
                },
            ),
            (
                PluginRequest::Eprintln {
                    msg: "Timeout".to_string(),
                },
                PluginRequest::Eprintln {
                    msg: "Timeout".to_string(),
                },
            ),
        ];

        for (exp1, exp2) in expected {
            let req1 = user_command_call[0].next();
            let req2 = user_command_call[1].next();

            assert_eq!(req1, Some(exp1));
            assert_eq!(req2, Some(exp2));
        }

        Ok(())
    }
}
