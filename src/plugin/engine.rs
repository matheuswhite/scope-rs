use super::{
    Plugin, PluginUnloadMode, RecvData,
    bridge::{PluginEngineGate, PluginMethodCallGate},
    installed::Installed,
    line_waiter::{LineWaiter, WaitQueue},
    messages::{self, PluginExternalRequest, PluginMethodMessage, PluginResponse},
    rx_lines::RxLines,
    stdlib::{self, STDLIB},
    suite,
};
use crate::{
    error, info,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
        task::{Shared, Task},
    },
    interfaces::{InterfaceCommand, InterfaceShared, InterfaceType, rtt_if::RttCommand},
    success, warning,
};
use chrono::Local;
use regex::Regex;
use std::{
    collections::HashMap,
    ops::Deref,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock, mpsc::Sender},
    time::{Duration, Instant},
};
use std::{path::Path, sync::mpsc::Receiver};
use tokio::{
    runtime::Runtime,
    task::{self, yield_now},
    time::sleep,
};
pub type PluginEngine = Task<(), PluginEngineCommand>;

/// The directory scope copies loaded plugins into (alongside the bundled
/// `scope.lua`): `<config_dir>/scope/plugins`, e.g. `~/.config/scope/plugins`.
/// Falls back to a relative `scope/plugins` when the config dir is unknown,
/// mirroring the crash-backup directory.
pub fn plugins_dir() -> PathBuf {
    dirs::config_dir()
        .map(|dir| dir.join("scope").join("plugins"))
        .unwrap_or_else(|| PathBuf::from("scope").join("plugins"))
}

/// Stage a plugin for loading: ensure the plugins directory and the bundled
/// standard libraries are up to date, copy the plugin in (unless the source
/// already *is* the destination), and return the path to load it from.
fn stage_plugin(dir: &Path, source: &Path, name: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("Cannot create plugins directory {:?}: {}", dir, err))?;

    stdlib::provision(dir)?;

    let dest = dir.join(format!("{}.lua", name));
    let same_file = match (std::fs::canonicalize(source), std::fs::canonicalize(&dest)) {
        (Ok(s), Ok(d)) => s == d,
        _ => false,
    };
    if !same_file {
        std::fs::copy(source, &dest)
            .map_err(|err| format!("Cannot copy plugin to {:?}: {}", dest, err))?;
    }

    Ok(dest)
}

pub enum PluginEngineCommand {
    SetLogLevel {
        plugin_name: String,
        log_level: LogLevel,
    },
    LoadPlugin {
        filepath: String,
    },
    /// Load the plugin now (like `LoadPlugin`) and record it in the persistent
    /// manifest so every subsequent session auto-loads it (issue #36).
    InstallPlugin {
        filepath: String,
    },
    /// Remove a plugin from the persistent manifest and delete its staged copy,
    /// so it no longer auto-loads at start-up (issue #37). Does not unload a
    /// running instance — that is `UnloadPlugin`.
    UninstallPlugin {
        plugin_name: String,
    },
    /// Log the names of the currently installed (auto-loaded) plugins.
    ListInstalledPlugins,
    UnloadPlugin {
        plugin_name: String,
    },
    UserCommand {
        plugin_name: String,
        command: String,
        options: Vec<String>,
    },
    SerialConnected {
        port: String,
        baudrate: u32,
    },
    SerialDisconnected {
        port: String,
        baudrate: u32,
    },
    RttConnected {
        target: String,
        channel: usize,
    },
    RttDisconnected {
        target: String,
        channel: usize,
    },
    RttReadResult {
        plugin_name: Arc<String>,
        method_id: u64,
        err: String,
        data: Vec<u8>,
    },
    /// `!test` with its arguments, resolved by [`suite::select`].
    RunTests {
        args: Vec<String>,
    },
    Exit,
}

pub struct PluginEngineConnections {
    logger: Logger,
    tx_producer: Producer<Arc<TimedBytes>>,
    tx_consumer: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    interface_shared: Shared<InterfaceShared>,
    latency: u64,
    interface_type: InterfaceType,
    interface_cmd_sender: Sender<InterfaceCommand>,
    plugins_dir: PathBuf,
}

impl PluginEngine {
    pub fn spawn_plugin_engine(
        connections: PluginEngineConnections,
        sender: std::sync::mpsc::Sender<PluginEngineCommand>,
        receiver: std::sync::mpsc::Receiver<PluginEngineCommand>,
    ) -> Self {
        Self::new((), connections, Self::task, sender, receiver)
    }

    pub fn task(
        shared: Arc<RwLock<()>>,
        private: PluginEngineConnections,
        cmd_receiver: Receiver<PluginEngineCommand>,
    ) {
        let rt = Runtime::new().expect("Cannot create tokio runtime");

        rt.block_on(async {
            let local = task::LocalSet::new();

            local
                .run_until(async move {
                    Self::task_async(shared, private, cmd_receiver).await;
                })
                .await;
        });
    }

