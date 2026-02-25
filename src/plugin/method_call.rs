#[allow(unused)]
use crate::debug;
use crate::{
    error,
    infra::logger::{LogLevel, Logger},
};

use super::{
    bridge::PluginMethodCallGate,
    messages::{PluginInternalRequest, PluginMethodMessage, PluginRequest, PluginResponse},
    shell::Shell,
};
use mlua::{Function, IntoLuaMulti, Lua, Table, Thread, Value};
use regex::Regex;
use std::{
    hash::{DefaultHasher, Hash, Hasher},
    rc::Rc,
    sync::Arc,
};

pub struct PluginMethodCallArgs<T> {
    pub plugin_name: Arc<String>,
    pub fn_name: String,
    pub index: u128,
    pub lua: Rc<Lua>,
    pub initial_args: T,
    pub gate: PluginMethodCallGate,
    pub logger: Logger,
    pub has_unpack: bool,
}

pub struct PluginMethodCall {
    plugin_name: Arc<String>,
    fn_name: Arc<String>,
    id: u64,
    gate: PluginMethodCallGate,
    #[allow(unused)]
    logger: Logger,
}

impl PluginMethodCall {
    pub fn spawn<T: for<'a> IntoLuaMulti<'a> + 'static>(args: PluginMethodCallArgs<T>) {
        let PluginMethodCallArgs {
            plugin_name,
            fn_name,
            index,
            lua,
            initial_args,
            gate,
            logger,
            has_unpack,
        } = args;
        let mut hasher = DefaultHasher::new();
        plugin_name.hash(&mut hasher);
        fn_name.hash(&mut hasher);
        index.hash(&mut hasher);

        let id = hasher.finish();

        let fn_name = Arc::new(fn_name);

        let sender = gate.sender.clone();
        let pmc = Self {
            plugin_name: plugin_name.clone(),
            fn_name: fn_name.clone(),
            id,
            gate,
            logger: logger.clone(),
        };

