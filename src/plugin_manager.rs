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
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

pub struct PluginManager {
    text_view: Arc<Mutex<TextView>>,
    plugins: HashMap<String, Plugin>,
    serial_rx_tx: Sender<(String, SerialRxCall)>,
    user_command_tx: Sender<(String, UserCommandCall)>,
    has_process_running: Arc<AtomicBool>,
    stop_process_flag: Arc<AtomicBool>,
}

impl PluginManager {
    pub fn new(interface: Arc<Mutex<SerialIF>>, text_view: Arc<Mutex<TextView>>) -> Self {
        let (serial_rx_tx, serial_rx_rx) = std::sync::mpsc::channel::<(String, SerialRxCall)>();
        let (user_command_tx, user_command_rx) =
            std::sync::mpsc::channel::<(String, UserCommandCall)>();
        let process_runner = ProcessRunner::new(text_view.clone());

        let (text_view2, interface2, process_runner2) =
            (text_view.clone(), interface.clone(), process_runner.clone());
        let text_view3 = text_view.clone();

        let has_process_running = Arc::new(AtomicBool::new(false));
        let has_process_running2 = has_process_running.clone();
        let has_process_running3 = has_process_running.clone();

        let stop_process_flag = Arc::new(AtomicBool::new(false));
        let stop_process_flag2 = stop_process_flag.clone();
        let stop_process_flag3 = stop_process_flag.clone();

        std::thread::spawn(move || 'user_command: loop {
            let Ok((plugin_name, mut user_command_call)) = user_command_rx.recv() else {
                break 'user_command;
            };

            while let Some(req) = user_command_call.next() {
                let Some(req_result) = PluginManager::exec_plugin_request(
                    text_view.clone(),
                    interface.clone(),
                    plugin_name.clone(),
                    &process_runner,
                    &has_process_running2,
                    stop_process_flag2.clone(),
                    false,
                    req,
                ) else {
                    continue;
                };

                user_command_call.attach_request_result(req_result);
            }
        });

        std::thread::spawn(move || 'serial_rx: loop {
            let Ok((plugin_name, mut serial_rx_call)) = serial_rx_rx.recv() else {
                break 'serial_rx;
            };

            while let Some(req) = serial_rx_call.next() {
                let Some(req_result) = PluginManager::exec_plugin_request(
                    text_view2.clone(),
                    interface2.clone(),
                    plugin_name.clone(),
                    &process_runner2,
                    &has_process_running3,
                    stop_process_flag3.clone(),
                    true,
                    req,
                ) else {
                    continue;
                };

                serial_rx_call.attach_request_result(req_result);
            }
        });

        Self {
            text_view: text_view3,
            plugins: HashMap::new(),
            serial_rx_tx,
            user_command_tx,
            has_process_running,
            stop_process_flag,
        }
    }

    pub fn has_process_running(&self) -> bool {
        self.has_process_running.load(Ordering::SeqCst)
    }

    pub fn stop_process(&mut self) {
        if self.has_process_running() {
            self.stop_process_flag.store(true, Ordering::SeqCst);
            Plugin::eprintln(
                self.text_view.clone(),
                "system".to_string(),
                "Execution stopped".to_string(),
            );
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

    fn exec_plugin_request(
        text_view: Arc<Mutex<TextView>>,
        interface: Arc<Mutex<SerialIF>>,
        plugin_name: String,
        process_runner: &ProcessRunner,
        has_running_process: &AtomicBool,
        stop_process_flag: Arc<AtomicBool>,
        is_from_serial_rx: bool,
        req: PluginRequest,
    ) -> Option<PluginRequestResult> {
        match req {
            PluginRequest::Println { msg } => {
                Plugin::println(text_view, plugin_name, msg);
            }
            PluginRequest::Eprintln { msg } => {
                Plugin::eprintln(text_view, plugin_name, msg);
            }
            PluginRequest::Connect { .. } => {}
            PluginRequest::Disconnect => {}
            PluginRequest::Reconnect => {}
            PluginRequest::SerialTx { msg } => {
                let mut text_view = text_view.lock().unwrap();
                let interface = interface.lock().unwrap();
                interface.send(UserTxData::PluginSerialTx {
                    plugin_name,
                    content: msg,
                });

                'plugin_serial_tx: loop {
                    match interface.recv() {
                        Ok(x) if x.is_plugin_serial_tx() => {
                            text_view.add_data_out(x);
                            break 'plugin_serial_tx;
                        }
                        Ok(x) => text_view.add_data_out(x),
                        Err(_) => {
                            break 'plugin_serial_tx;
                        }
                    }
                }
            }
            PluginRequest::Sleep { time } => std::thread::sleep(time),
            PluginRequest::Exec { cmd } => {
                if !is_from_serial_rx {
                    has_running_process.store(true, Ordering::SeqCst);
                    let res = Some(
                        process_runner
                            .run(plugin_name, cmd, stop_process_flag.clone())
                            .unwrap(),
                    );
                    has_running_process.store(false, Ordering::SeqCst);
                    stop_process_flag.store(false, Ordering::SeqCst);
                    return res;
                } else {
                    Plugin::eprintln(
                        text_view,
                        plugin_name,
                        "Cannot call \"scope.exec\" from \"serial_rx\"".to_string(),
                    )
                }
            }
        }

        None
    }
}