    pub async fn task_async(
        _shared: Arc<RwLock<()>>,
        private: PluginEngineConnections,
        cmd_receiver: Receiver<PluginEngineCommand>,
    ) {
        let mut plugin_list: HashMap<Arc<String>, Plugin> = HashMap::new();
        let mut engine_gate = PluginEngineGate::new(32);
        let mut interface_recv_reqs: Vec<PluginMethodMessage<PluginExternalRequest>> = vec![];
        let mut rtt_read_reqs: Vec<PluginMethodMessage<PluginExternalRequest>> = vec![];
        let mut waits: WaitQueue<u64> = WaitQueue::default();
        let mut running_suites: HashMap<Arc<String>, u64> = HashMap::new();
        let mut rx_lines = RxLines::default();
        let hooks = RecvHooks::of(&private.interface_type);
        let err_regex = Regex::new(r#".*: \[string ".*"]:"#).unwrap();

        // Auto-load every installed plugin (issue #36) before entering the
        // command loop, so a user's persistent set is ready without manual
        // `!plugin load`s. Each load logs per plugin; a missing/broken installed
        // plugin is reported and skipped rather than aborting start-up.
        Self::load_installed_plugins(
            &mut engine_gate,
            &mut plugin_list,
            &private.logger,
            &err_regex,
            &private.plugins_dir,
        )
        .await;

        'plugin_engine_loop: loop {
            if let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    PluginEngineCommand::Exit => break 'plugin_engine_loop,
                    PluginEngineCommand::SetLogLevel {
                        plugin_name,
                        log_level,
                    } => {
                        let Some(plugin) = plugin_list.get_mut(&plugin_name) else {
                            error!(private.logger, "Plugin \"{}\" not loaded", plugin_name);
                            continue 'plugin_engine_loop;
                        };

                        plugin.set_log_level(log_level);

                        success!(
                            private.logger,
                            "Log level setted to {:?}, on plugin {}",
                            log_level,
                            plugin_name
                        );
                    }
                    PluginEngineCommand::LoadPlugin { filepath } => {
                        let Some(plugin_name) = Self::get_plugin_name(&filepath) else {
                            continue 'plugin_engine_loop;
                        };

                        if let Some(plugin) = plugin_list.get_mut(&plugin_name) {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_unload",
                                (),
                                false,
                            );
                            plugin.set_unload_mode(PluginUnloadMode::Reload);

                            continue 'plugin_engine_loop;
                        }

                        let Ok(filepath) = PathBuf::from_str(&filepath);

                        let plugin_name = Arc::new(plugin_name);

                        match Self::load_plugin(
                            engine_gate.new_method_call_gate(),
                            plugin_name.clone(),
                            filepath,
                            &mut plugin_list,
                            private.logger.clone(),
                            &private.plugins_dir,
                        )
                        .await
                        {
                            Ok(_) => success!(private.logger, "Plugin \"{}\" loaded", plugin_name),
                            Err(err) => error!(private.logger, "{}", err_regex.replace(&err, "")),
                        }
                    }
                    PluginEngineCommand::InstallPlugin { filepath } => {
                        let Some(plugin_name) = Self::get_plugin_name(&filepath) else {
                            continue 'plugin_engine_loop;
                        };

                        let dir = &private.plugins_dir;

                        // Read the manifest up front so a corrupt file surfaces
                        // here instead of silently dropping the install.
                        let mut installed = match Installed::load(dir) {
                            Ok(installed) => installed,
                            Err(err) => {
                                error!(private.logger, "{}", err);
                                continue 'plugin_engine_loop;
                            }
                        };

                        // Make sure the plugin is loaded this session. If it
                        // isn't yet, load it now and bail on failure so a broken
                        // plugin is never persisted into the auto-load set.
                        if !plugin_list.contains_key(&plugin_name) {
                            let Ok(source) = PathBuf::from_str(&filepath);

                            if let Err(err) = Self::load_plugin(
                                engine_gate.new_method_call_gate(),
                                Arc::new(plugin_name.clone()),
                                source,
                                &mut plugin_list,
                                private.logger.clone(),
                                &private.plugins_dir,
                            )
                            .await
                            {
                                error!(private.logger, "{}", err_regex.replace(&err, ""));
                                continue 'plugin_engine_loop;
                            }
                        }

                        // Persist. `add` dedupes, so re-installing is idempotent.
                        let newly_added = installed.add(&plugin_name);
                        if let Err(err) = installed.save(dir) {
                            error!(private.logger, "{}", err);
                            continue 'plugin_engine_loop;
                        }

                        if newly_added {
                            success!(private.logger, "Plugin \"{}\" installed", plugin_name);
                        } else {
                            success!(
                                private.logger,
                                "Plugin \"{}\" is already installed",
                                plugin_name
                            );
                        }
                    }
                    PluginEngineCommand::UninstallPlugin { plugin_name } => {
                        // Accept `foo`, `foo.lua`, or a path — normalize to the
                        // bare name the manifest stores.
                        let Some(plugin_name) = Self::get_plugin_name(&plugin_name) else {
                            continue 'plugin_engine_loop;
                        };

                        let dir = &private.plugins_dir;

                        let mut installed = match Installed::load(dir) {
                            Ok(installed) => installed,
                            Err(err) => {
                                error!(private.logger, "{}", err);
                                continue 'plugin_engine_loop;
                            }
                        };

                        if !installed.remove(&plugin_name) {
                            error!(
                                private.logger,
                                "Plugin \"{}\" is not installed", plugin_name
                            );
                            continue 'plugin_engine_loop;
                        }
                        if let Err(err) = installed.save(dir) {
                            error!(private.logger, "{}", err);
                            continue 'plugin_engine_loop;
                        }

                        // Delete the staged copy so it's gone from the plugins
                        // folder. A missing file is fine (already gone); any other
                        // error is a non-fatal warning — the manifest, which drives
                        // auto-load, is already updated. A bundled stdlib file is
                        // never deleted: `!plugin install` rejects reserved names,
                        // so a manifest listing `scope`/`shell` can only come from a
                        // hand-edited file, and losing a user-customized `scope.lua`
                        // to it would be a nasty surprise (the manifest entry is
                        // still cleaned above).
                        let staged_name = format!("{}.lua", plugin_name);
                        let is_stdlib = STDLIB.iter().any(|(lib, _)| *lib == staged_name);
                        let staged = dir.join(&staged_name);
                        if !is_stdlib
                            && let Err(err) = std::fs::remove_file(&staged)
                            && err.kind() != std::io::ErrorKind::NotFound
                        {
                            warning!(
                                private.logger,
                                "Could not delete staged plugin {:?}: {}",
                                staged,
                                err
                            );
                        }

                        // A running instance is intentionally left loaded for this
                        // session (use `!plugin unload` to stop it now); uninstall
                        // only removes it from the persistent auto-load set.
                        success!(private.logger, "Plugin \"{}\" uninstalled", plugin_name);
                    }
                    PluginEngineCommand::ListInstalledPlugins => {
                        match Installed::load(&private.plugins_dir) {
                            Ok(installed) if installed.names().is_empty() => {
                                info!(private.logger, "No plugins installed");
                            }
                            Ok(installed) => {
                                info!(
                                    private.logger,
                                    "Installed plugins: {}",
                                    installed.names().join(", ")
                                );
                            }
                            Err(err) => error!(private.logger, "{}", err),
                        }
                    }
                    PluginEngineCommand::UnloadPlugin { plugin_name } => {
                        let Some(plugin) = plugin_list.get_mut(&plugin_name) else {
                            error!(private.logger, "Plugin \"{}\" not loaded", plugin_name);
                            continue 'plugin_engine_loop;
                        };

                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            "on_unload",
                            (),
                            false,
                        );
                        plugin.set_unload_mode(PluginUnloadMode::Unload);
                    }
                    PluginEngineCommand::UserCommand {
                        plugin_name,
                        command,
                        options,
                    } => {
                        let Some(plugin) = plugin_list.get_mut(&plugin_name) else {
                            error!(private.logger, "Plugin \"{}\" not loaded", plugin_name);
                            continue 'plugin_engine_loop;
                        };

                        if !plugin.is_user_command_valid(&command) {
                            error!(
                                private.logger,
                                "Plugin \"{}\" doesn't have \"{}\" command", plugin_name, command
                            );
                            continue 'plugin_engine_loop;
                        }

                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            &command,
                            options,
                            true,
                        );
                    }
                    PluginEngineCommand::SerialConnected { port, baudrate } => {
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_serial_connect",
                                [port.clone(), baudrate.to_string()],
                                true,
                            );
                        }
                    }
                    PluginEngineCommand::SerialDisconnected { port, baudrate } => {
                        rx_lines.reset();
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_serial_disconnect",
                                [port.clone(), baudrate.to_string()],
                                true,
                            );
                        }
                    }
                    PluginEngineCommand::RttConnected { target, channel } => {
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_rtt_connect",
                                [target.clone(), channel.to_string()],
                                true,
                            );
                        }
                    }
                    PluginEngineCommand::RttDisconnected { target, channel } => {
                        rx_lines.reset();
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_rtt_disconnect",
                                [target.clone(), channel.to_string()],
                                true,
                            );
                        }
                    }
                    PluginEngineCommand::RunTests { args } => {
                        let catalog = plugin_list
                            .iter()
                            .map(|(name, plugin)| (name.clone(), plugin.tests()))
                            .collect::<Vec<_>>();
                        let selection = match suite::select(&args, &catalog) {
                            Ok(selection) => selection,
                            Err(err) => {
                                error!(private.logger, "{}", err);
                                continue 'plugin_engine_loop;
                            }
                        };

                        if running_suites.contains_key(&selection.plugin) {
                            error!(
                                private.logger,
                                "A test suite is already running on plugin \"{}\"",
                                selection.plugin
                            );
                            continue 'plugin_engine_loop;
                        }

                        let Some(plugin) = plugin_list.get_mut(&selection.plugin) else {
                            continue 'plugin_engine_loop;
                        };
                        match plugin
                            .spawn_test_suite(engine_gate.new_method_call_gate(), selection.tests)
                        {
                            Some(method_id) => {
                                running_suites.insert(selection.plugin, method_id);
                            }
                            None => error!(
                                private.logger,
                                "Plugin \"{}\" is being unloaded", selection.plugin
                            ),
                        }
                    }
                    PluginEngineCommand::RttReadResult {
                        plugin_name,
                        method_id,
                        err,
                        data,
                    } => {
                        let rtt_read_reqs_filtered = rtt_read_reqs
                            .extract_if(
                                ..,
                                |PluginMethodMessage {
                                     plugin_name: req_plugin_name,
                                     method_id: req_method_id,
                                     ..
                                 }| {
                                    plugin_name == *req_plugin_name && method_id == *req_method_id
                                },
                            )
                            .collect::<Vec<_>>();

                        for PluginMethodMessage { method_id, .. } in rtt_read_reqs_filtered {
                            let rsp = PluginResponse::RttRead {
                                err: err.clone(),
                                data: data.clone(),
                            };

                            engine_gate.respond(method_id, rsp);
                        }
                    }
                }
            }

            // Drain the bus: headless publishes one message per received byte,
            // and taking a single one per pass would let the line hook fall
            // behind the wire.
            while let Ok(rx_msg) = private.rx.try_recv() {
                // Deprecated: gets the message in whatever shape the interface
                // published it, which differs between the TUI and headless.
                for plugin in plugin_list.values_mut() {
                    let data = RecvData {
                        bytes: rx_msg.message.clone(),
                        as_string: plugin.recv_as_string(),
                    };
                    plugin.spawn_method_call(
                        engine_gate.new_method_call_gate(),
                        hooks.legacy,
                        data,
                        false,
                    );
                }

                // The two hooks below see the same stream in both modes. They
                // are only spawned for plugins that define them: the byte hook
                // runs once per received byte, and an undefined hook would
                // still cost a task per call.
                for plugin in plugin_list.values_mut() {
                    if !plugin.defines(hooks.byte) {
                        continue;
                    }
                    for &byte in rx_msg.message.iter() {
                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            hooks.byte,
                            byte,
                            false,
                        );
                    }
                }

                let framed = rx_lines.push(&rx_msg.message);
                if framed.dropped > 0 {
                    warning!(
                        private.logger,
                        "Dropped {} received bytes without a line break; {} only gets complete lines",
                        framed.dropped,
                        hooks.line
                    );
                }
                for plugin in plugin_list.values_mut() {
                    if !plugin.defines(hooks.line) {
                        continue;
                    }
                    let as_string = plugin.recv_as_string();
                    for line in framed.lines.iter() {
                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            hooks.line,
                            RecvData {
                                bytes: line.clone(),
                                as_string,
                            },
                            false,
                        );
                    }
                }

                for line in framed.lines.iter() {
                    if let Some((method_id, result)) = waits.feed(line) {
                        engine_gate.respond(method_id, PluginResponse::SendAndWait(result));
                    }
                }

                for interface_recv_req in interface_recv_reqs.drain(..) {
                    let method_id = interface_recv_req.method_id;

                    let rsp = match private.interface_type {
                        InterfaceType::Serial => PluginResponse::SerialRecv {
                            err: "".to_string(),
                            message: rx_msg.message.clone(),
                        },
                        InterfaceType::Rtt => PluginResponse::RttRecv {
                            err: "".to_string(),
                            message: rx_msg.message.clone(),
                        },
                    };

                    engine_gate.respond(method_id, rsp);
                }
            }

            interface_recv_reqs.retain(
                |PluginMethodMessage {
                     data, method_id, ..
                 }| {
                    match data {
                        PluginExternalRequest::RttRecv { timeout } => {
                            let InterfaceType::Rtt = private.interface_type else {
                                engine_gate.respond(*method_id, PluginResponse::RttRecv {
                                        err: "wrong interface".to_string(),
                                        message: vec![],
                                    });
                                return false;
                            };

                            if let Some(t) = timeout
                                && Instant::now() >= *t
                            {
                                engine_gate.respond(*method_id, PluginResponse::RttRecv {
                                        err: "timeout".to_string(),
                                        message: vec![],
                                    });

                                return false;
                            }

                            true
                        }
                        PluginExternalRequest::SerialRecv { timeout } => {
                            let InterfaceType::Serial = private.interface_type else {
                                let rsp = PluginResponse::SerialRecv {
                                    err: "wrong interface".to_string(),
                                    message: vec![],
                                };
                                engine_gate.respond(*method_id, rsp);
                                return false;
                            };

                            if let Some(t) = timeout
                                && Instant::now() >= *t
                            {
                                engine_gate.respond(*method_id, PluginResponse::SerialRecv {
                                        err: "timeout".to_string(),
                                        message: vec![],
                                    });

                                return false;
                            }

                            true
                        }
                        _ => unreachable!("Only recv requests should be in interface_recv_reqs. This is a bug. Please, report it."),
                    }
                },
            );

            rtt_read_reqs.retain(
                |PluginMethodMessage {
                     method_id, data, ..
                 }| {
                    let PluginExternalRequest::RttRead { timeout, .. } = data else {
                        unreachable!("RttRead requests should be the only ones in rtt_read_reqs. This is a bug. Please, report it.");
                    };

                    let InterfaceType::Rtt = private.interface_type else {
                        engine_gate.respond(*method_id, PluginResponse::RttRead {
                                err: "wrong interface".to_string(),
                                data: vec![],
                            });
                        return false;
                    };

                    if let Some(t) = timeout
                        && Instant::now() >= *t
                    {
                        engine_gate.respond(*method_id, PluginResponse::RttRead {
                                err: "timeout".to_string(),
                                data: vec![],
                            });

                        return false;
                    }

                    true
                },
            );

            if let Some(method_id) = waits.expire(Instant::now()) {
                engine_gate.respond(
                    method_id,
                    PluginResponse::SendAndWait(Err("timeout".to_string())),
                );
            }

            send_next_wait(&mut waits, &private);

            while let Ok(PluginMethodMessage {
                plugin_name,
                method_id,
                data,
            }) = engine_gate.receiver.try_recv()
            {
                let finished = matches!(data, PluginExternalRequest::Finish { .. });
                if finished {
                    engine_gate.finished(method_id);
                    if running_suites.get(&plugin_name) == Some(&method_id) {
                        running_suites.remove(&plugin_name);
                    }
                }

                if !plugin_list
                    .get(&plugin_name)
                    .is_some_and(|plugin| plugin.owns(method_id))
                {
                    if !finished {
                        engine_gate.respond(method_id, PluginResponse::Unloaded);
                    }
                    continue;
                }

                let mut plugin = plugin_list
                    .remove(&plugin_name)
                    .expect("the plugin owning the call was just found");
                if finished {
                    plugin.finished(method_id);
                }

                let rsp = match data {
                    super::messages::PluginExternalRequest::SerialInfo => {
                        let (port, baudrate) = {
                            let interface_shared = private
                                .interface_shared
                                .read()
                                .expect("Cannot get interface lock for read");
                            match interface_shared.deref() {
                                InterfaceShared::Serial(serial_shared) => {
                                    (serial_shared.port.clone(), serial_shared.baudrate)
                                }
                                _ => {
                                    warning!(
                                        private.logger,
                                        "Plugin requested :serial.info but the active interface is not Serial; returning empty port and baudrate 0"
                                    );
                                    ("".to_string(), 0)
                                }
                            }
                        };

                        Some(PluginResponse::SerialInfo { port, baudrate })
                    }
                    super::messages::PluginExternalRequest::SerialSend { message } => {
                        match private.interface_type {
                            InterfaceType::Serial => private.transmit(message),
                            _ => {
                                warning!(
                                    private.logger,
                                    "Plugin requested :serial.send but the active interface is not Serial."
                                );
                            }
                        }

                        Some(PluginResponse::SerialSend)
                    }
                    super::messages::PluginExternalRequest::SerialRecv { timeout } => {
                        match private.interface_type {
                            InterfaceType::Serial => {
                                if timeout.is_some_and(|t| Instant::now() >= t) {
                                    Some(PluginResponse::SerialRecv {
                                        err: "timeout".to_string(),
                                        message: vec![],
                                    })
                                } else {
                                    interface_recv_reqs.push(PluginMethodMessage {
                                        plugin_name: plugin_name.clone(),
                                        method_id,
                                        data: PluginExternalRequest::SerialRecv { timeout },
                                    });

                                    None
                                }
                            }
                            _ => {
                                warning!(
                                    private.logger,
                                    "Plugin requested :serial.recv but the active interface is not Serial."
                                );

                                Some(PluginResponse::SerialRecv {
                                    err: "Plugin requested :serial.recv but the active interface is not Serial.".to_string(),
                                    message: vec![],
                                })
                            }
                        }
                    }
                    super::messages::PluginExternalRequest::RttInfo => {
                        let (target, channel) = {
                            let interface_shared = private
                                .interface_shared
                                .read()
                                .expect("Cannot get interface lock for read");
                            match interface_shared.deref() {
                                InterfaceShared::Rtt(rtt_shared) => {
                                    (rtt_shared.target.clone(), rtt_shared.channel)
                                }
                                _ => {
                                    warning!(
                                        private.logger,
                                        "Plugin requested :rtt.info but the active interface is not RTT; returning empty target and channel 0"
                                    );
                                    ("".to_string(), 0)
                                }
                            }
                        };

                        Some(PluginResponse::RttInfo { target, channel })
                    }
                    super::messages::PluginExternalRequest::RttSend { message } => {
                        match private.interface_type {
                            InterfaceType::Rtt => private.transmit(message),
                            _ => {
                                warning!(
                                    private.logger,
                                    "Plugin requested :rtt.send but the active interface is not RTT."
                                );
                            }
                        }

                        Some(PluginResponse::RttSend)
                    }
                    super::messages::PluginExternalRequest::RttRecv { timeout } => {
                        match private.interface_type {
                            InterfaceType::Rtt => {
                                if timeout.is_some_and(|t| Instant::now() >= t) {
                                    Some(PluginResponse::RttRecv {
                                        err: "timeout".to_string(),
                                        message: vec![],
                                    })
                                } else {
                                    interface_recv_reqs.push(PluginMethodMessage {
                                        plugin_name: plugin_name.clone(),
                                        method_id,
                                        data: PluginExternalRequest::RttRecv { timeout },
                                    });

                                    None
                                }
                            }
                            _ => {
                                warning!(
                                    private.logger,
                                    "Plugin requested :rtt.recv but the active interface is not RTT."
                                );

                                Some(PluginResponse::RttRecv {
                                    err: "Plugin requested :rtt.recv but the active interface is not RTT.".to_string(),
                                    message: vec![],
                                })
                            }
                        }
                    }
                    super::messages::PluginExternalRequest::RttRead {
                        timeout,
                        plugin_name,
                        method_id,
                        address,
                        size,
                    } => match private.interface_type {
                        InterfaceType::Rtt => {
                            let res = private.interface_cmd_sender.send(InterfaceCommand::Rtt(
                                RttCommand::PluginRead {
                                    plugin_name: plugin_name.clone(),
                                    method_id,
                                    address,
                                    size,
                                },
                            ));

                            match res {
                                Ok(_) => {
                                    if timeout.is_some_and(|t| Instant::now() >= t) {
                                        Some(PluginResponse::RttRead {
                                            err: "timeout".to_string(),
                                            data: vec![],
                                        })
                                    } else {
                                        rtt_read_reqs.push(PluginMethodMessage {
                                            plugin_name: plugin_name.clone(),
                                            method_id,
                                            data: PluginExternalRequest::RttRead {
                                                timeout,
                                                plugin_name,
                                                method_id,
                                                address,
                                                size,
                                            },
                                        });

                                        None
                                    }
                                }
                                Err(err) => {
                                    error!(
                                        private.logger,
                                        "Failed to send RTT read command to interface: {}", err
                                    );
                                    Some(PluginResponse::RttRead {
                                        err: "Failed to send RTT read command to interface"
                                            .to_string(),
                                        data: vec![],
                                    })
                                }
                            }
                        }
                        _ => {
                            warning!(
                                private.logger,
                                "Plugin requested :rtt.read but the active interface is not RTT."
                            );

                            Some(PluginResponse::RttRead {
                                    err: "Plugin requested :rtt.read but the active interface is not RTT.".to_string(),
                                    data: vec![],
                                })
                        }
                    },
                    PluginExternalRequest::SendAndWait {
                        interface,
                        message,
                        spec,
                    } => {
                        if interface != private.interface_type {
                            let module = RecvHooks::of(&interface).prefix;
                            let err = format!(
                                "Plugin requested :{module}.send_and_wait but the active interface is not {module}."
                            );
                            warning!(private.logger, "{}", err);
                            Some(PluginResponse::SendAndWait(Err(err)))
                        } else {
                            match LineWaiter::new(&spec) {
                                Ok(waiter) => {
                                    waits.push(method_id, message, waiter);
                                    send_next_wait(&mut waits, &private);
                                    None
                                }
                                Err(err) => Some(PluginResponse::SendAndWait(Err(err))),
                            }
                        }
                    }
                    super::messages::PluginExternalRequest::Log {
                        level,
                        message,
                        plugin_name,
                        id,
                    } => {
                        if level as u32 <= plugin.log_level() as u32 {
                            let _ = private.logger.write_with_source_id(
                                message,
                                level,
                                plugin_name,
                                id,
                            );
                        }

                        Some(PluginResponse::Log)
                    }
                    messages::PluginExternalRequest::Finish { fn_name } => {
                        if fn_name.as_str() == "on_unload" {
                            let calls = plugin.calls().clone();
                            for method_id in waits.cancel(|method_id| calls.contains(method_id)) {
                                engine_gate.respond(method_id, PluginResponse::Unloaded);
                            }
                            for reqs in [&mut interface_recv_reqs, &mut rtt_read_reqs] {
                                reqs.retain(|req| {
                                    if !calls.contains(&req.method_id) {
                                        return true;
                                    }
                                    engine_gate.respond(req.method_id, PluginResponse::Unloaded);
                                    false
                                });
                            }

                            if let PluginUnloadMode::Reload = plugin.unload_mode() {
                                match Self::load_plugin(
                                    engine_gate.new_method_call_gate(),
                                    plugin_name.clone(),
                                    plugin.filepath(),
                                    &mut plugin_list,
                                    private.logger.clone(),
                                    &private.plugins_dir,
                                )
                                .await
                                {
                                    Ok(_) => success!(
                                        private.logger,
                                        "Plugin \"{}\" reloaded",
                                        plugin_name
                                    ),
                                    Err(err) => {
                                        error!(private.logger, "{}", err_regex.replace(&err, ""));
                                    }
                                }
                            } else {
                                warning!(private.logger, "Plugin \"{}\" unloaded", plugin_name);
                            }
                        } else {
                            plugin_list.insert(plugin_name.clone(), plugin);
                        }

                        /* don't yield here! Because this request doesn't have response and we don't want to reinsert the plugin. */
                        continue;
                    }
                };

                plugin_list.insert(plugin_name.clone(), plugin);

                let Some(rsp) = rsp else {
                    /* don't yield here! Because the request doesn't have response. */
                    continue;
                };

                engine_gate.respond(method_id, rsp);
            }

            send_next_wait(&mut waits, &private);

            if let Ok(tx_msg) = private.tx_consumer.try_recv() {
                for plugin in plugin_list.values_mut() {
                    let fn_name = match private.interface_type {
                        InterfaceType::Rtt => "on_rtt_send",
                        InterfaceType::Serial => "on_serial_send",
                    };
                    plugin.spawn_method_call(
                        engine_gate.new_method_call_gate(),
                        fn_name,
                        tx_msg.message.clone(),
                        false,
                    );
                }
            }

            if private.latency > 0 {
                sleep(Duration::from_micros(private.latency)).await;
            } else {
                yield_now().await;
            }
        }
    }

    fn get_plugin_name(filepath: &str) -> Option<String> {
        Path::new(filepath)
            .with_extension("")
            .file_name()
            .and_then(|filename| filename.to_str())
            .map(|filename| filename.to_string())
    }

    /// Load every plugin recorded in the install manifest, staging each from its
    /// copy in the plugins directory. A malformed manifest, a missing file, or a
    /// broken plugin is logged and skipped so a bad entry can't stop scope from
    /// starting. Each successful load logs per plugin, mirroring `!plugin load`.
    async fn load_installed_plugins(
        engine_gate: &mut PluginEngineGate,
        plugin_list: &mut HashMap<Arc<String>, Plugin>,
        logger: &Logger,
        err_regex: &Regex,
        dir: &Path,
    ) {
        let installed = match Installed::load(dir) {
            Ok(installed) => installed,
            Err(err) => {
                error!(logger, "{}", err);
                return;
            }
        };

        for name in installed.names() {
            // Load from the staged copy in the plugins directory; `load_plugin`
            // re-stages (a no-op when source == dest) and loads it.
            let source = dir.join(format!("{}.lua", name));
            match Self::load_plugin(
                engine_gate.new_method_call_gate(),
                Arc::new(name.clone()),
                source,
                plugin_list,
                logger.clone(),
                dir,
            )
            .await
            {
                Ok(_) => success!(logger, "Plugin \"{}\" loaded", name),
                Err(err) => error!(logger, "{}", err_regex.replace(&err, "")),
            }
        }
    }

    async fn load_plugin(
        gate: PluginMethodCallGate,
        plugin_name: Arc<String>,
        filepath: PathBuf,
        plugin_list: &mut HashMap<Arc<String>, Plugin>,
        logger: Logger,
        dir: &Path,
    ) -> Result<(), String> {
        // These names belong to the embedded standard-library modules the engine
        // writes into the plugins directory; a plugin may not shadow them.
        if matches!(plugin_name.as_str(), "scope" | "shell" | "test") {
            return Err(format!("\"{}\" is a reserved plugin name", plugin_name));
        }

        let source = match filepath.extension() {
            Some(extension) if extension.as_encoded_bytes() != b"lua" => {
                return Err(format!("Invalid plugin extension: {:?}", extension));
            }
            Some(_extension) => filepath,
            None => filepath.with_extension("lua"),
        };

        if !source.exists() {
            return Err(format!("Filepath \"{:?}\" doesn't exist!", source));
        }

        // Stage the plugin into the known plugins directory next to the bundled
        // standard libraries and load it from there, so `require("scope")` /
        // `require("shell")` resolve without the user staging them by hand
        // (issue #206). The original path is kept as the plugin's identity so
        // `!plugin reload` re-copies the (possibly edited) source.
        let dest = stage_plugin(dir, &source, plugin_name.as_str())?;

        let mut plugin = Plugin::new(
            plugin_name.clone(),
            source,
            dest,
            logger.clone().with_source((*plugin_name).clone()),
        )?;
        for (legacy, byte, line) in RecvHooks::DEPRECATED {
            if plugin.defines(legacy) {
                warning!(
                    logger,
                    "Plugin \"{}\": {} is deprecated, as the message it gets is split differently in the TUI and in headless mode. Use {} (whole lines) or {} (single bytes) instead",
                    plugin_name,
                    legacy,
                    line,
                    byte
                );
            }
        }
        plugin.spawn_method_call(gate, "on_load", (), false);

        plugin_list.insert(plugin_name.clone(), plugin);

        Ok(())
    }
}

