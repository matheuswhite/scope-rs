use crate::messages::{SerialRxData, UserTxData};
use crate::plugin::{
    Plugin, PluginRequest, PluginRequestResult, PluginRequestResultHolder, SerialRxCall,
    UserCommandCall,
};
use crate::process::ProcessRunner;
use crate::serial::SerialIF;
use crate::text::TextView;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;

pub struct PluginManager {
    text_view: Arc<Mutex<TextView>>,
    plugins: HashMap<String, Plugin>,
    serial_rx_tx: UnboundedSender<(String, SerialRxCall)>,
    user_command_tx: UnboundedSender<(String, UserCommandCall)>,
    flags: PluginManagerFlags,
}

impl PluginManager {
    pub fn new(interface: Arc<Mutex<SerialIF>>, text_view: Arc<Mutex<TextView>>) -> Self {
        let (serial_rx_tx, mut serial_rx_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, SerialRxCall)>();
        let (user_command_tx, mut user_command_rx) =
            tokio::sync::mpsc::unbounded_channel::<(String, UserCommandCall)>();
        let process_runner = ProcessRunner::new(text_view.clone());

        let (text_view2, interface2, process_runner2) =
            (text_view.clone(), interface.clone(), process_runner.clone());
        let text_view3 = text_view.clone();

        let flags = PluginManagerFlags::default();
        let flags2 = flags.clone();
        let flags3 = flags.clone();

        tokio::spawn(async move {
            'user_command: loop {
                let Some((plugin_name, mut user_command_call)) = user_command_rx.recv().await
                else {
                    break 'user_command;
                };

                while let Some(req) = user_command_call.next() {
                    let Some(req_result) = PluginManager::exec_plugin_request(
                        text_view.clone(),
                        interface.clone(),
                        plugin_name.clone(),
                        &process_runner,
                        flags2.clone(),
                        ExecutionOrigin::UserCommand,
                        req,
                    )
                    .await
                    else {
                        continue;
                    };

                    user_command_call.attach_request_result(req_result);
                }
            }
        });

