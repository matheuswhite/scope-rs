use crate::messages::{SerialRxData, UserTxData};
use crate::plugin::{Plugin, PluginRequest, SerialRxCall, UserCommandCall};
use crate::serial::SerialIF;
use crate::text::TextView;
use chrono::Local;
use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader, Lines, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
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
        let (serial_rx_tx, serial_rx_rx) = std::sync::mpsc::channel::<(String, SerialRxCall)>();
        let (user_command_tx, user_command_rx) =
            std::sync::mpsc::channel::<(String, UserCommandCall)>();

        let (text_view2, interface2) = (text_view.clone(), interface.clone());

        std::thread::spawn(move || 'user_command: loop {
            let Ok((plugin_name, user_command_call)) = user_command_rx.recv() else {
                break 'user_command;
            };

            for req in user_command_call {
                PluginManager::exec_plugin_request(
                    text_view.clone(),
                    interface.clone(),
                    plugin_name.clone(),
                    req,
                );
            }
        });

        std::thread::spawn(move || 'serial_rx: loop {
            let Ok((plugin_name, serial_rx_call)) = serial_rx_rx.recv() else {
                break 'serial_rx;
            };

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

    fn plugin_println<B: Backend + Sync + Send + 'static>(
        text_view: Arc<Mutex<TextView<B>>>,
        plugin_name: String,
        content: String,
    ) {
        let mut text_view = text_view.lock().unwrap();
        text_view.add_data_out(SerialRxData::Plugin {
            timestamp: Local::now(),
            plugin_name,
            content,
            is_successful: true,
        })
    }

    fn plugin_eprintln<B: Backend + Sync + Send + 'static>(
        text_view: Arc<Mutex<TextView<B>>>,
        plugin_name: String,
        content: String,
    ) {
        let mut text_view = text_view.lock().unwrap();
        text_view.add_data_out(SerialRxData::Plugin {
            timestamp: Local::now(),
            plugin_name,
            content,
            is_successful: false,
        })
    }

    fn spawn_read_pipe<P>(
        is_end: &'static AtomicBool,
        mut pipe: Lines<BufReader<P>>,
        mut print_fn: impl FnMut(String) + Send + 'static,
    ) where
        P: Read + Send + 'static,
    {
        std::thread::spawn(move || {
            while is_end.load(Ordering::SeqCst) {
                if let Some(Ok(line)) = pipe.next() {
                    print_fn(line);
                    // Self::plugin_println(text_view.clone(), plugin_name.clone(), line);
                }
            }

            'read_loop: loop {
                let stderr_next = pipe.next();

                if stderr_next.is_none() {
                    break 'read_loop;
                }

                if let Some(Ok(line)) = stderr_next {
                    print_fn(line);
                    // Self::plugin_println(text_view.clone(), plugin_name.clone(), line);
                }
            }
        });
    }

    fn exec_process<B: Backend + Sync + Send + 'static>(
        text_view: Arc<Mutex<TextView<B>>>,
        plugin_name: String,
        cmd: String,
    ) -> Result<(), String> {
        Self::plugin_println(text_view.clone(), plugin_name.clone(), cmd.clone());

        let mut child = if cfg!(target_os = "windows") {
            unimplemented!()
        } else {
            Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|err| err.to_string())?
        };

        static IS_END: AtomicBool = AtomicBool::new(false);
        let text_view2 = text_view.clone();
        let plugin_name2 = plugin_name.clone();

        let stdout = child.stdout.take().ok_or("Cannot get stdout".to_string())?;
        let stdout = BufReader::new(stdout).lines();
        Self::spawn_read_pipe(&IS_END, stdout, move |line| {
            Self::plugin_println(text_view.clone(), plugin_name.clone(), line)
        });

        let stderr = child.stderr.take().ok_or("Cannot get stderr".to_string())?;
        let stderr = BufReader::new(stderr).lines();
        Self::spawn_read_pipe(&IS_END, stderr, move |line| {
            Self::plugin_eprintln(text_view2.clone(), plugin_name2.clone(), line)
        });

        let _ = child.wait();
        IS_END.store(true, Ordering::SeqCst);

        Ok(())
    }

    fn exec_plugin_request<B: Backend + Sync + Send + 'static>(
        text_view: Arc<Mutex<TextView<B>>>,
        interface: Arc<Mutex<SerialIF>>,
        plugin_name: String,
        req: PluginRequest,
    ) {
        match req {
            PluginRequest::Println { msg } => {
                Self::plugin_println(text_view, plugin_name, msg);
            }
            PluginRequest::Eprintln { msg } => {
                Self::plugin_eprintln(text_view, plugin_name, msg);
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
                Self::exec_process(text_view, plugin_name, cmd).unwrap();
            }
        }
    }
}
