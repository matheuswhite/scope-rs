use super::{
    bridge::{PluginEngineGate, PluginMethodCallGate},
    messages::{self, PluginExternalRequest, PluginMethodMessage, PluginResponse},
    plugin::{Plugin, PluginUnloadMode},
};
use crate::{
    error,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
        task::{Shared, Task},
    },
    serial::serial_if::SerialShared,
    success,
};
use chrono::Local;
use std::{
    collections::HashMap,
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    str::FromStr,
    sync::{Arc, RwLock},
    time::Instant,
};
use std::{path::Path, sync::mpsc::Receiver};
use tokio::{
    runtime::Runtime,
    task::{self, yield_now},
};
pub type PluginEngine = Task<(), PluginEngineCommand>;

pub enum PluginEngineCommand {
    SetLogLevel {
        plugin_name: String,
        log_level: LogLevel,
    },
    LoadPlugin {
        filepath: String,
    },
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
    Exit,
}

pub struct PluginEngineConnections {
    logger: Logger,
    tx_producer: Producer<Arc<TimedBytes>>,
    tx_consumer: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    serial_shared: Shared<SerialShared>,
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

        let _ = rt.block_on(async {
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
        let mut serial_recv_reqs = vec![];

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
                            );
                            plugin.set_unload_mode(super::plugin::PluginUnloadMode::Reload);

                            continue 'plugin_engine_loop;
                        }

                        let Ok(filepath) = PathBuf::from_str(&filepath) else {
                            error!(private.logger, "Invalid filepath {}", filepath);
                            continue 'plugin_engine_loop;
                        };

                        let plugin_name = Arc::new(plugin_name);

