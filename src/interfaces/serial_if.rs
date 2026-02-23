use crate::{
    error,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
    },
    interfaces::{InterfaceCommand, InterfaceShared},
    plugin::engine::PluginEngineCommand,
    success, warning,
};
use chrono::Local;
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::{
    io::{self, Read, Write},
    ops::{Deref, DerefMut},
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender},
    },
    thread::{sleep, yield_now},
    time::{Duration, Instant},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
type SerialPort = serialport::TTYPort;
#[cfg(target_os = "windows")]
type SerialPort = serialport::COMPort;

pub struct SerialShared {
    pub port: String,
    pub baudrate: u32,
    pub mode: SerialMode,
    pub data_bits: DataBits,
    pub flow_control: FlowControl,
    pub parity: Parity,
    pub stop_bits: StopBits,
}

#[derive(Default)]
pub struct SerialSetup {
    pub port: Option<String>,
    pub baudrate: Option<u32>,
    pub data_bits: Option<DataBits>,
    pub flow_control: Option<FlowControl>,
    pub parity: Option<Parity>,
    pub stop_bits: Option<StopBits>,
}

pub struct SerialConnections {
    logger: Logger,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Producer<Arc<TimedBytes>>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    latency: u64,
}

pub enum SerialCommand {
    Connect,
    Disconnect,
    Exit,
    Setup(SerialSetup),
}

#[derive(Copy, Clone)]
pub enum SerialMode {
    DoNotConnect,
    Reconnecting,
    Connected,
}

pub struct SerialInterface;

impl SerialShared {
    pub fn new(setup: SerialSetup) -> Self {
        Self {
            port: setup.port.clone().unwrap_or("".to_string()),
            baudrate: setup.baudrate.unwrap_or(0),
            data_bits: setup.data_bits.unwrap_or(DataBits::Eight),
            flow_control: setup.flow_control.unwrap_or(FlowControl::None),
            parity: setup.parity.unwrap_or(Parity::None),
            stop_bits: setup.stop_bits.unwrap_or(StopBits::One),
            mode: if !setup.port.unwrap_or("".to_string()).is_empty()
                && setup.baudrate.unwrap_or(0) != 0
            {
                SerialMode::Reconnecting
            } else {
                SerialMode::DoNotConnect
            },
        }
    }
}

impl SerialInterface {
    const NEW_LINE_TIMEOUT_MS: u128 = 1_000;
    const SERIAL_TIMEOUT_MS: u64 = 100;

    fn set_mode(shared: Arc<RwLock<InterfaceShared>>, mode: Option<SerialMode>) {
        let Some(mode) = mode else {
            return;
        };

        let mut sw = shared.write().expect("Cannot get serial lock for write");
        let sw = match sw.deref_mut() {
            InterfaceShared::Serial(sw) => sw,
            _ => unreachable!(),
        };

        sw.mode = mode;
    }

    fn wait(latency: u64) {
        if latency > 0 {
            sleep(Duration::from_micros(latency));
        } else {
            yield_now();
        }
    }

    pub fn task(
        shared: Arc<RwLock<InterfaceShared>>,
        connections: SerialConnections,
        cmd_receiver: Receiver<InterfaceCommand>,
    ) {
        let SerialConnections {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
            latency,
        } = connections;
        let mut line = vec![];
        let mut buffer = [0u8];
        let mut serial = None;
        let mut now = Instant::now();

        'task_loop: loop {
            if let Ok(InterfaceCommand::Serial(cmd)) = cmd_receiver.try_recv() {
                let new_mode = match cmd {
                    SerialCommand::Connect => Self::connect(
                        shared.clone(),
                        &mut serial,
                        &logger,
                        &plugin_engine_cmd_sender,
                    ),
                    SerialCommand::Disconnect => Self::disconnect(
                        shared.clone(),
                        &mut serial,
                        &logger,
                        &plugin_engine_cmd_sender,
                    ),
                    SerialCommand::Exit => break 'task_loop,
                    SerialCommand::Setup(setup) => Self::setup(
                        shared.clone(),
                        setup,
                        &mut serial,
                        &logger,
                        &plugin_engine_cmd_sender,
                    ),
                };
                Self::set_mode(shared.clone(), new_mode);
            }

            {
                let sr = shared.read().expect("Cannot get serial shared for read");
                let sr = match sr.deref() {
                    InterfaceShared::Serial(sr) => sr,
                    _ => unreachable!(),
                };
                let mode = sr.mode;

                match mode {
                    SerialMode::DoNotConnect => {
                        Self::wait(latency);
                        continue 'task_loop;
                    }
                    SerialMode::Reconnecting => {
                        let new_mode = Self::connect(
                            shared.clone(),
                            &mut serial,
                            &logger,
                            &plugin_engine_cmd_sender,
                        );
                        Self::set_mode(shared.clone(), new_mode);
                    }
                    SerialMode::Connected => { /* Do nothing. It's already connected. */ }
                }
            }

            let Some(mut ser) = serial.take() else {
                Self::wait(latency);
                continue 'task_loop;
            };

            if let Ok(data_to_send) = tx.try_recv()
                && ser.write_all(data_to_send.message.as_slice()).is_err()
            {
                error!(logger, "Cannot send: {:?}", data_to_send.message);
            }

            match ser.read(&mut buffer) {
                Ok(_) => {
                    now = Instant::now();
                    line.push(buffer[0]);
                    if buffer[0] == b'\n' {
                        rx.produce(Arc::new(TimedBytes {
                            timestamp: Local::now(),
                            message: std::mem::take(&mut line),
                        }));
                        now = Instant::now();
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
                Err(ref e)
                    if e.kind() == io::ErrorKind::PermissionDenied
                        || e.kind() == io::ErrorKind::BrokenPipe =>
                {
                    let _ = Self::disconnect(
                        shared.clone(),
                        &mut Some(ser),
                        &logger,
                        &plugin_engine_cmd_sender,
                    );
                    Self::set_mode(shared.clone(), Some(SerialMode::Reconnecting));
                    Self::wait(latency);
                    continue 'task_loop;
                }
                Err(_) => {}
            }

            if now.elapsed().as_millis() > Self::NEW_LINE_TIMEOUT_MS {
                now = Instant::now();

                if !line.is_empty() {
                    rx.produce(Arc::new(TimedBytes {
                        timestamp: Local::now(),
                        message: std::mem::take(&mut line),
                    }));
                }
            }

            serial = Some(ser);

            Self::wait(latency);
        }
    }

    fn connect(
        shared: Arc<RwLock<InterfaceShared>>,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let sr = shared
            .read()
            .expect("Cannot get serial share lock for read");
        let sr = match sr.deref() {
            InterfaceShared::Serial(sr) => sr,
            _ => unreachable!(),
        };

        if let SerialMode::Connected = sr.mode {
            return None;
        }

        let connect_res = serialport::new(sr.port.clone(), sr.baudrate)
            .data_bits(sr.data_bits)
            .flow_control(sr.flow_control)
            .parity(sr.parity)
            .stop_bits(sr.stop_bits)
            .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT_MS))
            .open_native();