        tokio::task::spawn_local(async move {
            if let Err(err) = pmc.call_fn(&lua, initial_args, has_unpack).await {
                error!(logger, "{}", err);
            }

            let _ = sender
                .send(PluginMethodMessage {
                    plugin_name,
                    method_id: id,
                    data: super::messages::PluginExternalRequest::Finish { fn_name },
                })
                .await;
        });
    }

    async fn call_fn<'a>(
        mut self,
        lua: &'a Lua,
        initial_args: impl IntoLuaMulti<'a>,
        has_unpack: bool,
    ) -> Result<(), String> {
        let plugin_table: Table = lua.globals().get("M").unwrap();

        let Ok(_plugin_fn) = plugin_table
            .get::<_, Function>(self.fn_name.as_str())
            .map_err(|err| err.to_string())
        else {
            return Ok(());
        };

        let thread_code = if has_unpack {
            format!(include_str!("unpack.lua"), self.fn_name)
        } else {
            format!(include_str!("thread.lua"), self.fn_name)
        };
        let thread: Thread = lua
            .load(thread_code)
            .eval_async()
            .await
            .map_err(|err| err.to_string())?;

        let Some(mut table) = self.call_fn_inner(lua, &thread, initial_args).await? else {
            return Ok(());
        };

        'run_loop: loop {
            match self.call_fn_inner(lua, &thread, table).await {
                Ok(Some(t)) => table = t,
                Ok(None) => break 'run_loop Ok(()),
                Err(err) => break 'run_loop Err(err),
            }
        }
    }

    async fn call_fn_inner<'a>(
        &mut self,
        lua: &'a Lua,
        thread: &Thread<'a>,
        plugin_fn_args: impl IntoLuaMulti<'a>,
    ) -> Result<Option<Table<'a>>, String> {
        let plugin_req: Table = match thread.resume(plugin_fn_args) {
            Ok(plugin_req) => plugin_req,
            Err(mlua::Error::CoroutineInactive) => return Ok(None),
            Err(mlua::Error::FromLuaConversionError { .. }) => return Ok(None),
            Err(err) => return Err(err.to_string()),
        };

        let plugin_req: PluginRequest = PluginRequest::from_table(
            plugin_req,
            (*self.plugin_name).clone(),
            (*self.fn_name).clone(),
        )
        .map_err(|err: String| err.to_string())?;

        let rsp = match plugin_req {
            PluginRequest::Internal(internal_req) => {
                self.handle_internal_plugin_request(internal_req).await
            }
            PluginRequest::External(external_req) => {
                self.gate
                    .sender
                    .send(PluginMethodMessage {
                        plugin_name: self.plugin_name.clone(),
                        method_id: self.id,
                        data: external_req,
                    })
                    .await
                    .map_err(|err| err.to_string())?;

                'rsp_loop: loop {
                    let PluginMethodMessage {
                        plugin_name: _plugin_name,
                        method_id,
                        data,
                    } = self
                        .gate
                        .receiver
                        .recv()
                        .await
                        .map_err(|err| err.to_string())?;

                    if method_id != self.id {
                        continue 'rsp_loop;
                    }

                    break 'rsp_loop data;
                }
            }
        };

        let next_table = self.rsp_decode(lua, rsp)?;

        Ok(Some(next_table))
    }

    async fn handle_internal_plugin_request(&self, req: PluginInternalRequest) -> PluginResponse {
        match req {
            PluginInternalRequest::SysSleep { time } => {
                tokio::time::sleep(time).await;
                PluginResponse::SysSleep
            }
            PluginInternalRequest::ReLiteral { string } => {
                let special_chars = "/.*+?|[](){}\\";
                let literal = string
                    .chars()
                    .map(|c| {
                        if special_chars.contains(c) {
                            format!("\\{}", c)
                        } else {
                            c.to_string()
                        }
                    })
                    .collect();

                PluginResponse::ReLiteral { literal }
            }
            PluginInternalRequest::ReMatches {
                string,
                pattern_table,
            } => {
                let pos = pattern_table
                    .iter()
                    .filter_map(|pattern| Regex::new(pattern).ok())
                    .position(|re| re.is_match(&string));
                let pattern = pos
                    .and_then(|pos| pattern_table.get(pos))
                    .map(|pattern| pattern.to_string());

                PluginResponse::ReMatches { pattern }
            }
            PluginInternalRequest::ReMatch { string, pattern } => {
                let is_match = Regex::new(&pattern)
                    .ok()
                    .and_then(|regex| regex.is_match(&string).then_some(()))
                    .is_some();

                PluginResponse::ReMatch { is_match }
            }
            PluginInternalRequest::ShellRun { cmd } => {
                let (stdout, stderr) = match Shell::run(cmd).await {
                    Ok(r) => r,
                    Err(err) => ("".to_string(), err),
                };

                PluginResponse::ShellRun { stdout, stderr }
            }
            PluginInternalRequest::ShellExist { program } => {
                let exist = Shell::exist(program).await;

                PluginResponse::ShellExist { exist }
            }
        }
    }

    fn rsp_decode<'a>(&self, lua: &'a Lua, rsp: PluginResponse) -> Result<Table<'a>, String> {
        let table = lua.create_table().map_err(|err| err.to_string())?;

        match rsp {
            PluginResponse::Log
            | PluginResponse::SerialSend
            | PluginResponse::RttSend
            | PluginResponse::SysSleep => {}
            PluginResponse::ReMatches { pattern } => {
                table
                    .set(
                        "fn_name",
                        match pattern {
                            Some(pattern) => Value::String(
                                lua.create_string(pattern).map_err(|err| err.to_string())?,
                            ),
                            None => Value::Nil,
                        },
                    )
                    .map_err(|err| err.to_string())?;
            }
            PluginResponse::ReMatch { is_match } => table
                .set("is_match", is_match)
                .map_err(|err| err.to_string())?,
            PluginResponse::SerialInfo { port, baudrate } => {
                table.set("port", port).map_err(|err| err.to_string())?;
                table
                    .set("baud_rate", baudrate)
                    .map_err(|err| err.to_string())?;
            }
            PluginResponse::SerialRecv { err, message } => {
                if err.is_empty() {
                    table
                        .set("err", Value::Nil)
                        .map_err(|err| err.to_string())?;
                } else {
                    table.set("err", err).map_err(|err| err.to_string())?;
                }
                table.set("data", message).map_err(|err| err.to_string())?;
            }
            PluginResponse::ReLiteral { literal } => {
                table
                    .set("literal", literal)
                    .map_err(|err| err.to_string())?;
            }
            PluginResponse::ShellRun { stdout, stderr } => {
                table.set("stdout", stdout).map_err(|err| err.to_string())?;
                table.set("stderr", stderr).map_err(|err| err.to_string())?;
            }
            PluginResponse::ShellExist { exist } => {
                table.set("exist", exist).map_err(|err| err.to_string())?;
            }
            PluginResponse::RttInfo { target, channel } => {
                table.set("target", target).map_err(|err| err.to_string())?;
                table
                    .set("channel", channel)
                    .map_err(|err| err.to_string())?;
            }
            PluginResponse::RttRecv { err, message } => {
                if err.is_empty() {
                    table
                        .set("err", Value::Nil)
                        .map_err(|err| err.to_string())?;
                } else {
                    table.set("err", err).map_err(|err| err.to_string())?;
                }
                table.set("data", message).map_err(|err| err.to_string())?;
            }
            PluginResponse::RttRead { err, data } => {
                if err.is_empty() {
                    table
                        .set("err", Value::Nil)
                        .map_err(|err| err.to_string())?;
                } else {
                    table.set("err", err).map_err(|err| err.to_string())?;
                }
                table.set("data", data).map_err(|err| err.to_string())?;
            }
        }

        Ok(table)
    }
}
