use mlua::{IntoLuaMulti, Lua, LuaOptions, Table};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::infra::LogLevel;

use super::bridge::PluginMethodCallGate;
use super::method_call::PluginMethodCall;

pub struct Plugin {
    name: Arc<String>,
    filepath: PathBuf,
    lua: Arc<Lua>,
    log_level: LogLevel,
    index: u128,
    unload_mode: PluginUnloadMode,
}

#[derive(Clone, Copy)]
pub enum PluginUnloadMode {
    None,
    Unload,
    Reload,
}

impl Plugin {
    pub fn new(name: Arc<String>, filepath: PathBuf) -> Result<Self, String> {
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
            lua: Arc::new(lua),
            index: 0,
            log_level: LogLevel::Info,
            unload_mode: PluginUnloadMode::None,
        })
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
    ) {
        if !matches!(self.unload_mode, PluginUnloadMode::None) {
            return;
        }

        PluginMethodCall::spawn(
            self.name.clone(),
            fn_name.to_string(),
            self.index,
            self.lua.clone(),
            initial_args,
            gate,
        );

        self.index = self.index.overflowing_add_signed(0).0;
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc};

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
        );
    }
}
