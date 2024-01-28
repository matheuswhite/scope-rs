use anyhow::Result;
use homedir::get_my_home;
use rlua::{Context, Function, Lua, RegistryKey, Table, Thread};
use std::io::{Error, ErrorKind};
use std::path::PathBuf;

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
}

pub struct SerialRxCall {
    lua: Lua,
    thread: RegistryKey,
    msg: Vec<u8>,
}

pub struct UserCommandCall {
    lua: Lua,
    thread: RegistryKey,
    arg_list: Vec<String>,
}

impl<'a> TryFrom<Table<'a>> for PluginRequest {
    type Error = Error;

    fn try_from(value: Table) -> std::result::Result<Self, Self::Error> {
        let id: String = value.get(1).unwrap();

        match id.as_str() {
            ":println" => Ok(PluginRequest::Println {
                msg: value.get(2).unwrap(),
            }),
            ":eprintln" => Ok(PluginRequest::Eprintln {
                msg: value.get(2).unwrap(),
            }),
            ":connect" => Ok(PluginRequest::Connect {
                port: value.get(2).unwrap(),
                baud_rate: value.get(3).unwrap(),
            }),
            ":disconnect" => Ok(PluginRequest::Disconnect),
            ":reconnect" => Ok(PluginRequest::Reconnect),
            ":serial_tx" => Ok(PluginRequest::SerialTx {
                msg: value.get(2).unwrap(),
            }),
            _ => Err(Error::from(ErrorKind::Other)),
        }
    }
}

impl Plugin {
    const SCOPE_LUA: &'static str = include_str!("../plugins/scope.lua");

    pub fn new(filepath: PathBuf) -> Result<Plugin> {
        let name = filepath
            .with_extension("")
            .file_name()
            .ok_or(Error::from(ErrorKind::Other))?
            .to_str()
            .ok_or(Error::from(ErrorKind::Other))?
            .to_string();
        let code = std::fs::read_to_string(filepath)?;

        let lua = Lua::new();

        Plugin::check_integrity(&lua, &code)?;

        Ok(Plugin { name, code })
    }

    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    #[allow(unused)]
    pub fn serial_rx_call(&self, msg: Vec<u8>) -> SerialRxCall {
        let lua = Lua::new();
        let code = self.code.as_str();

        let serial_rx_reg: Result<RegistryKey> = lua.context(move |lua_ctx| {
            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx.load(code).exec()?;

            let serial_rx: Thread = lua_ctx.load(r#"coroutine.create(serial_rx)"#).eval()?;
            let reg = lua_ctx.create_registry_value(serial_rx)?;
            Ok(reg)
        });

        SerialRxCall {
            lua,
            thread: serial_rx_reg.unwrap(),
            msg,
        }
    }

    pub fn user_command_call(&self, arg_list: Vec<String>) -> UserCommandCall {
        let lua = Lua::new();
        let code = self.code.as_str();

        let user_command_reg: Result<RegistryKey> = lua.context(move |lua_ctx| {
            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx.load(code).exec()?;

            let user_command: Thread = lua_ctx.load(r#"coroutine.create(user_command)"#).eval()?;
            let reg = lua_ctx.create_registry_value(user_command)?;
            Ok(reg)
        });

        UserCommandCall {
            lua,
            thread: user_command_reg.unwrap(),
            arg_list,
        }
    }

    fn append_plugins_dir(lua_ctx: &Context) -> rlua::Result<()> {
        let home_dir = get_my_home()
            .unwrap()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        lua_ctx
            .load(
                format!(
                    "package.path = package.path .. ';{}/.config/scope/plugins/?.lua'",
                    home_dir
                )
                .as_str(),
            )
            .exec()
    }

    fn check_integrity(lua: &Lua, code: &str) -> Result<()> {
        lua.context(|lua_ctx| {
            let globals = lua_ctx.globals();

            Plugin::append_plugins_dir(&lua_ctx)?;

            lua_ctx.load(Plugin::SCOPE_LUA).exec()?;
            lua_ctx.load(code).exec()?;

            globals.get::<_, Function>("serial_rx")?;
            globals.get::<_, Function>("user_command")?;

            Ok(())
        })
    }
}

impl Iterator for SerialRxCall {
    type Item = PluginRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let thread = &self.thread;
        let msg = self.msg.clone();

        self.lua.context(move |lua_ctx| {
            let serial_rx: Thread = lua_ctx.registry_value(thread).unwrap();

            match serial_rx.resume::<_, Table>(msg) {
                Ok(req) => {
                    let req: PluginRequest = req.try_into().unwrap();
                    Some(req)
                }
                Err(_) => None,
            }
        })
    }
}

impl Iterator for UserCommandCall {
    type Item = PluginRequest;

    fn next(&mut self) -> Option<Self::Item> {
        let thread = &self.thread;
        let arg_list = self.arg_list.clone();

        self.lua.context(move |lua_ctx| {
            let user_command: Thread = lua_ctx.registry_value(thread).unwrap();

            match user_command.resume::<_, Table>(arg_list) {
                Ok(req) => {
                    let req: PluginRequest = req.try_into().unwrap();
                    Some(req)
                }
                Err(_) => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::plugin::{Plugin, PluginRequest};
    use std::path::PathBuf;

    #[test]
    fn test_echo() {
        Plugin::new(PathBuf::from("plugins/echo.lua")).unwrap();
    }

    #[test]
    fn test_get_name() {
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua")).unwrap();
        let expected = "test";

        assert_eq!(plugin.name(), expected);
    }

    #[test]
    fn test_serial_rx_iter() {
        let msg = "Hello, World!";
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua")).unwrap();
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
    }

    #[test]
    fn test_2_serial_rx_iter() {
        let msg = ["Hello, World!", "Other Message"];
        let plugin = [
            Plugin::new(PathBuf::from("plugins/test.lua")).unwrap(),
            Plugin::new(PathBuf::from("plugins/test.lua")).unwrap(),
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

            assert_eq!(req1.unwrap(), exp1);
            assert_eq!(req2.unwrap(), exp2);
        }
    }

    #[test]
    fn test_user_command_iter() {
        let arg_list = vec!["Hello", "World!"]
            .into_iter()
            .map(|arg| arg.to_string())
            .collect();
        let plugin = Plugin::new(PathBuf::from("plugins/test.lua")).unwrap();
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
    }

    #[test]
    fn test_2_user_command_iter() {
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
            Plugin::new(PathBuf::from("plugins/test.lua")).unwrap(),
            Plugin::new(PathBuf::from("plugins/test.lua")).unwrap(),
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

            assert_eq!(req1.unwrap(), exp1);
            assert_eq!(req2.unwrap(), exp2);
        }
    }
}