/// Sends the `send_and_wait` message whose turn has come, if any.
fn send_next_wait(waits: &mut WaitQueue<u64>, private: &PluginEngineConnections) {
    if let Some(message) = waits.next_to_send(Instant::now()) {
        private.transmit(message);
    }
}

/// The plugin hooks that receive data from the active interface (issue #250).
struct RecvHooks {
    /// The interface's module in `scope.lua` (`serial`, `rtt`).
    prefix: &'static str,
    /// Deprecated: called with each message on the RX bus as the interface
    /// published it — a line in the TUI, a byte (serial) or a read chunk (RTT)
    /// in headless.
    legacy: &'static str,
    /// Called once per received byte, with the byte as a number.
    byte: &'static str,
    /// Called once per received line, `\n` included; a partial line is never
    /// delivered.
    line: &'static str,
}

impl RecvHooks {
    const SERIAL: Self = Self {
        prefix: "serial",
        legacy: "on_serial_recv",
        byte: "on_serial_recv_byte",
        line: "on_serial_recv_line",
    };
    const RTT: Self = Self {
        prefix: "rtt",
        legacy: "on_rtt_recv",
        byte: "on_rtt_recv_byte",
        line: "on_rtt_recv_line",
    };
    /// Every deprecated hook with its replacements, as `(legacy, byte, line)`.
    const DEPRECATED: [(&'static str, &'static str, &'static str); 2] = [
        (Self::SERIAL.legacy, Self::SERIAL.byte, Self::SERIAL.line),
        (Self::RTT.legacy, Self::RTT.byte, Self::RTT.line),
    ];

    fn of(interface_type: &InterfaceType) -> Self {
        match interface_type {
            InterfaceType::Serial => Self::SERIAL,
            InterfaceType::Rtt => Self::RTT,
        }
    }
}

impl PluginEngineConnections {
    /// Sends `message` on the active interface, as the plugin engine.
    fn transmit(&self, message: Vec<u8>) {
        self.tx_producer.produce_without_loopback(
            Arc::new(TimedBytes {
                timestamp: Local::now(),
                message,
            }),
            self.tx_consumer.id(),
        );
    }

