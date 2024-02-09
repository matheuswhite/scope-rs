use crate::messages::{SerialRxData, UserTxData};
use chrono::Local;
use serialport::{self, SerialPort};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{io, thread};

pub struct SerialIF {
    serial_tx: Sender<UserTxData>,
    data_rx: Receiver<SerialRxData>,
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

    pub fn try_recv(&self) -> Result<SerialRxData, TryRecvError> {
        self.data_rx.try_recv()
    }

    pub fn recv(&self) -> Result<SerialRxData, RecvError> {
        self.data_rx.recv()
    }

    pub fn description(&self) -> String {
        format!("Serial {}:{}bps", self.port, self.baudrate)
    }

    pub fn new(port: &str, baudrate: u32) -> Self {
        let (serial_tx, serial_rx) = channel();
        let (data_tx, data_rx) = channel();

        let is_connected = Arc::new(AtomicBool::new(false));

        let port_clone = port.to_string();
        let is_connected_clone = is_connected.clone();
        thread::spawn(move || {
            SerialIF::send_task(
                &port_clone,
                baudrate,
                serial_rx,
                data_tx,
                is_connected_clone,
            );
        });

        Self {
            serial_tx,
            data_rx,
            port: port.to_string(),
            baudrate,
            is_connected,
        }
    }

    fn reconnect(
        port: &str,
        baudrate: u32,
        interval: Duration,
        is_connected: Arc<AtomicBool>,
    ) -> Box<dyn SerialPort> {
        'reconnect: loop {
            if let Ok(serial) = serialport::new(port, baudrate)
                .data_bits(serialport::DataBits::Eight)
                .flow_control(serialport::FlowControl::Hardware)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .timeout(SerialIF::SERIAL_TIMEOUT)
                .open()
            {
                is_connected.store(true, Ordering::SeqCst);
                break 'reconnect serial;
            }
            thread::sleep(interval);
        }
    }

    fn send_task(
        port: &str,
        baudrate: u32,
        serial_rx: Receiver<UserTxData>,
        data_tx: Sender<SerialRxData>,
        is_connected: Arc<AtomicBool>,
    ) {
        let mut serial = SerialIF::reconnect(
            port,
            baudrate,
            SerialIF::RECONNECT_INTERVAL,
            is_connected.clone(),
        );

        let mut line = String::new();
        let mut buffer = [0u8];
        let mut now = Instant::now();

        'task: loop {
            if let Ok(data_to_send) = serial_rx.try_recv() {
                match data_to_send {
                    UserTxData::Exit => break 'task,
                    UserTxData::Data { content } => {
                        match serial.write(format!("{content}\r\n").as_bytes()) {
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
                    } => match serial.write(format!("{content}\r\n").as_bytes()) {
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
                    },
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
                    line.push(buffer[0] as char);
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
                Err(ref e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    is_connected.store(false, Ordering::SeqCst);
                    serial = SerialIF::reconnect(
                        port,
                        baudrate,
                        SerialIF::RECONNECT_INTERVAL,
                        is_connected.clone(),
                    );
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
