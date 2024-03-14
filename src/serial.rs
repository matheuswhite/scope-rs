use crate::messages::{SerialRxData, UserTxData};
use chrono::Local;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio_serial::SerialPort;

pub struct SerialIF {
    serial_tx: UnboundedSender<UserTxData>,
    data_rx: UnboundedReceiver<SerialRxData>,
    port: String,
    baudrate: u32,
    is_connected: Arc<AtomicBool>,
}

impl Drop for SerialIF {
    fn drop(&mut self) {
        let _ = self.serial_tx.send(UserTxData::Exit);
    }
}

impl SerialIF {
    const SERIAL_TIMEOUT: Duration = Duration::from_millis(100);
    const RECONNECT_INTERVAL: Duration = Duration::from_millis(200);

    pub fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    pub fn send(&self, data: UserTxData) {
        self.serial_tx.send(data).unwrap();
    }

    pub fn try_recv(&mut self) -> Result<SerialRxData, TryRecvError> {
        self.data_rx.try_recv()
    }

    pub async fn recv(&mut self) -> Option<SerialRxData> {
        self.data_rx.recv().await
    }

    pub fn description(&self) -> String {
        format!("Serial {}:{}bps", self.port, self.baudrate)
    }

    pub fn new(port: &str, baudrate: u32) -> Self {
        let (serial_tx, serial_rx) = unbounded_channel();
        let (data_tx, data_rx) = unbounded_channel();

        let is_connected = Arc::new(AtomicBool::new(false));

        let port_clone = port.to_string();
        let is_connected_clone = is_connected.clone();
        tokio::task::spawn_local(async move {
            SerialIF::send_task(
                &port_clone,
                baudrate,
                serial_rx,
                data_tx,
                is_connected_clone,
            )
            .await;
        });

        Self {
            serial_tx,
            data_rx,
            port: port.to_string(),
            baudrate,
            is_connected,
        }
    }

    async fn reconnect(
        port: &str,
        baudrate: u32,
        interval: Duration,
        is_connected: Arc<AtomicBool>,
    ) -> Box<dyn SerialPort> {
        'reconnect: loop {
            if let Ok(serial) = tokio_serial::new(port, baudrate)
                .data_bits(tokio_serial::DataBits::Eight)
                .flow_control(tokio_serial::FlowControl::Hardware)
                .parity(tokio_serial::Parity::None)
                .stop_bits(tokio_serial::StopBits::One)
                .timeout(SerialIF::SERIAL_TIMEOUT)
                .open()
            {
                is_connected.store(true, Ordering::SeqCst);
                break 'reconnect serial;
            }
            tokio::time::sleep(interval).await;
        }
    }

    async fn send_task(
        port: &str,
        baudrate: u32,
        mut serial_rx: UnboundedReceiver<UserTxData>,
        data_tx: UnboundedSender<SerialRxData>,
        is_connected: Arc<AtomicBool>,
    ) {
        let mut serial = SerialIF::reconnect(
            port,
            baudrate,
            SerialIF::RECONNECT_INTERVAL,
            is_connected.clone(),
        )
        .await;

        let mut line = vec![];
        let mut buffer = [0u8];
        let mut now = Instant::now();

        'task: loop {
            if let Ok(data_to_send) = serial_rx.try_recv() {
                match data_to_send {
                    UserTxData::Exit => break 'task,
                    UserTxData::Data { content } => {
                        let content = format!("{content}\r\n");

                        match serial.write(content.as_bytes()) {
                            Ok(_) => {
                                data_tx
                                    .send(SerialRxData::TxData {
                                        timestamp: Local::now(),
                                        content,
                                        is_successful: true,
                                    })
                                    .expect("Cannot send data confirm");
                            }
                            Err(err) => {
                                data_tx
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

                        match serial.write(content.as_bytes()) {
                            Ok(_) => {
                                data_tx
                                    .send(SerialRxData::Command {
                                        timestamp: Local::now(),
                                        command_name,
                                        content,
                                        is_successful: true,
                                    })
                                    .expect("Cannot send command confirm");
                            }
                            Err(_) => {
                                data_tx
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
                        Ok(_) => data_tx
                            .send(SerialRxData::HexString {
                                timestamp: Local::now(),
                                content,
                                is_successful: true,
                            })
                            .expect("Cannot send hex string comfirm"),
                        Err(_) => data_tx
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
                        Ok(_) => data_tx
                            .send(SerialRxData::PluginSerialTx {
                                timestamp: Local::now(),
                                plugin_name,
                                content,
                                is_successful: true,
                            })
                            .expect("Cannot send hex string comfirm"),
                        Err(_) => data_tx
                            .send(SerialRxData::PluginSerialTx {
                                timestamp: Local::now(),
                                plugin_name,
                                content,
                                is_successful: false,
                            })
                            .expect("Cannot send hex string fail"),
                    },
                }
            }

            match serial.read(&mut buffer) {
                Ok(_) => {
                    line.push(buffer[0]);
                    if buffer[0] == b'\n' {
                        data_tx
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
                    is_connected.store(false, Ordering::SeqCst);
                    serial = SerialIF::reconnect(
                        port,
                        baudrate,
                        SerialIF::RECONNECT_INTERVAL,
                        is_connected.clone(),
                    )
                    .await;
                }
                Err(_e) => {}
            }

            if now.elapsed().as_millis() > 1_000 {
                now = Instant::now();

                if !line.is_empty() {
                    data_tx
                        .send(SerialRxData::RxData {
                            timestamp: Local::now(),
                            content: line.clone(),
                        })
                        .expect("Cannot forward message read from serial");
                    line.clear();
                }
            }
        }
    }
}
