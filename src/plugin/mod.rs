pub mod bridge;
pub mod engine;
pub mod installed;
pub mod line_waiter;
pub mod messages;
pub mod method_call;
pub mod rx_lines;
pub mod shell;
pub mod stdlib;
pub mod suite;

use crate::infra::LogLevel;
use crate::infra::logger::Logger;
use crate::plugin::method_call::PluginMethodCallArgs;
use bridge::PluginMethodCallGate;
use method_call::{Entry, PluginMethodCall};
use mlua::{Function, IntoLua, IntoLuaMulti, Lua, LuaOptions, Table, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

pub struct Plugin {
    name: Arc<String>,
    filepath: PathBuf,
    lua: Rc<Lua>,
    log_level: LogLevel,
    /// The method calls this instance started that have not finished yet.
    calls: HashSet<u64>,
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
    /// `source_filepath` is where the plugin came from — it is stored as the
    /// plugin's identity and re-read on reload. `load_filepath` is where the
    /// code is actually read from and whose directory comes first in Lua's
    /// `package.path` (so `require("scope")` resolves against the `scope.lua`
    /// sitting next to it, not one in the working directory). They differ when the engine copies the plugin into
    /// the known plugins directory before loading it.
    pub fn new(
        name: Arc<String>,
        source_filepath: PathBuf,
        load_filepath: PathBuf,
        logger: Logger,
    ) -> Result<Self, String> {
        let lua = Lua::new_with(mlua::StdLib::ALL_SAFE, LuaOptions::default())
            .map_err(|err| err.to_string())?;
        let plugin_dir = load_filepath.parent().unwrap_or(Path::new("/"));
        let code = std::fs::read_to_string(&load_filepath).map_err(|err| err.to_string())?;
        {
            let package: Table = lua
                .globals()
                .get("package")
                .map_err(|err| err.to_string())?;
            let path: String = package.get("path").map_err(|err| err.to_string())?;
            package
                .set(
                    "path",
                    format!("{}/?.lua;{}", plugin_dir.to_string_lossy(), path),
                )
                .map_err(|err| err.to_string())?;
        }
        let plugin_table: Table = lua.load(code).eval().map_err(|err| err.to_string())?;
        lua.globals()
            .set("M", plugin_table)
            .map_err(|err| err.to_string())?;
        lua.load(r#"require("scope")._init_config(M)"#)
            .exec()
            .map_err(|err| err.to_string())?;

        Ok(Self {
            name,
            filepath: source_filepath,
            lua: Rc::new(lua),
            calls: HashSet::new(),
            log_level: LogLevel::Info,
            unload_mode: PluginUnloadMode::None,
            logger,
        })
    }

    pub fn is_user_command_valid(&self, user_command: &str) -> bool {
        self.defines(user_command)
    }

    /// Whether the plugin table has a function named `fn_name`.
    pub fn defines(&self, fn_name: &str) -> bool {
        let table: Table = self.lua.globals().get("M").unwrap();

        table.get::<_, Function>(fn_name).is_ok()
    }

    /// The plugin's `test_*` functions, in the order they are defined.
    pub fn tests(&self) -> Vec<String> {
        let table: Table = self.lua.globals().get("M").unwrap();
        let mut tests = table
            .pairs::<Value, Value>()
            .filter_map(Result::ok)
            .filter_map(|(key, value)| {
                let (Value::String(key), Value::Function(function)) = (key, value) else {
                    return None;
                };
                let name = key.to_str().ok()?;
                let short = name.strip_prefix(suite::TEST_PREFIX)?;

                (!short.is_empty()).then(|| (function.info().line_defined, name.to_string()))
            })
            .collect::<Vec<_>>();
        tests.sort();

        tests.into_iter().map(|(_, name)| name).collect()
    }

    /// Whether the plugin asked, through `M.config.recv_as_string`, to get
    /// received data as a Lua string instead of a list of bytes.
    pub fn recv_as_string(&self) -> bool {
        let table: Table = self.lua.globals().get("M").unwrap();

        table
            .get::<_, Table>("config")
            .and_then(|config| config.get::<_, bool>("recv_as_string"))
            .unwrap_or(false)
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

    /// Whether the method call `method_id` was started by this instance and
    /// still runs.
    pub fn owns(&self, method_id: u64) -> bool {
        self.calls.contains(&method_id)
    }

    pub fn finished(&mut self, method_id: u64) {
        self.calls.remove(&method_id);
    }

    pub fn calls(&self) -> &HashSet<u64> {
        &self.calls
    }

    pub fn spawn_method_call(
        &mut self,
        gate: PluginMethodCallGate,
        fn_name: &str,
        initial_args: impl for<'a> IntoLuaMulti<'a> + 'static,
        has_unpack: bool,
    ) {
        let entry = if has_unpack {
            Entry::Unpack
        } else {
            Entry::Args
        };

        self.spawn(gate, fn_name, initial_args, entry);
    }

    /// Runs `tests` (full `test_*` names) as a suite, and returns the id of the
    /// method call running it.
    pub fn spawn_test_suite(
        &mut self,
        gate: PluginMethodCallGate,
        tests: Vec<String>,
    ) -> Option<u64> {
        self.spawn(gate, "test", tests, Entry::TestSuite)
    }

    fn spawn(
        &mut self,
        gate: PluginMethodCallGate,
        fn_name: &str,
        initial_args: impl for<'a> IntoLuaMulti<'a> + 'static,
        entry: Entry,
    ) -> Option<u64> {
        if !matches!(self.unload_mode, PluginUnloadMode::None) {
            return None;
        }

        let id = gate.id;
        PluginMethodCall::spawn(PluginMethodCallArgs {
            plugin_name: self.name.clone(),
            fn_name: fn_name.to_string(),
            lua: self.lua.clone(),
            initial_args,
            gate,
            logger: self.logger.clone().with_id(fn_name.to_string()),
            entry,
        });
        self.calls.insert(id);

        Some(id)
    }
}

/// Received bytes handed to a receive hook: a list of bytes, or a Lua string
/// when the plugin set `M.config.recv_as_string`.
pub struct RecvData {
    pub bytes: Vec<u8>,
    pub as_string: bool,
}

impl<'lua> IntoLua<'lua> for RecvData {
    fn into_lua(self, lua: &'lua Lua) -> mlua::Result<Value<'lua>> {
        if self.as_string {
            lua.create_string(&self.bytes).map(Value::String)
        } else {
            self.bytes.into_lua(lua)
        }
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
            ["data", "level", "on_serial_recv_line"]
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

    fn plugin_from(tag: &str, code: &str) -> Plugin {
        let dir = std::env::temp_dir().join(format!("scope-plugin-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        super::stdlib::provision(&dir).unwrap();
        let file = dir.join(format!("{tag}.lua"));
        std::fs::write(&file, code).unwrap();

        let plugin = Plugin::new(
            Arc::new(tag.to_string()),
            file.clone(),
            file,
            Logger::new("test".to_string()).0,
        );
        let _ = std::fs::remove_dir_all(&dir);

        plugin.unwrap()
    }

    #[test]
    fn tests_are_listed_in_definition_order() {
        let plugin = plugin_from(
            "order",
            "local M = { test_ = 1 }\n\
             function M.test_zeta() end\n\
             function M.helper() end\n\
             function M.test_alpha() end\n\
             M.test_value = 3\n\
             return M\n",
        );

        assert_eq!(plugin.tests(), ["test_zeta", "test_alpha"]);
    }

    #[test]
    fn loading_gives_the_plugin_a_config_table_with_the_defaults() {
        let plugin = plugin_from("config", "return {}");
        let table: Table = plugin.lua.globals().get("M").unwrap();
        let config: Table = table.get("config").unwrap();

        assert_eq!(config.get::<_, u64>("default_timeout").unwrap(), 1000);
        assert!(!plugin.recv_as_string());
    }

    #[test]
    fn recv_as_string_follows_the_plugin_config() {
        let plugin = plugin_from("as-string", "return { config = { recv_as_string = true } }");

        assert!(plugin.recv_as_string());
    }

    #[test]
    fn the_plugin_directory_comes_first_in_the_lua_path() {
        let plugin = plugin_from("lua-path", "return {}");
        let package: Table = plugin.lua.globals().get("package").unwrap();
        let path: String = package.get("path").unwrap();
        let dir = std::env::temp_dir().join("scope-plugin-test-lua-path");

        assert!(
            path.starts_with(&format!("{}/?.lua;", dir.to_string_lossy())),
            "{path}"
        );
    }

    #[test]
    fn a_config_that_is_not_a_table_fails_the_load() {
        let dir = std::env::temp_dir().join("scope-plugin-test-bad-config");
        let _ = std::fs::remove_dir_all(&dir);
        super::stdlib::provision(&dir).unwrap();
        let file = dir.join("bad.lua");
        std::fs::write(&file, "return { config = true }").unwrap();

        let err = Plugin::new(
            Arc::new("bad".to_string()),
            file.clone(),
            file,
            Logger::new("test".to_string()).0,
        )
        .err()
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            err.contains("M.config must be a table, not boolean"),
            "{err}"
        );
    }

    #[test]
    fn recv_data_becomes_a_string_or_a_list_of_bytes() {
        let lua = Lua::new();
        let check = lua
            .load("return function(data) return type(data), #data end")
            .eval::<mlua::Function>()
            .unwrap();
        let call = |as_string| {
            check
                .call::<_, (String, usize)>(super::RecvData {
                    bytes: b"OK\r\n".to_vec(),
                    as_string,
                })
                .unwrap()
        };

        assert_eq!(call(true), ("string".to_string(), 4));
        assert_eq!(call(false), ("table".to_string(), 4));
    }

    #[test]
    fn test_plugin_new() {
        let _plugin = Plugin::new(
            Arc::new("echo".to_string()),
            PathBuf::from("plugins/echo.lua"),
            PathBuf::from("plugins/echo.lua"),
            Logger::new("test".to_string()).0,
        );
    }
}
