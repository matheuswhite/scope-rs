use super::{
    Plugin, PluginUnloadMode,
    bridge::{PluginEngineGate, PluginMethodCallGate},
    messages::{self, PluginExternalRequest, PluginMethodMessage, PluginResponse},
};
use crate::{
    debug, error,
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
    RttConnected {
        target: String,
        channel: usize,
    },
    RttDisconnected {
        target: String,
        channel: usize,
    },
    RttReadResult {
        err: String,
        data: Vec<u8>,
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
        let mut interface_recv_reqs = vec![];
        let mut rtt_read_reqs = vec![];
        let err_regex = Regex::new(r#".*: \[string ".*"]:"#).unwrap();

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
                        )
                        .await
                        {
                            Ok(_) => success!(private.logger, "Plugin \"{}\" loaded", plugin_name),
                            Err(err) => error!(private.logger, "{}", err_regex.replace(&err, "")),
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
                        for plugin in plugin_list.values_mut() {
                            plugin.spawn_method_call(
                                engine_gate.new_method_call_gate(),
                                "on_rtt_disconnect",
                                [target.clone(), channel.to_string()],
                                true,
                            );
                        }
                    }
                    PluginEngineCommand::RttReadResult { err, data } => {
                        debug!(
                            private.logger,
                            "Received RTT read result from interface, err: {}, data length: {}",
                            err,
                            data.len()
                        );
                        for rtt_read_req in rtt_read_reqs.drain(..) {
                            let PluginMethodMessage {
                                plugin_name,
                                method_id,
                                ..
                            } = rtt_read_req;

                            let rsp = PluginResponse::RttRead {
                                err: err.clone(),
                                data: data.clone(),
                            };

                            let _ = engine_gate.sender.send(PluginMethodMessage {
                                plugin_name,
                                method_id,
                                data: rsp,
                            });
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
                            interface_recv_reqs.push(PluginMethodMessage {
                                plugin_name: plugin_name.clone(),
                                method_id,
                                data: PluginExternalRequest::SerialRecv { timeout },
                            });

                            None
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
                        let id = private.tx_consumer.id();
                        private.tx_producer.produce_without_loopback(
                            Arc::new(TimedBytes {
                                timestamp: Local::now(),
                                message,
                            }),
                            id,
                        );

                        Some(PluginResponse::RttSend)
                    }
                    super::messages::PluginExternalRequest::RttRecv { timeout } => {
                        if Instant::now() >= timeout {
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
                    super::messages::PluginExternalRequest::RttRead { address, size } => {
                        let _ = private.interface_cmd_sender.send(InterfaceCommand::Rtt(
                            RttCommand::PluginRead { address, size },
                        ));
                        debug!(
                            private.logger,
                            "Plugin requested RTT read at address {:#X} with size {}, sending command to interface",
                            address,
                            size
                        );

                        rtt_read_reqs.push(PluginMethodMessage {
                            plugin_name: plugin_name.clone(),
                            method_id,
                            data: PluginExternalRequest::RttRead { address, size },
                        });
                        debug!(
                            private.logger,
                            "Plugin RTT read request queued, waiting for result from interface"
                        );

                        None
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
                            if let PluginUnloadMode::Reload = plugin.unload_mode() {
                                match Self::load_plugin(
                                    engine_gate.new_method_call_gate(),
                                    plugin_name.clone(),
                                    plugin.filepath(),
                                    &mut plugin_list,
                                    private.logger.clone(),
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
                        false,
                    );
                }
            }

            interface_recv_reqs.retain(|PluginMethodMessage { data, .. }| {
                if let PluginExternalRequest::SerialRecv { timeout } = data {
                    Instant::now() < *timeout
                } else {
                    false
                }
            });

            if let Ok(rx_msg) = private.rx.try_recv() {
                let fn_name = match private.interface_type {
                    InterfaceType::Serial => "on_serial_recv",
                    InterfaceType::Rtt => "on_rtt_recv",
                };

                for plugin in plugin_list.values_mut() {
                    plugin.spawn_method_call(
                        engine_gate.new_method_call_gate(),
                        fn_name,
                        rx_msg.message.clone(),
                        false,
                    );
                }

                for interface_recv_req in interface_recv_reqs.drain(..) {
                    let PluginMethodMessage {
                        plugin_name,
                        method_id,
                        ..
                    } = interface_recv_req;

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

                    let _ = engine_gate.sender.send(PluginMethodMessage {
                        plugin_name,
                        method_id,
                        data: rsp,
                    });
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

    async fn load_plugin(
        gate: PluginMethodCallGate,
        plugin_name: Arc<String>,
        filepath: PathBuf,
        plugin_list: &mut HashMap<Arc<String>, Plugin>,
        logger: Logger,
    ) -> Result<(), String> {
        let filepath = match filepath.extension() {
            Some(extension) if extension.as_encoded_bytes() != b"lua" => {
                return Err(format!("Invalid plugin extension: {:?}", extension));
            }
            Some(_extension) => filepath,
            None => filepath.with_extension("lua"),
        };

        if !filepath.exists() {
            return Err(format!("Filepath \"{:?}\" doesn't exist!", filepath));
        }

        let mut plugin = Plugin::new(
            plugin_name.clone(),
            filepath,
            logger.with_source((*plugin_name).clone()),
        )?;
        plugin.spawn_method_call(gate, "on_load", (), false);

        plugin_list.insert(plugin_name.clone(), plugin);

        Ok(())
    }
}

impl PluginEngineConnections {
    pub fn new(
        logger: Logger,
        tx_producer: Producer<Arc<TimedBytes>>,
        tx_consumer: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
        interface_shared: Shared<InterfaceShared>,
        latency: u64,
        interface_type: InterfaceType,
        interface_cmd_sender: Sender<InterfaceCommand>,
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
        }
    }
}