                        match Self::load_plugin(
                            engine_gate.new_method_call_gate(),
                            plugin_name.clone(),
                            filepath,
                            &mut plugin_list,
                            private.logger.clone(),
                        )
                        .await
                        {
                            Ok(_) => {
                                success!(private.logger, "Plugin \"{}\" loaded", plugin_name);
                            }
                            Err(err) => error!(private.logger, "{}", err),
                        }
                    }
                    PluginEngineCommand::UnloadPlugin { plugin_name } => {
                        let Some(plugin) = plugin_list.get_mut(&plugin_name) else {
                            continue 'plugin_engine_loop;
                        };

                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            "on_unload",
                            (),
                        );
                        plugin.set_unload_mode(super::plugin::PluginUnloadMode::Unload);
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

                        plugin.spawn_method_call(
                            engine_gate.new_method_call_gate(),
                            &command,
                            options,
                        );
                    }
                    PluginEngineCommand::SerialConnected { port, baudrate } => {
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_serial_connect",
                                (port.clone(), baudrate),
                            );
                        }
                    }
                    PluginEngineCommand::SerialDisconnected { port, baudrate } => {
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_serial_disconnect",
                                (port.clone(), baudrate),
                            );
                        }
                    }
                }
            }

            while let Ok(PluginMethodMessage {
                plugin_name,
                method_id,
                data,
            }) = engine_gate.receiver.try_recv()
            {
                let Some(plugin) = plugin_list.remove(&plugin_name) else {
                    continue;
                };

                let rsp = match data {
                    super::messages::PluginExternalRequest::SerialInfo => {
                        let (port, baudrate) = {
                            let serial_shared = private
                                .serial_shared
                                .read()
                                .expect("Cannot get serial shared for read");
                            (serial_shared.port.clone(), serial_shared.baudrate)
                        };

                        Some(PluginResponse::SerialInfo { port, baudrate })
                    }
                    super::messages::PluginExternalRequest::SerialSend { message } => {
                        let id = private.tx_consumer.id();
                        private.tx_producer.produce_without_loopback(
                            Arc::new(TimedBytes {
                                timestamp: Local::now(),
                                message,
                            }),
                            id,
                        );

                        Some(PluginResponse::SerialSend)
                    }
                    super::messages::PluginExternalRequest::SerialRecv { timeout } => {
                        if Instant::now() >= timeout {
                            Some(PluginResponse::SerialRecv {
                                err: "timeout".to_string(),
                                message: vec![],
                            })
                        } else {
                            serial_recv_reqs.push(PluginMethodMessage {
                                plugin_name: plugin_name.clone(),
                                method_id,
                                data: PluginExternalRequest::SerialRecv { timeout },
                            });

                            None
                        }
                    }
                    super::messages::PluginExternalRequest::Log { level, message } => {
                        if level as u32 <= plugin.log_level() as u32 {
                            let _ = private.logger.write(message, level);
                        }

                        Some(PluginResponse::Log)
                    }
                    messages::PluginExternalRequest::Finish { fn_name } => {
                        if fn_name.as_str() == "on_unload" {
                            if matches!(plugin.unload_mode(), PluginUnloadMode::Reload) {
                                success!(private.logger, "Plugin \"{}\" unloaded", plugin_name);
                                if let Err(err) = Self::load_plugin(
                                    engine_gate.new_method_call_gate(),
                                    plugin_name.clone(),
                                    plugin.filepath(),
                                    &mut plugin_list,
                                    private.logger.clone(),
                                )
                                .await
                                {
                                    error!(private.logger, "{}", err);
                                }
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

                let _ = engine_gate.sender.send(PluginMethodMessage {
                    plugin_name,
                    method_id,
                    data: rsp,
                });
            }

            if let Ok(tx_msg) = private.tx_consumer.try_recv() {
                for plugin in plugin_list.values_mut() {
                    plugin.spawn_method_call(
                        engine_gate.new_method_call_gate(),
                        "on_serial_send",
                        tx_msg.message.clone(),
                    );
                }
            }

            serial_recv_reqs = serial_recv_reqs
                .into_iter()
                .filter(|PluginMethodMessage { data, .. }| {
                    if let PluginExternalRequest::SerialRecv { timeout } = data {
                        Instant::now() < *timeout
                    } else {
                        false
                    }
                })
                .collect();

            if let Ok(rx_msg) = private.rx.try_recv() {
                for plugin in plugin_list.values_mut() {
                    plugin.spawn_method_call(
                        engine_gate.new_method_call_gate(),
                        "on_serial_recv",
                        rx_msg.message.clone(),
                    );
                }

                for serial_recv_req in serial_recv_reqs.drain(..) {
                    let PluginMethodMessage {
                        plugin_name,
                        method_id,
                        ..
                    } = serial_recv_req;

                    let _ = engine_gate.sender.send(PluginMethodMessage {
                        plugin_name,
                        method_id,
                        data: PluginResponse::SerialRecv {
                            err: "".to_string(),
                            message: rx_msg.message.clone(),
                        },
                    });
                }
            }

            yield_now().await;
        }
    }

    fn get_plugin_name(filepath: &str) -> Option<String> {
        Path::new(filepath)
            .with_extension("")
            .file_name()
            .map(|filename| filename.to_str())
            .flatten()
            .map(|filename| filename.to_string())
    }

    async fn load_plugin(
        gate: PluginMethodCallGate,
        plugin_name: Arc<String>,
        filepath: PathBuf,
        plugin_list: &mut HashMap<Arc<String>, Plugin>,
        logger: Logger,
    ) -> Result<(), String> {
        let filepath = match filepath.extension() {
            Some(extension) if extension.as_bytes() != b"lua" => {
                return Err(format!("Invalid plugin extension: {:?}", extension));
            }
            Some(_extension) => filepath,
            None => filepath.with_extension("lua"),
        };

        if !filepath.exists() {
            return Err(format!("Filepath \"{:?}\" doesn't exist!", filepath));
        }

        let mut plugin = Plugin::new(plugin_name.clone(), filepath, logger)?;
        plugin.spawn_method_call(gate, "on_load", ());

        plugin_list.insert(plugin_name, plugin);

        Ok(())
    }
}

impl PluginEngineConnections {
    pub fn new(
        logger: Logger,
        tx_producer: Producer<Arc<TimedBytes>>,
        tx_consumer: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
        serial_shared: Shared<SerialShared>,
    ) -> Self {
        Self {
            logger,
            tx_producer,
            tx_consumer,
            rx,
            serial_shared,
        }
    }
}
