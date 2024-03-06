use crate::messages::SerialRxData;
use crate::text::TextView;
use anyhow::Result;
use chrono::Local;
use homedir::get_my_home;
use rlua::{Context, Function, Lua, RegistryKey, Table, Thread};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Plugin {
    name: String,
    code: String,
}

#[derive(Debug, PartialEq)]
pub enum PluginRequest {
    Println { msg: String },
    Eprintln { msg: String },
    Connect { port: String, baud_rate: u32 },
    Disconnect,
    Reconnect,
    SerialTx { msg: Vec<u8> },
    Sleep { time: Duration },
    Exec { cmd: String },
}

pub enum PluginRequestResult {
    Exec {
        stdout: Vec<String>,
        stderr: Vec<String>,
    },
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
            ":connect" => Ok(PluginRequest::Connect {
                port: value.get(2).map_err(|err| err.to_string())?,
                baud_rate: value.get(3).map_err(|err| err.to_string())?,
            }),
            ":disconnect" => Ok(PluginRequest::Disconnect),
            ":reconnect" => Ok(PluginRequest::Reconnect),
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
            }),
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

        let lua = Lua::new();

        Plugin::check_integrity(&lua, &code)?;

        Ok(Plugin { name, code })
    }

    pub fn println(text_view: Arc<Mutex<TextView>>, plugin_name: String, content: String) {
        let mut text_view = text_view.lock().unwrap();
        text_view.add_data_out(SerialRxData::Plugin {
            timestamp: Local::now(),
            plugin_name,
            content,
            is_successful: true,
        })
    }

    pub fn eprintln(text_view: Arc<Mutex<TextView>>, plugin_name: String, content: String) {
        let mut text_view = text_view.lock().unwrap();
        text_view.add_data_out(SerialRxData::Plugin {
            timestamp: Local::now(),
            plugin_name,
            content,
            is_successful: false,
        })
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    pub fn serial_rx_call(&self, msg: Vec<u8>) -> SerialRxCall {
        let lua = Lua::new();
        let code = self.code.as_str();

        let serial_rx_reg: Result<RegistryKey, String> = lua.context(move |lua_ctx| {
            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx
                .load(code)
                .exec()
                .map_err(|_| "Fail to load Lua code".to_string())?;

            let serial_rx: Thread = lua_ctx
                .load(r#"coroutine.create(serial_rx)"#)
                .eval()
                .map_err(|_| "Fail to create coroutine for serial_rx".to_string())?;
            let reg = lua_ctx
                .create_registry_value(serial_rx)
                .map_err(|_| "Fail to create register for serial_rx coroutines".to_string())?;
            Ok(reg)
        });

        SerialRxCall {
            lua,
            thread: serial_rx_reg.expect("Cannot get serial_rx register"),
            msg,
            req_result: None,
        }
    }

    pub fn user_command_call(&self, arg_list: Vec<String>) -> UserCommandCall {
        let lua = Lua::new();
        let code = self.code.as_str();

        let user_command_reg: Result<RegistryKey, String> = lua.context(move |lua_ctx| {
            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx
                .load(code)
                .exec()
                .map_err(|_| "Fail to load Lua code".to_string())?;

            let user_command: Thread = lua_ctx
                .load(r#"coroutine.create(user_command)"#)
                .eval()
                .map_err(|_| "Fail to create coroutine for user_command".to_string())?;
            let reg = lua_ctx
                .create_registry_value(user_command)
                .map_err(|_| "Fail to create register for user_command coroutines".to_string())?;
            Ok(reg)
        });

        UserCommandCall {
            lua,
            thread: user_command_reg.expect("Cannot get user_command register"),
            arg_list,
            req_result: None,
        }
    }

    fn append_plugins_dir(lua_ctx: &Context) -> Result<(), String> {
        let home_dir = get_my_home()
            .expect("Cannot get home directory")
            .expect("Cannot get home directory")
            .to_str()
            .expect("Cannot get home directory")
            .to_string();

        if lua_ctx
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
        lua.context(|lua_ctx| {
            let globals = lua_ctx.globals();

            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx
                .load(code)
                .exec()
                .map_err(|_| "Fail to load Lua code".to_string())?;

            globals
                .get::<_, Function>("serial_rx")
                .map_err(|_| "serial_rx function not found in Lua code")?;
            globals
                .get::<_, Function>("user_command")
                .map_err(|_| "user_command function not found in Lua code")?;

            Ok(())
        })
    }
}

fn resume_lua_thread<T: for<'a> rlua::ToLuaMulti<'a> + Send>(
    thread: &Thread,
    data: T,
) -> Option<PluginRequest> {
    match thread.resume::<T, Table>(data) {
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

        self.lua.context(move |lua_ctx| {
            let serial_rx: Thread = lua_ctx
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
            }
        })
    }
}

impl Iterator for UserCommandCall {
    type Item = PluginRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let req_result = self.take_request_result();
        let thread = &self.thread;
        let arg_list = self.arg_list.clone();

        self.lua.context(move |lua_ctx| {
            let user_command: Thread = lua_ctx
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
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin::{Plugin, PluginRequest, PluginRequestResult, PluginRequestResultHolder};
    use crate::plugin_installer::PluginInstaller;
    use std::env::current_dir;
    use std::path::{Path, PathBuf};

    #[test]
    fn test_echo() -> Result<(), String> {
        PluginInstaller.post()?;
        Plugin::new(PathBuf::from("plugins/echo.lua"))?;

        Ok(())
    }

    #[test]
    fn test_west_build() -> Result<(), String> {
        let zephyr_base = env!("ZEPHYR_BASE").to_string();
        let path = zephyr_base + "/samples/hello_world";
        let path = Path::new(&path);
        let old_dir = current_dir().expect("Cannot get current dir");

        std::env::set_current_dir(path).map_err(|err| err.to_string())?;
        let Ok(_) = PluginInstaller.post() else {
            return std::env::set_current_dir(old_dir).map_err(|err| err.to_string());
        };
        let Ok(west) = Plugin::new(PathBuf::from("west.lua")) else {
            return std::env::set_current_dir(old_dir).map_err(|err| err.to_string());
        };

        let west_build = || {
            let mut cmd_call = west.user_command_call(
                vec!["build", "-p", "-b", "nrf52dk_nrf52832"]
                    .into_iter()
                    .map(|x| x.to_string())
                    .collect(),
            );

            while let Some(req) = cmd_call.next() {
                dbg!(req);
                cmd_call.attach_request_result(PluginRequestResult::Exec {
                    stdout: vec!["/home/matheuswhite/zephyrproject/zephyr".to_string()],
                    stderr: vec![],
                });
            }
        };

        for _ in 0..2 {
            west_build();
        }

        std::env::set_current_dir(old_dir).map_err(|err| err.to_string())
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
        let expected = vec![
            PluginRequest::Connect {
                port: "/dev/ttyACM0".to_string(),
                baud_rate: 115200,
            },
            PluginRequest::Disconnect,
            PluginRequest::Reconnect,
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
                    port: "/dev/ttyACM0".to_string(),
                    baud_rate: 115200,
                },
                PluginRequest::Connect {
                    port: "/dev/ttyACM0".to_string(),
                    baud_rate: 115200,
                },
            ),
            (PluginRequest::Disconnect, PluginRequest::Disconnect),
            (PluginRequest::Reconnect, PluginRequest::Reconnect),
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
        let expected = vec![
            PluginRequest::Connect {
                port: "/dev/ttyACM0".to_string(),
                baud_rate: 115200,
            },
            PluginRequest::Disconnect,
            PluginRequest::Reconnect,
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
                    port: "/dev/ttyACM0".to_string(),
                    baud_rate: 115200,
                },
                PluginRequest::Connect {
                    port: "/dev/ttyACM0".to_string(),
                    baud_rate: 115200,
                },
            ),
            (PluginRequest::Disconnect, PluginRequest::Disconnect),
            (PluginRequest::Reconnect, PluginRequest::Reconnect),
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
