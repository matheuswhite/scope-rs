use crate::messages::{SerialRxData, UserTxData};
use crate::plugin::{Plugin, PluginRequest, SerialRxCall, UserCommandCall};
use crate::serial::SerialIF;
use crate::text::TextView;
use chrono::Local;
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use tui::backend::Backend;

pub struct PluginManager {
    plugins: HashMap<String, Plugin>,
    serial_rx_tx: Sender<(String, SerialRxCall)>,
    user_command_tx: Sender<(String, UserCommandCall)>,
}

impl PluginManager {
    pub fn new<B: Backend + Sync + Send + 'static>(
        interface: Arc<Mutex<SerialIF>>,
        text_view: Arc<Mutex<TextView<B>>>,
    ) -> Self {
        let (serial_rx_tx, serial_rx_rx) = std::sync::mpsc::channel();
        let (user_command_tx, user_command_rx) = std::sync::mpsc::channel();

        let (text_view2, interface2) = (text_view.clone(), interface.clone());

        std::thread::spawn(move || loop {
            let (plugin_name, user_command_call): (String, UserCommandCall) =
                user_command_rx.recv().unwrap();

            for req in user_command_call {
                PluginManager::exec_plugin_request(
                    text_view.clone(),
                    interface.clone(),
                    plugin_name.clone(),
                    req,
                );
            }
        });

        std::thread::spawn(move || loop {
            let (plugin_name, serial_rx_call): (String, SerialRxCall) =
                serial_rx_rx.recv().unwrap();

            for req in serial_rx_call {
                PluginManager::exec_plugin_request(
                    text_view2.clone(),
                    interface2.clone(),
                    plugin_name.clone(),
                    req,
                );
            }
        });

        Self {
            plugins: HashMap::new(),
            serial_rx_tx,
            user_command_tx,
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
            let SerialRxData::Data(_timestamp, line) = &data_out else {
                continue;
            };

            let serial_rx_call = plugin.serial_rx_call(line.as_bytes().to_vec());
            let plugin_name = plugin.name().to_string();
            self.serial_rx_tx
                .send((plugin_name, serial_rx_call))
                .map_err(|e| e.to_string())
                .unwrap();
        }
    }

    fn exec_plugin_request<B: Backend + Sync + Send + 'static>(
        text_view: Arc<Mutex<TextView<B>>>,
        interface: Arc<Mutex<SerialIF>>,
        plugin_name: String,
        req: PluginRequest,
    ) {
        match req {
            PluginRequest::Println { msg } => {
                let mut text_view = text_view.lock().unwrap();
                text_view.add_data_out(SerialRxData::Plugin(Local::now(), plugin_name, msg))
            }
            PluginRequest::Eprintln { msg } => {
                let mut text_view = text_view.lock().unwrap();
                text_view.add_data_out(SerialRxData::FailPlugin(Local::now(), plugin_name, msg))
            }
            PluginRequest::Connect { .. } => {}
            PluginRequest::Disconnect => {}
            PluginRequest::Reconnect => {}
            PluginRequest::SerialTx { msg } => {
                let mut text_view = text_view.lock().unwrap();
                let interface = interface.lock().unwrap();
                interface.send(UserTxData::PluginSerialTx(plugin_name, msg));

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
        }
    }
}