        tokio::spawn(async move {
            'serial_rx: loop {
                let Some((plugin_name, mut serial_rx_call)) = serial_rx_rx.recv().await else {
                    break 'serial_rx;
                };

                while let Some(req) = serial_rx_call.next() {
                    let Some(req_result) = PluginManager::exec_plugin_request(
                        text_view2.clone(),
                        interface2.clone(),
                        plugin_name.clone(),
                        &process_runner2,
                        flags3.clone(),
                        ExecutionOrigin::SerialRx,
                        req,
                    )
                    .await
                    else {
                        continue;
                    };

                    serial_rx_call.attach_request_result(req_result);
                }
            }
        });

        let serial_plugin = Plugin::from_string(
            "serial".to_string(),
            include_str!("../plugins/serial.lua").to_string(),
        )
        .unwrap();

        let mut plugins = HashMap::new();
        plugins.insert(serial_plugin.name().to_string(), serial_plugin);

        Self {
            text_view: text_view3,
            plugins,
            serial_rx_tx,
            user_command_tx,
            flags,
        }
    }

    pub fn has_process_running(&self) -> bool {
        self.flags.has_process_running.load(Ordering::SeqCst)
    }

    pub async fn stop_process(&mut self) {
        if self.has_process_running() {
            self.flags.stop_process.store(true, Ordering::SeqCst);
            Plugin::eprintln(
                self.text_view.clone(),
                "system".to_string(),
                "Execution stopped".to_string(),
            )
            .await;
        }
    }

    pub fn handle_plugin_command(
        &mut self,
        arg_list: Vec<String>,
    ) -> Result<(String, String), String> {
        if arg_list.is_empty() {
            return Err("Please, use !plugin followed by a command".to_string());
        }

        let command = arg_list[0].as_str();
        let mut plugin_path = env::current_dir().expect("Cannot get the current directory");

        let plugin_name = match command {
            "load" => {
                if arg_list.len() < 2 {
                    return Err("Please, inform the plugin path to be loaded".to_string());
                }

                plugin_path.push(PathBuf::from(arg_list[1].as_str()));

                let plugin = match Plugin::new(plugin_path.clone()) {
                    Ok(plugin) => plugin,
                    Err(err) => return Err(err),
                };

                let plugin_name = plugin.name().to_string();

                if self.plugins.contains_key(plugin_name.as_str()) {
                    return Err(format!(
                        "Plugin {} already loaded. Use the reload command instead.",
                        plugin_name
                    ));
                }

                self.plugins.insert(plugin_name.clone(), plugin.clone());

                plugin_name
            }
            "reload" => {
                if arg_list.len() < 2 {
                    return Err("Please, inform the plugin path to be loaded".to_string());
                }

                plugin_path.push(PathBuf::from(arg_list[1].as_str()));

                let plugin = match Plugin::new(plugin_path.clone()) {
                    Ok(plugin) => plugin,
                    Err(err) => return Err(err),
                };

                let plugin_name = plugin.name().to_string();

                if !self.plugins.contains_key(plugin_name.as_str()) {
                    return Err(format!(
                        "Plugin {} already loaded. Use the reload command instead.",
                        plugin_name
                    ));
                } else {
                    self.plugins.remove(plugin_name.as_str());
                }

                self.plugins.insert(plugin_name.clone(), plugin.clone());

                plugin_name
            }
            _ => return Err(format!("Unknown command {} for !plugin", command)),
        };

        Ok((command.to_string(), plugin_name))
    }

    pub fn call_plugin_user_command(
        &self,
        name: &str,
        arg_list: Vec<String>,
    ) -> Result<(), String> {
        if !self.plugins.contains_key(name) {
            return Err(format!("Command <!{}> not found", &name));
        }

        let plugin = self.plugins.get(name).unwrap();
        let user_command_call = plugin.user_command_call(arg_list);

        let plugin_name = plugin.name().to_string();
        self.user_command_tx
            .send((plugin_name, user_command_call))
            .unwrap();

        Ok(())
    }

    pub fn call_plugins_serial_rx(&self, data_out: SerialRxData) {
        for plugin in self.plugins.values().cloned().collect::<Vec<_>>() {
            let SerialRxData::RxData {
                timestamp: _timestamp,
                content: line,
            } = &data_out
            else {
                continue;
            };

            let serial_rx_call = plugin.serial_rx_call(line.clone());
            let plugin_name = plugin.name().to_string();
            self.serial_rx_tx
                .send((plugin_name, serial_rx_call))
                .map_err(|e| e.to_string())
                .unwrap();
        }
    }

    async fn exec_plugin_request(
        text_view: Arc<Mutex<TextView>>,
        interface: Arc<Mutex<SerialIF>>,
        plugin_name: String,
        process_runner: &ProcessRunner,
        flags: PluginManagerFlags,
        origin: ExecutionOrigin,
        req: PluginRequest,
    ) -> Option<PluginRequestResult> {
        match req {
            PluginRequest::Println { msg } => {
                Plugin::println(text_view, plugin_name, msg).await;
            }
            PluginRequest::Eprintln { msg } => {
                Plugin::eprintln(text_view, plugin_name, msg).await;
            }
            PluginRequest::Connect { port, baud_rate } => {
                let mut interface = interface.lock().await;
                if interface.is_connected() && port.is_none() && baud_rate.is_none() {
                    Plugin::eprintln(
                        text_view,
                        plugin_name,
                        "Serial port already connected".to_string(),
                    )
                    .await;
                    return None;
                }

                interface.setup(port, baud_rate).await;
            }
            PluginRequest::Disconnect => {
                let mut interface = interface.lock().await;
                if !interface.is_connected() {
                    Plugin::eprintln(
                        text_view,
                        plugin_name,
                        "Serial port already disconnected".to_string(),
                    )
                    .await;
                    return None;
                }

                interface.disconnect().await;
            }
            PluginRequest::SerialTx { msg } => {
                let mut text_view = text_view.lock().await;
                let mut interface = interface.lock().await;
                interface.send(UserTxData::PluginSerialTx {
                    plugin_name,
                    content: msg,
                });

                'plugin_serial_tx: loop {
                    match interface.recv().await {
                        Some(x) if x.is_plugin_serial_tx() => {
                            text_view.add_data_out(x).await;
                            break 'plugin_serial_tx;
                        }
                        Some(x) => text_view.add_data_out(x).await,
                        None => {
                            break 'plugin_serial_tx;
                        }
                    }
                }
            }
            PluginRequest::Sleep { time } => tokio::time::sleep(time).await,
            PluginRequest::Exec { cmd, quiet } => match origin {
                ExecutionOrigin::SerialRx => {
                    Plugin::eprintln(
                        text_view,
                        plugin_name,
                        "Cannot call \"scope.exec\" from \"serial_rx\"".to_string(),
                    )
                    .await
                }
                ExecutionOrigin::UserCommand => {
                    flags.has_process_running.store(true, Ordering::SeqCst);
                    let res = Some(
                        process_runner
                            .run(plugin_name, cmd, quiet, flags.stop_process.clone())
                            .await
                            .unwrap(),
                    );
                    flags.has_process_running.store(false, Ordering::SeqCst);
                    flags.stop_process.store(false, Ordering::SeqCst);
                    return res;
                }
            },
        }

        None
    }
}

#[derive(Default, Clone)]
struct PluginManagerFlags {
    has_process_running: Arc<AtomicBool>,
    stop_process: Arc<AtomicBool>,
}

enum ExecutionOrigin {
    SerialRx,
    UserCommand,
}
