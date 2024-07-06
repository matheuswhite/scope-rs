use crate::messages::{SerialRxData, UserTxData};
use crate::task_bridge::{Task, TaskBridge};
use chrono::Local;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex;
use tokio::time::Instant;

pub type SerialIF = TaskBridge<SerialIFShared, SerialIFSynchronized, UserTxData, SerialRxData>;

pub struct SerialIFShared {
    info: SerialInfo,
    is_connected: Arc<AtomicBool>,
    reconnect: bool,
    is_exit: bool,
    disconnect: Option<SerialInfo>,
}

pub struct SerialIFSynchronized {
    info: SerialInfo,
    is_connected: Arc<AtomicBool>,
}

impl SerialIF {
    pub fn build_and_connect(port: &str, baudrate: u32) -> Self {
        let info = SerialInfo {
            port: port.to_string(),
            baudrate,
        };
        let info2 = info.clone();

        let is_connected = Arc::new(AtomicBool::new(false));
        let is_connected2 = is_connected.clone();

        Self::new::<SerialTask>(
            SerialIFShared {
                is_exit: false,
                is_connected,
                disconnect: None,
                reconnect: true,
                info,
            },
            SerialIFSynchronized {
                info: info2,
                is_connected: is_connected2,
            },
        )
    }

    pub fn get_info(&self) -> SerialInfo {
        self.synchronized().info.clone()
    }

    pub fn is_connected(&self) -> bool {
        self.synchronized().is_connected.load(Ordering::SeqCst)
    }

    pub fn description(&self) -> String {
        let info = &self.synchronized().info;

        format!("Serial {}:{}bps", info.port, info.baudrate)
    }

    pub async fn exit(&self) {
        self.shared().await.is_exit = true;
    }

    pub async fn setup(&mut self, port: Option<String>, baudrate: Option<u32>) {
        self.synchronized_mut().info = {
            let mut shared = self.shared().await;

            if let Some(port) = port {
                shared.info.port = port;
            }

            if let Some(baudrate) = baudrate {
                shared.info.baudrate = baudrate;
            }

            shared.reconnect = true;

            shared.info.clone()
        };
    }

    pub async fn disconnect(&mut self) {
        let mut shared = self.shared().await;
        shared.reconnect = false;
        let info = shared.info.clone();
        let _ = shared.disconnect.insert(info);
    }
}

struct SerialTask;

impl SerialTask {
    const SERIAL_TIMEOUT: Duration = Duration::from_millis(100);

    fn set_is_connected(
        info: SerialInfo,
        is_connected: &AtomicBool,
        to_bridge: &UnboundedSender<SerialRxData>,
        val: bool,
    ) {
        is_connected.store(val, Ordering::SeqCst);

        if is_connected.load(Ordering::SeqCst) {
            to_bridge
                .send(SerialRxData::Plugin {
                    is_successful: true,
                    timestamp: Local::now(),
                    plugin_name: "serial".to_string(),
                    content: format!("Connected at \"{}\" with {}bps", info.port, info.baudrate),
                })
                .expect("Cannot forward message read from serial")
        } else {
            to_bridge
                .send(SerialRxData::Plugin {
                    is_successful: true,
                    timestamp: Local::now(),
                    plugin_name: "serial".to_string(),
                    content: format!(
                        "Disconnected from \"{}\" with {}bps",
                        info.port, info.baudrate
                    ),
                })
                .expect("Cannot forward message read from serial");
        }
    }
}