        match connect_res {
            Ok(ser) => {
                *serial = Some(ser);
                success!(
                    logger,
                    "Connected at \"{}\" with {}bps",
                    sr.port,
                    sr.baudrate
                );
                let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::SerialConnected {
                    port: sr.port.clone(),
                    baudrate: sr.baudrate,
                });
                Some(SerialMode::Connected)
            }
            Err(_) => {
                let _ = serial.take();
                match sr.mode {
                    SerialMode::Reconnecting => None,
                    _ => Some(SerialMode::Reconnecting),
                }
            }
        }
    }

    fn disconnect(
        shared: Arc<RwLock<InterfaceShared>>,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let _ = serial.take();
        let sr = shared.read().expect("Cannot get serial lock for read");
        let sr = match sr.deref() {
            InterfaceShared::Serial(sr) => sr,
            _ => unreachable!(),
        };

        if let SerialMode::Connected = sr.mode {
            warning!(
                logger,
                "Disconnected from \"{}\" with {}bps",
                sr.port,
                sr.baudrate
            );
            let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::SerialDisconnected {
                port: sr.port.clone(),
                baudrate: sr.baudrate,
            });
        }

        match sr.mode {
            SerialMode::DoNotConnect => None,
            _ => Some(SerialMode::DoNotConnect),
        }
    }

    fn setup(
        shared: Arc<RwLock<InterfaceShared>>,
        setup: SerialSetup,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let mut has_changes = false;
        let mut sw = shared
            .write()
            .expect("Cannot get serial shared lock for write");
        let sw_ref = match sw.deref_mut() {
            InterfaceShared::Serial(sw) => sw,
            _ => unreachable!(),
        };

        if let Some(port) = setup.port {
            sw_ref.port = port;
            has_changes = true;
        }

        if let Some(baudrate) = setup.baudrate {
            sw_ref.baudrate = baudrate;
            has_changes = true;
        }

        if let Some(databits) = setup.data_bits {
            sw_ref.data_bits = databits;
            has_changes = true;
        }

        if let Some(flow_control) = setup.flow_control {
            sw_ref.flow_control = flow_control;
            has_changes = true;
        }

        if let Some(parity) = setup.parity {
            sw_ref.parity = parity;
            has_changes = true;
        }

        if let Some(stop_bits) = setup.stop_bits {
            sw_ref.stop_bits = stop_bits;
            has_changes = true;
        }

        let last_mode = sw_ref.mode;
        if has_changes {
            drop(sw);
            let _ = Self::disconnect(shared.clone(), serial, logger, plugin_engine_cmd_sender);

            match last_mode {
                SerialMode::Reconnecting => None,
                _ => Some(SerialMode::Reconnecting),
            }
        } else {
            None
        }
    }
}

impl SerialConnections {
    pub fn new(
        logger: Logger,
        tx: Consumer<Arc<TimedBytes>>,
        rx: Producer<Arc<TimedBytes>>,
        plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
        latency: u64,
    ) -> Self {
        Self {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
            latency,
        }
    }
}