    // One argument per channel end or setting `main` wires in; bundling them
    // would only move the same list into a struct literal.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        logger: Logger,
        tx_producer: Producer<Arc<TimedBytes>>,
        tx_consumer: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
        interface_shared: Shared<InterfaceShared>,
        latency: u64,
        interface_type: InterfaceType,
        interface_cmd_sender: Sender<InterfaceCommand>,
        plugins_dir: PathBuf,
    ) -> Self {
        Self {
            logger,
            tx_producer,
            tx_consumer,
            rx,
            interface_shared,
            latency,
            interface_type,
            interface_cmd_sender,
            plugins_dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PluginEngine, PluginEngineCommand, PluginEngineConnections, STDLIB, stage_plugin};
    use crate::infra::{
        logger::{LogMessage, Logger},
        messages::TimedBytes,
        mpmc::{Channel, Consumer, Producer},
        task::Shared,
    };
    use crate::interfaces::{
        InterfaceShared, InterfaceType,
        serial_if::{SerialSetup, SerialShared},
    };
    use chrono::Local;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, mpsc};
    use std::time::{Duration, Instant};

    /// A self-cleaning unique temp directory (this crate has no tempfile dev-dep).
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("scope-stage-test-{tag}"));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn stdlib(name: &str) -> &'static str {
        STDLIB.iter().find(|(n, _)| *n == name).unwrap().1
    }

    #[test]
    fn stage_provisions_stdlib_and_copies_plugin() {
        let tmp = TempDir::new("provision");
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        let source = src.join("myplug.lua");
        fs::write(&source, "-- plugin body\n").unwrap();

        let plugins = tmp.path().join("plugins");
        let dest = stage_plugin(&plugins, &source, "myplug").unwrap();

        assert_eq!(dest, plugins.join("myplug.lua"));
        assert_eq!(fs::read_to_string(&dest).unwrap(), "-- plugin body\n");
        assert_eq!(
            fs::read_to_string(plugins.join("scope.lua")).unwrap(),
            stdlib("scope.lua")
        );
        assert_eq!(
            fs::read_to_string(plugins.join("shell.lua")).unwrap(),
            stdlib("shell.lua")
        );
        assert_eq!(
            fs::read_to_string(plugins.join("codec").join("at.lua")).unwrap(),
            stdlib("codec/at.lua")
        );
    }

    #[test]
    fn stage_rewrites_a_stale_stdlib() {
        let tmp = TempDir::new("stale-stdlib");
        let plugins = tmp.path().join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        fs::write(plugins.join("scope.lua"), "-- OLD\n").unwrap();

        let source = tmp.path().join("p.lua");
        fs::write(&source, "-- p\n").unwrap();
        stage_plugin(&plugins, &source, "p").unwrap();

        for (name, _) in STDLIB {
            assert_eq!(
                fs::read_to_string(plugins.join(name)).unwrap(),
                stdlib(name),
                "{name}"
            );
        }
    }

    #[test]
    fn restage_recopies_edited_source() {
        let tmp = TempDir::new("recopy");
        let plugins = tmp.path().join("plugins");
        let source = tmp.path().join("p.lua");

        fs::write(&source, "-- v1\n").unwrap();
        let dest = stage_plugin(&plugins, &source, "p").unwrap();
        assert_eq!(fs::read_to_string(&dest).unwrap(), "-- v1\n");

        // Editing the source and re-staging re-copies it (the reload path).
        fs::write(&source, "-- v2\n").unwrap();
        let dest2 = stage_plugin(&plugins, &source, "p").unwrap();
        assert_eq!(dest2, dest);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "-- v2\n");
    }

    #[test]
    fn stage_is_noop_copy_when_source_is_destination() {
        let tmp = TempDir::new("same-file");
        let plugins = tmp.path().join("plugins");
        fs::create_dir_all(&plugins).unwrap();
        // Loading directly from the plugins dir: source == destination.
        let source = plugins.join("p.lua");
        fs::write(&source, "-- inplace\n").unwrap();

        let dest = stage_plugin(&plugins, &source, "p").unwrap();
        assert_eq!(dest, source);
        assert_eq!(fs::read_to_string(&dest).unwrap(), "-- inplace\n");
    }

    const MODEM_PLUGIN: &str = r#"