impl Task<SerialIFShared, UserTxData, SerialRxData> for SerialTask {
    async fn run(
        shared: Arc<Mutex<SerialIFShared>>,
        mut from_bridge: UnboundedReceiver<UserTxData>,
        to_bridge: UnboundedSender<SerialRxData>,
    ) {
        let mut line = vec![];
        let mut buffer = [0u8];
        let mut now = Instant::now();
        let mut serial_wrapper = None;

        'task: loop {
            let mut shared = shared.lock().await;

            if shared.is_exit {
                break 'task;
            }

            if let Some(old_serial_info) = shared.disconnect.take() {
                let _ = serial_wrapper.take();
                Self::set_is_connected(old_serial_info, &shared.is_connected, &to_bridge, false);
                continue 'task;
            }

            if shared.reconnect {
                let _ = serial_wrapper.take();
                shared.is_connected.store(false, Ordering::SeqCst);
                let Ok(serial) = tokio_serial::new(&shared.info.port, shared.info.baudrate)
                    .data_bits(tokio_serial::DataBits::Eight)
                    .flow_control(tokio_serial::FlowControl::None)
                    .parity(tokio_serial::Parity::None)
                    .stop_bits(tokio_serial::StopBits::One)
                    .timeout(Self::SERIAL_TIMEOUT)
                    .open()
                else {
                    continue 'task;
                };

                Self::set_is_connected(shared.info.clone(), &shared.is_connected, &to_bridge, true);
                shared.reconnect = false;
                let _ = serial_wrapper.insert(serial);
            }

            let Some(mut serial) = serial_wrapper.take() else {
                continue 'task;
            };

            if let Ok(data_to_send) = from_bridge.try_recv() {
                match data_to_send {
                    UserTxData::Data { content } => {
                        let content = format!("{content}\r\n");

                        match serial.write_all(content.as_bytes()) {
                            Ok(_) => {
                                to_bridge
                                    .send(SerialRxData::TxData {
                                        timestamp: Local::now(),
                                        content,
                                        is_successful: true,
                                    })
                                    .expect("Cannot send data confirm");
                            }
                            Err(err) => {
                                to_bridge
                                    .send(SerialRxData::TxData {
                                        timestamp: Local::now(),
                                        content: content + &err.to_string(),
                                        is_successful: false,
                                    })
                                    .expect("Cannot send data fail");
                            }
                        }
                    }
                    UserTxData::Command {
                        command_name,
                        content,
                    } => {
                        let content = format!("{content}\r\n");

                        match serial.write_all(content.as_bytes()) {
                            Ok(_) => {
                                to_bridge
                                    .send(SerialRxData::Command {
                                        timestamp: Local::now(),
                                        command_name,
                                        content,
                                        is_successful: true,
                                    })
                                    .expect("Cannot send command confirm");
                            }
                            Err(_) => {
                                to_bridge
                                    .send(SerialRxData::Command {
                                        timestamp: Local::now(),
                                        command_name,
                                        content,
                                        is_successful: false,
                                    })
                                    .expect("Cannot send command fail");
                            }
                        }
                    }
                    UserTxData::HexString { content } => match serial.write(&content) {
                        Ok(_) => to_bridge
                            .send(SerialRxData::HexString {
                                timestamp: Local::now(),
                                content,
                                is_successful: true,
                            })
                            .expect("Cannot send hex string comfirm"),
                        Err(_) => to_bridge
                            .send(SerialRxData::HexString {
                                timestamp: Local::now(),
                                content,
                                is_successful: false,
                            })
                            .expect("Cannot send hex string fail"),
                    },
                    UserTxData::PluginSerialTx {
                        plugin_name,
                        content,
                    } => match serial.write(&content) {
                        Ok(_) => to_bridge
                            .send(SerialRxData::PluginSerialTx {
                                timestamp: Local::now(),
                                plugin_name,
                                content,
                                is_successful: true,
                            })
                            .expect("Cannot send plugin serial tx comfirm"),
                        Err(_) => to_bridge
                            .send(SerialRxData::PluginSerialTx {
                                timestamp: Local::now(),
                                plugin_name,
                                content,
                                is_successful: false,
                            })
                            .expect("Cannot send plugin serial tx fail"),
                    },
                }
            }

            match serial.read(&mut buffer) {
                Ok(_) => {
                    now = Instant::now();
                    line.push(buffer[0]);
                    if buffer[0] == b'\n' {
                        to_bridge
                            .send(SerialRxData::RxData {
                                timestamp: Local::now(),
                                content: line.clone(),
                            })
                            .expect("Cannot forward message read from serial");
                        line.clear();
                        now = Instant::now();
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(ref e)
                    if e.kind() == io::ErrorKind::PermissionDenied
                        || e.kind() == io::ErrorKind::BrokenPipe =>
                {
                    shared.reconnect = true;
                    continue 'task;
                }
                Err(_) => {}
            }

            if now.elapsed().as_millis() > 1_000 {
                now = Instant::now();

                if !line.is_empty() {
                    to_bridge
                        .send(SerialRxData::RxData {
                            timestamp: Local::now(),
                            content: line.clone(),
                        })
                        .expect("Cannot forward message read from serial");
                    line.clear();
                }
            }

            serial_wrapper = Some(serial);
        }
    }
}

#[derive(Clone)]
pub struct SerialInfo {
    port: String,
    baudrate: u32,
}

impl SerialInfo {
    pub fn port(&self) -> &str {
        &self.port
    }

    pub fn baudrate(&self) -> u32 {
        self.baudrate
    }
}
