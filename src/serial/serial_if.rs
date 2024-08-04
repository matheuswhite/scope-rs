use crate::{
    error,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
        task::Task,
    },
    plugin::engine::PluginEngineCommand,
    success, warning,
};
use chrono::Local;
use serialport::{DataBits, FlowControl, Parity, StopBits, TTYPort};
use std::{
    io::{self, Read, Write},
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

pub type SerialInterface = Task<SerialShared, SerialCommand>;

#[cfg(target_os = "linux")]
type SerialPort = TTYPort;
#[cfg(target_os = "windows")]
type SerialPort = COMPort;

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

impl SerialInterface {
    const NEW_LINE_TIMEOUT_MS: u128 = 1_000;
    const SERIAL_TIMEOUT_MS: u64 = 100;

    pub fn spawn_serial_interface(
        connections: SerialConnections,
        cmd_sender: Sender<SerialCommand>,
        cmd_receiver: Receiver<SerialCommand>,
        setup: SerialSetup,
    ) -> Self {
        Self::new(
            SerialShared {
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
            },
            connections,
            Self::task,
            cmd_sender,
            cmd_receiver,
        )
    }

    fn set_mode(shared: Arc<RwLock<SerialShared>>, mode: Option<SerialMode>) {
        let Some(mode) = mode else {
            return;
        };

        let mut sw = shared.write().expect("Cannot get serial lock for write");
        sw.mode = mode;
    }

    fn task(
        shared: Arc<RwLock<SerialShared>>,
        connections: SerialConnections,
        cmd_receiver: Receiver<SerialCommand>,
    ) {
        let SerialConnections {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
        } = connections;
        let mut line = vec![];
        let mut buffer = [0u8];
        let mut serial = None;
        let mut now = Instant::now();

        'task_loop: loop {
            if let Ok(cmd) = cmd_receiver.try_recv() {
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
                let mode = shared
                    .read()
                    .expect("Cannot get serial shared for read")
                    .mode;

                match mode {
                    SerialMode::DoNotConnect => {
                        std::thread::yield_now();
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
                std::thread::yield_now();
                continue 'task_loop;
            };

            if let Ok(data_to_sent) = tx.try_recv() {
                if ser.write_all(data_to_sent.message.as_slice()).is_err() {
                    error!(logger, "Cannot sent: {:?}", data_to_sent.message);
                }
            }

            match ser.read(&mut buffer) {
                Ok(_) => {
                    now = Instant::now();
                    line.push(buffer[0]);
                    if buffer[0] == b'\n' {
                        rx.produce(Arc::new(TimedBytes {
                            timestamp: Local::now(),
                            message: line.drain(..).collect(),
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
                    std::thread::yield_now();
                    continue 'task_loop;
                }
                Err(_) => {}
            }

            if now.elapsed().as_millis() > Self::NEW_LINE_TIMEOUT_MS {
                now = Instant::now();

                if !line.is_empty() {
                    rx.produce(Arc::new(TimedBytes {
                        timestamp: Local::now(),
                        message: line.drain(..).collect(),
                    }));
                }
            }

            serial = Some(ser);

            std::thread::yield_now();
        }
    }

    fn connect(
        shared: Arc<RwLock<SerialShared>>,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let sw = shared
            .read()
            .expect("Cannot get serial share lock for write");

        if let SerialMode::Connected = sw.mode {
            return None;
        }

        let connect_res = serialport::new(sw.port.clone(), sw.baudrate)
            .data_bits(sw.data_bits)
            .flow_control(sw.flow_control)
            .parity(sw.parity)
            .stop_bits(sw.stop_bits)
            .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT_MS))
            .open_native();

        match connect_res {
            Ok(ser) => {
                *serial = Some(ser);
                success!(
                    logger,
                    "Connected at \"{}\" with {}bps",
                    sw.port,
                    sw.baudrate
                );
                let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::SerialConnected {
                    port: sw.port.clone(),
                    baudrate: sw.baudrate,
                });
                Some(SerialMode::Connected)
            }
            Err(_) => {
                let _ = serial.take();
                match sw.mode {
                    SerialMode::Reconnecting => None,
                    _ => Some(SerialMode::Reconnecting),
                }
            }
        }
    }

    fn disconnect(
        shared: Arc<RwLock<SerialShared>>,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let _ = serial.take();
        let sw = shared.read().expect("Cannot get serial lock for read");
        if let SerialMode::Connected = sw.mode {
            warning!(
                logger,
                "Disconnected from \"{}\" with {}bps",
                sw.port,
                sw.baudrate
            );
            let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::SerialDisconnected {
                port: sw.port.clone(),
                baudrate: sw.baudrate,
            });
        }

        match sw.mode {
            SerialMode::DoNotConnect => None,
            _ => Some(SerialMode::DoNotConnect),
        }
    }

    fn setup(
        shared: Arc<RwLock<SerialShared>>,
        setup: SerialSetup,
        serial: &mut Option<SerialPort>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
    ) -> Option<SerialMode> {
        let mut has_changes = false;
        let mut sw = shared
            .write()
            .expect("Cannot get serial shared lock for write");

        if let Some(port) = setup.port {
            sw.port = port;
            has_changes = true;
        }

        if let Some(baudrate) = setup.baudrate {
            sw.baudrate = baudrate;
            has_changes = true;
        }

        if let Some(databits) = setup.data_bits {
            sw.data_bits = databits;
            has_changes = true;
        }

        if let Some(flow_control) = setup.flow_control {
            sw.flow_control = flow_control;
            has_changes = true;
        }

        if let Some(parity) = setup.parity {
            sw.parity = parity;
            has_changes = true;
        }

        if let Some(stop_bits) = setup.stop_bits {
            sw.stop_bits = stop_bits;
            has_changes = true;
        }

        let last_mode = sw.mode;
        if has_changes {
            drop(sw);
            let _ = Self::disconnect(shared.clone(), serial, &logger, &plugin_engine_cmd_sender);

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
    ) -> Self {
        Self {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
        }
    }
}
