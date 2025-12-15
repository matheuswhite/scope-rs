pub mod bridge;
pub mod engine;
pub mod messages;
pub mod method_call;
pub mod shell;

use bridge::PluginMethodCallGate;
use method_call::PluginMethodCall;
use crate::infra::logger::Logger;
use crate::infra::LogLevel;
use crate::plugin::method_call::PluginMethodCallArgs;
use mlua::{Function, IntoLuaMulti, Lua, LuaOptions, Table};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

pub struct Plugin {
    name: Arc<String>,
    filepath: PathBuf,
    lua: Rc<Lua>,
    log_level: LogLevel,
    index: u128,
    unload_mode: PluginUnloadMode,
    logger: Logger,
}

#[derive(Clone, Copy)]
pub enum PluginUnloadMode {
    None,
    Unload,
    Reload,
}

impl Plugin {
    pub fn new(name: Arc<String>, filepath: PathBuf, logger: Logger) -> Result<Self, String> {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, LuaOptions::default())
            .map_err(|err| err.to_string())?;
        let plugin_dir = filepath.parent().unwrap_or(Path::new("/"));
        let code = std::fs::read_to_string(&filepath).map_err(|err| err.to_string())?;
        lua.load(format!(
            "package.path = package.path .. ';{}/?.lua'",
            plugin_dir.to_str().unwrap_or("")
        ))
        .exec()
        .map_err(|err| err.to_string())?;
        let plugin_table: Table = lua.load(code).eval().map_err(|err| err.to_string())?;
        lua.globals()
            .set("M", plugin_table)
            .map_err(|err| err.to_string())?;

        Ok(Self {
            name,
            filepath,
            lua: Rc::new(lua),
            index: 0,
            log_level: LogLevel::Info,
            unload_mode: PluginUnloadMode::None,
            logger,
        })
    }

    pub fn is_user_command_valid(&self, user_command: &str) -> bool {
        let table: Table = self.lua.globals().get("M").unwrap();

        table.get::<_, Function>(user_command).is_ok()
    }

    pub fn log_level(&self) -> LogLevel {
        self.log_level
    }

    pub fn set_log_level(&mut self, log_level: LogLevel) {
        self.log_level = log_level;
    }

    pub fn unload_mode(&self) -> PluginUnloadMode {
        self.unload_mode
    }

    pub fn set_unload_mode(&mut self, mode: PluginUnloadMode) {
        self.unload_mode = mode;
    }

    pub fn filepath(self) -> PathBuf {
        self.filepath
    }

    pub fn spawn_method_call(
        &mut self,
        gate: PluginMethodCallGate,
        fn_name: &str,
        initial_args: impl for<'a> IntoLuaMulti<'a> + 'static,
        has_unpack: bool,
    ) {
        if !matches!(self.unload_mode, PluginUnloadMode::None) {
            return;
        }

        PluginMethodCall::spawn(PluginMethodCallArgs {
            plugin_name: self.name.clone(),
            fn_name: fn_name.to_string(),
            index: self.index,
            lua: self.lua.clone(),
            initial_args,
            gate,
            logger: self.logger.clone().with_id(fn_name.to_string()),
            has_unpack,
        });

        self.index = self.index.overflowing_add_signed(1).0;
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

    use crate::infra::logger::Logger;

    use super::Plugin;
    use mlua::{Lua, LuaOptions, Table, Value};

    fn print_table(lua: Lua) {
        let table: Table = lua.globals().get("M").unwrap();
        let mut keys = vec![];

        for r in table.pairs() {
            let (k, _): (String, Value) = r.unwrap();

            keys.push(k);
        }
        keys.sort();

        assert_eq!(
            keys,
            ["data", "level", "on_serial_recv"]
                .into_iter()
                .map(|x| x.to_string())
                .collect::<Vec<_>>()
        )
    }

    #[test]
    fn test_lua_load_plugin() {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, LuaOptions::default()).unwrap();
        let code = std::fs::read_to_string("plugins/echo.lua").unwrap();

        lua.load("package.path = package.path .. ';plugins/?.lua'")
            .exec()
            .unwrap();
        let table: Table = lua.load(code).eval().unwrap();
        lua.globals().set("M", table).unwrap();

        print_table(lua);
    }

    #[test]
    fn test_plugin_new() {
        let _plugin = Plugin::new(
            Arc::new("echo".to_string()),
            PathBuf::from("plugins/echo.lua"),
            Logger::new("test".to_string()).0,
        );
    }
}
