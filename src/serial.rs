use crate::interface::{DataIn, DataOut, Interface};
use chrono::Local;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::time::Duration;
use std::{io, thread};
use tui::style::Color;

pub struct SerialIF {
    serial_tx: Sender<DataIn>,
    data_rx: Receiver<DataOut>,
    port: String,
    baudrate: u32,
    is_connected: Arc<AtomicBool>,
}

impl Drop for SerialIF {
    fn drop(&mut self) {
        self.serial_tx.send(DataIn::Exit).unwrap()
    }
}

impl Interface for SerialIF {
    fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    fn send(&self, data: DataIn) {
        self.serial_tx.send(data).unwrap();
    }

    fn try_recv(&self) -> Result<DataOut, TryRecvError> {
        self.data_rx.try_recv()
    }

    fn description(&self) -> String {
        format!("Serial {}:{}bps", self.port, self.baudrate)
    }

    fn color(&self) -> Color {
        Color::Yellow
    }
}

#[allow(unused)]
impl SerialIF {
    const SERIAL_TIMEOUT: Duration = Duration::from_millis(10);
    const RECONNECT_INTERVAL: Duration = Duration::from_millis(200);

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
            if let Ok(mut serial) = serialport::new(port, baudrate).open() {
                serial
                    .set_timeout(SerialIF::SERIAL_TIMEOUT)
                    .expect("Cannot set serialport timeout");
                is_connected.store(true, Ordering::SeqCst);
                break 'reconnect serial;
            }
            thread::sleep(interval);
        }
    }

    fn send_task(
        port: &str,
        baudrate: u32,
        serial_rx: Receiver<DataIn>,
        data_tx: Sender<DataOut>,
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

        'task: loop {
            if let Ok(data_to_send) = serial_rx.try_recv() {
                match data_to_send {
                    DataIn::Exit => break 'task,
                    DataIn::Data(data_to_send) => {
                        match serial.write(format!("{data_to_send}\r\n").as_bytes()) {
                            Ok(_) => {
                                data_tx
                                    .send(DataOut::ConfirmData(Local::now(), data_to_send))
                                    .expect("Cannot send data confirm");
                                eprint!("Data tx sent\r");
                            }
                            Err(_) => {
                                data_tx
                                    .send(DataOut::FailData(Local::now(), data_to_send))
                                    .expect("Canot send data fail");
                            }
                        }
                    }
                    DataIn::Command(command_name, data_to_send) => {
                        match serial.write(format!("{data_to_send}\r\n").as_bytes()) {
                            Ok(_) => {
                                data_tx
                                    .send(DataOut::ConfirmCommand(
                                        Local::now(),
                                        command_name,
                                        data_to_send,
                                    ))
                                    .expect("Cannot send command confirm");
                            }
                            Err(_) => {
                                data_tx
                                    .send(DataOut::FailCommand(
                                        Local::now(),
                                        command_name,
                                        data_to_send,
                                    ))
                                    .expect("Cannot send command fail");
                            }
                        }
                    }
                    DataIn::HexString(bytes) => {
                        let content = [bytes.clone(), b"\r\n".to_vec()].concat();
                        match serial.write(&content) {
                            Ok(_) => data_tx
                                .send(DataOut::ConfirmHexString(Local::now(), bytes))
                                .expect("Cannot send hex string comfirm"),
                            Err(_) => data_tx
                                .send(DataOut::FailHexString(Local::now(), bytes))
                                .expect("Cannot send hex string fail"),
                        }
                    }
                }
            }

            match serial.read(&mut buffer) {
                Ok(_) => {
                    if buffer[0] == b'\n' {
                        data_tx
                            .send(DataOut::Data(Local::now(), line.clone()))
                            .expect("Cannot forward message read from serial");
                        line.clear();
                    } else {
                        line.push(buffer[0] as char);
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
                Err(e) => eprint!("{:?}", e.kind()),
            }
        }
    }
}