local scope = require("scope")
local at = scope.codec.at
local serial = scope.serial
local test = scope.test

local M = {
    config = {
        send_prefix = "modem at",
        ignore_msg = { "uart.*" },
        error_pattern = "\\+CME ERROR: .*",
        default_timeout = 2000,
    },
}

function M.test_cops()
    local lines, matched = serial.send_and_wait("AT+COPS?")
    test.assert(lines, matched)
    test.assert_eq(matched, "OK")
    test.assert_eq(at.decode(lines).provider, "TIMBrasil")
end

function M.test_error()
    test.assert_eq({ serial.send_and_wait("AT+CSQ") }, { nil, "+CME ERROR: 10" })
end

function M.test_timeout()
    test.assert_eq({ serial.send_and_wait("AT+SILENT", "OK", 100) }, { nil, "timeout" })
end

return M
"#;

    fn modem_reply(command: &[u8]) -> Option<&'static [u8]> {
        match command {
            b"modem at AT+COPS?\r\n" => {
                Some(b"uart:~$ modem at AT+COPS?\r\n\r\n+COPS: 0,0,\"TIMBrasil\",7\r\n\r\nOK\r\n")
            }
            b"modem at AT+CGATT?\r\n" => Some(b"uart:~$ modem at AT+CGATT?\r\n+CGATT: 1\r\nOK\r\n"),
            b"modem at AT+CSQ\r\n" => Some(b"uart:~$ modem at AT+CSQ\r\n+CME ERROR: 10\r\n"),
            _ => None,
        }
    }

    /// The plugin engine on a temporary plugins directory, with a fake modem
    /// answering on the RX bus what the engine sends on the TX bus.
    struct Rig {
        tmp: TempDir,
        sender: mpsc::Sender<PluginEngineCommand>,
        engine: Option<PluginEngine>,
        wire: Consumer<Arc<TimedBytes>>,
        device: Producer<Arc<TimedBytes>>,
        logs: mpsc::Receiver<LogMessage>,
        seen: Vec<String>,
        sent: Vec<Vec<u8>>,
    }

    impl Rig {
        fn new(tag: &str) -> Self {
            let tmp = TempDir::new(tag);
            let mut tx = Channel::default();
            let wire = tx.new_consumer();
            let engine_tx = tx.new_consumer();
            let tx = Arc::new(tx);
            let mut rx = Channel::default();
            let engine_rx = rx.new_consumer();
            let device = Arc::new(rx).new_producer();

            let (logger, logs) = Logger::new("test".to_string());
            let (interface_sender, _) = mpsc::channel();
            let (sender, receiver) = mpsc::channel();
            let engine = PluginEngine::spawn_plugin_engine(
                PluginEngineConnections::new(
                    logger,
                    tx.new_producer(),
                    engine_tx,
                    engine_rx,
                    Shared::new(InterfaceShared::Serial(SerialShared::new(
                        SerialSetup::default(),
                    ))),
                    100,
                    InterfaceType::Serial,
                    interface_sender,
                    tmp.path().join("plugins"),
                ),
                sender.clone(),
                receiver,
            );

            Self {
                tmp,
                sender,
                engine: Some(engine),
                wire,
                device,
                logs,
                seen: vec![],
                sent: vec![],
            }
        }

        fn load(&mut self, name: &str, code: &str) {
            let source = self.tmp.path().join(format!("{name}.lua"));
            fs::write(&source, code).unwrap();
            self.send(PluginEngineCommand::LoadPlugin {
                filepath: source.to_str().unwrap().to_string(),
            });
            self.wait_for_log(&format!("Plugin \"{name}\" loaded"));
        }

        fn send(&self, command: PluginEngineCommand) {
            self.sender.send(command).unwrap();
        }

        fn test(&self, args: &[&str]) {
            self.send(PluginEngineCommand::RunTests {
                args: args.iter().map(|arg| arg.to_string()).collect(),
            });
        }

        fn wait_until(&mut self, what: &str, done: impl Fn(&Self) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !done(self) {
                assert!(
                    Instant::now() < deadline,
                    "no {what} in {:#?}\nsent: {:?}",
                    self.seen,
                    self.sent
                );
                while let Ok(command) = self.wire.try_recv() {
                    if let Some(reply) = modem_reply(&command.message) {
                        self.device.produce(Arc::new(TimedBytes {
                            timestamp: Local::now(),
                            message: reply.to_vec(),
                        }));
                    }
                    self.sent.push(command.message.clone());
                }
                self.seen
                    .extend(self.logs.try_iter().map(|log| log.message));
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn wait_for_log(&mut self, text: &str) {
            self.wait_until(&format!("\"{text}\""), |rig| {
                rig.seen.iter().any(|line| line.contains(text))
            });
        }

        fn report(&self, source: &str) -> Vec<&String> {
            self.seen
                .iter()
                .filter(|line| line.starts_with(source))
                .collect()
        }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            if let Some(engine) = self.engine.take() {
                let _ = self.sender.send(PluginEngineCommand::Exit);
                engine.join();
            }
        }
    }

    #[test]
    fn a_test_suite_talks_to_the_device_through_send_and_wait() {
        let mut rig = Rig::new("suite");
        rig.load("modem", MODEM_PLUGIN);

        rig.test(&["modem"]);
        rig.wait_for_log("passed");

        assert_eq!(
            rig.report("[modem:test]"),
            [
                "[modem:test] [1/3] cops: pass",
                "[modem:test] [2/3] error: pass",
                "[modem:test] [3/3] timeout: pass",
                "[modem:test] 3 passed, 0 failed, 0 skipped",
            ],
            "{:#?}",
            rig.seen
        );
    }

    const SILENT_PLUGIN: &str = r#"
local serial = require("scope").serial

local M = {}

function M.test_silent()
    serial.send_and_wait("AT+SILENT", "OK", 60000)
end

return M
"#;

    #[test]
    fn unloading_a_plugin_mid_suite_lets_its_tests_run_again() {
        let mut rig = Rig::new("unload-mid-suite");
        let silent = b"modem at AT+SILENT\r\n".to_vec();
        let sent_silent = |rig: &Rig| rig.sent.iter().filter(|sent| **sent == silent).count();

        rig.load(
            "modem",
            &SILENT_PLUGIN.replace(
                "local M = {}",
                "local M = { config = { send_prefix = \"modem at\" } }",
            ),
        );
        rig.test(&["modem"]);
        rig.wait_until("AT+SILENT on the wire", |rig| sent_silent(rig) == 1);

        rig.send(PluginEngineCommand::UnloadPlugin {
            plugin_name: "modem".to_string(),
        });
        rig.wait_for_log("[modem:test] Stopped: the plugin was unloaded");

        rig.load(
            "modem",
            &SILENT_PLUGIN.replace(
                "local M = {}",
                "local M = { config = { send_prefix = \"modem at\" } }",
            ),
        );
        rig.test(&["modem"]);
        rig.wait_until("AT+SILENT on the wire again", |rig| sent_silent(rig) == 2);

        assert!(
            !rig.seen.iter().any(|line| line.contains("already running")),
            "{:#?}",
            rig.seen
        );
    }

    const CGATT_PLUGIN: &str = r#"
local scope = require("scope")
local test = scope.test

local M = { config = { send_prefix = "modem at", ignore_msg = { "uart.*" } } }

function M.test_cgatt()
    local lines = scope.serial.send_and_wait("AT+CGATT?")
    test.assert_eq(lines, { "+CGATT: 1" })
end

return M
"#;

    #[test]
    fn send_and_wait_calls_of_two_plugins_take_turns() {
        let mut rig = Rig::new("two-plugins");
        rig.load("cgatt", CGATT_PLUGIN);
        rig.load("modem", MODEM_PLUGIN);

        rig.test(&["cgatt"]);
        rig.test(&["modem", "cops"]);
        rig.wait_until("both summaries", |rig| {
            rig.seen
                .iter()
                .filter(|line| line.contains(" passed, "))
                .count()
                == 2
        });

        assert_eq!(
            rig.report("[cgatt:test]"),
            [
                "[cgatt:test] [1/1] cgatt: pass",
                "[cgatt:test] 1 passed, 0 failed, 0 skipped",
            ],
            "{:#?}",
            rig.seen
        );
        assert_eq!(
            rig.report("[modem:test]"),
            [
                "[modem:test] [1/1] cops: pass",
                "[modem:test] 1 passed, 0 failed, 0 skipped",
            ],
            "{:#?}",
            rig.seen
        );
    }
}
