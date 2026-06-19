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
use serialport::{
    DataBits, FlowControl, Parity, SerialPortInfo, SerialPortType, StopBits, UsbPortInfo,
};
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
    /// USB identity of the connected device, learned on the first successful
    /// connection. Used to track the device across kernel renames on re-plug
    /// (e.g. /dev/ttyUSB0 -> /dev/ttyUSB1 on Linux). `None` for non-USB ports.
    pub usb_id: Option<UsbPortInfo>,
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
            usb_id: None,
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
    /// Minimum gap between reconnect attempts. Each attempt may enumerate every
    /// serial port (to follow a renamed device), so retrying in a tight loop
    /// while a device is unplugged would waste CPU and churn sysfs on Linux.
    const RECONNECT_INTERVAL_MS: u64 = 500;

    fn set_mode(shared: Arc<RwLock<InterfaceShared>>, mode: Option<SerialMode>) {
        let Some(mode) = mode else {
            return;
        };

        let mut sw = shared.write().expect("Cannot get serial lock for write");
        let sw = match sw.deref_mut() {
            InterfaceShared::Serial(sw) => sw,
            _ => unreachable!(
                "SerialInterface should only be used with Serial shared. This is a bug. Please, report it."
            ),
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
        let mut last_reconnect: Option<Instant> = None;

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

            let mode = {
                let sr = shared.read().expect("Cannot get serial shared for read");
                match sr.deref() {
                    InterfaceShared::Serial(sr_ref) => sr_ref.mode,
                    _ => unreachable!(
                        "SerialInterface should only be used with Serial shared. This is a bug. Please, report it."
                    ),
                }
            };

            match mode {
                SerialMode::DoNotConnect => {
                    Self::wait(latency);
                    continue 'task_loop;
                }
                SerialMode::Reconnecting => {
                    // Throttle attempts (see RECONNECT_INTERVAL_MS); the first
                    // one after a disconnect still fires immediately for quick
                    // recovery, since the previous attempt is far in the past.
                    let due = last_reconnect.is_none_or(|t| {
                        t.elapsed() >= Duration::from_millis(Self::RECONNECT_INTERVAL_MS)
                    });
                    if due {
                        last_reconnect = Some(Instant::now());
                        // The read lock is released above before connecting:
                        // `connect` may take a write lock to record a renamed path.
                        let new_mode = Self::connect(
                            shared.clone(),
                            &mut serial,
                            &logger,
                            &plugin_engine_cmd_sender,
                        );
                        Self::set_mode(shared.clone(), new_mode);
                    } else {
                        Self::wait(latency);
                        continue 'task_loop;
                    }
                }
                SerialMode::Connected => { /* Do nothing. It's already connected. */ }
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
        // Snapshot the connection settings and release the lock: opening a port
        // and enumerating devices can block, and on success `connect` takes a
        // write lock to persist a learned USB identity or a renamed path.
        let (mode, port, baudrate, data_bits, flow_control, parity, stop_bits, usb_id) = {
            let sr = shared
                .read()
                .expect("Cannot get serial share lock for read");
            let sr = match sr.deref() {
                InterfaceShared::Serial(sr) => sr,
                _ => unreachable!(
                    "SerialInterface::connect should only be called with Serial shared. This is a bug. Please, report it."
                ),
            };
            (
                sr.mode,
                sr.port.clone(),
                sr.baudrate,
                sr.data_bits,
                sr.flow_control,
                sr.parity,
                sr.stop_bits,
                sr.usb_id.clone(),
            )
        };

        if let SerialMode::Connected = mode {
            return None;
        }

        let open = |port: &str| {
            serialport::new(port, baudrate)
                .data_bits(data_bits)
                .flow_control(flow_control)
                .parity(parity)
                .stop_bits(stop_bits)
                .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT_MS))
                .open_native()
        };

        // Try the known path first. If it's gone but we know the device's USB
        // identity, look for the same device under a new name — the Linux
        // re-plug rename from issue #53 (e.g. ttyUSB0 -> ttyUSB1).
        let mut connected_port = port.clone();
        let mut connect_res = open(&connected_port);
        if connect_res.is_err()
            && let Some(usb_id) = &usb_id
            && let Some(new_port) = Self::find_renamed_port(&connected_port, usb_id)
            && new_port != connected_port
        {
            if let Ok(ser) = open(&new_port) {
                connected_port = new_port;
                connect_res = Ok(ser);
            }
        }

        match connect_res {
            Ok(ser) => {
                *serial = Some(ser);

                // Reconcile the stored USB identity with the device we actually
                // opened: learn it on the first connection, refresh it if a
                // different device now sits on this path, and clear it for a
                // non-USB port. This keeps later rename scans pointed at the
                // current device instead of a previously connected one.
                let current_usb_id = Self::usb_info_for(&connected_port);
                let usb_id_changed = current_usb_id != usb_id;
                // Record any path change too, so the status bar reflects where
                // we actually reconnected.
                let renamed = connected_port != port;
                if renamed || usb_id_changed {
                    let mut sw = shared.write().expect("Cannot get serial lock for write");
                    if let InterfaceShared::Serial(sw) = sw.deref_mut() {
                        if renamed {
                            sw.port = connected_port.clone();
                        }
                        if usb_id_changed {
                            sw.usb_id = current_usb_id;
                        }
                    }
                }

                if renamed {
                    warning!(
                        logger,
                        "Serial device moved from \"{}\" to \"{}\"",
                        port,
                        connected_port
                    );
                }
                success!(
                    logger,
                    "Connected at \"{}\" with {}bps",
                    connected_port,
                    baudrate
                );
                let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::SerialConnected {
                    port: connected_port,
                    baudrate,
                });
                Some(SerialMode::Connected)
            }
            Err(_) => {
                let _ = serial.take();
                match mode {
                    SerialMode::Reconnecting => None,
                    _ => Some(SerialMode::Reconnecting),
                }
            }
        }
    }

    /// Look up the USB identity of the currently-available port named `port`.
    /// Returns `None` for non-USB ports (or if enumeration fails), in which case
    /// the device can't be tracked across renames and we keep using the path.
    fn usb_info_for(port: &str) -> Option<UsbPortInfo> {
        serialport::available_ports().ok()?.into_iter().find_map(
            |SerialPortInfo {
                 port_name,
                 port_type,
             }| match port_type {
                SerialPortType::UsbPort(info) if port_name == port => Some(info),
                _ => None,
            },
        )
    }

    /// Find where the device identified by `usb_id` moved to after its original
    /// path `current_port` disappeared. Returns `None` (so we keep retrying the
    /// same path) when `current_port` is still present — then the open failure
    /// was transient (busy/permission), not a rename, and scanning could pick a
    /// different identical adapter. Also `None` when no matching device is found.
    fn find_renamed_port(current_port: &str, usb_id: &UsbPortInfo) -> Option<String> {
        let ports = serialport::available_ports().ok()?;

        // The original path still exists: not a rename, don't go looking.
        if ports.iter().any(|p| p.port_name == current_port) {
            return None;
        }

        ports.into_iter().find_map(
            |SerialPortInfo {
                 port_name,
                 port_type,
             }| match port_type {
                SerialPortType::UsbPort(info) if Self::usb_matches(usb_id, &info) => {
                    Some(port_name)
                }
                _ => None,
            },
        )
    }

    /// Whether `candidate` is the same physical device as the `stored` identity.
    /// Vendor and product ids must always match. If we learned a serial number
    /// for the device, the candidate must report the same one — a serial is
    /// stable across re-plugs, so a candidate lacking it (or reporting another)
    /// is a different unit. Only when the device reports no serial at all (many
    /// cheap adapters don't) do we fall back to vid+pid alone.
    fn usb_matches(stored: &UsbPortInfo, candidate: &UsbPortInfo) -> bool {
        if stored.vid != candidate.vid || stored.pid != candidate.pid {
            return false;
        }

        match &stored.serial_number {
            Some(stored_serial) => candidate.serial_number.as_deref() == Some(stored_serial),
            None => true,
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
            _ => unreachable!(
                "SerialInterface::disconnect should only be called with Serial shared. This is a bug. Please, report it."
            ),
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
            _ => unreachable!(
                "SerialInterface::setup should only be called with Serial shared. This is a bug. Please, report it."
            ),
        };

        if let Some(port) = setup.port {
            // A new port may be a different physical device, so drop the learned
            // USB identity: otherwise a reconnect from the new path could scan
            // and re-attach to the *previous* device. It's re-learned on the
            // next successful connection.
            if sw_ref.port != port {
                sw_ref.usb_id = None;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usb(vid: u16, pid: u16, serial: Option<&str>) -> UsbPortInfo {
        UsbPortInfo {
            vid,
            pid,
            serial_number: serial.map(str::to_string),
            manufacturer: None,
            product: None,
            interface: None,
        }
    }

    #[test]
    fn matches_same_device_by_serial_number() {
        let a = usb(0x10c4, 0xea60, Some("ABC123"));
        let b = usb(0x10c4, 0xea60, Some("ABC123"));
        assert!(SerialInterface::usb_matches(&a, &b));
    }

    #[test]
    fn rejects_different_serial_number() {
        let a = usb(0x10c4, 0xea60, Some("ABC123"));
        let b = usb(0x10c4, 0xea60, Some("XYZ789"));
        assert!(!SerialInterface::usb_matches(&a, &b));
    }

    #[test]
    fn rejects_candidate_without_serial_when_stored_has_one() {
        // A serial number is stable across re-plugs, so a same-vid/pid device
        // that reports none is a different unit, not the one we learned.
        let stored = usb(0x10c4, 0xea60, Some("ABC123"));
        assert!(!SerialInterface::usb_matches(
            &stored,
            &usb(0x10c4, 0xea60, None)
        ));
    }

    #[test]
    fn rejects_different_vid_or_pid() {
        let a = usb(0x10c4, 0xea60, Some("ABC123"));
        assert!(!SerialInterface::usb_matches(
            &a,
            &usb(0x0403, 0xea60, Some("ABC123"))
        ));
        assert!(!SerialInterface::usb_matches(
            &a,
            &usb(0x10c4, 0x6001, Some("ABC123"))
        ));
    }

    #[test]
    fn falls_back_to_vid_pid_when_serial_number_absent() {
        // Cheap adapters (e.g. CH340) report no serial number: vid+pid is then
        // the best signal we have, so a match on those alone is accepted.
        let stored = usb(0x1a86, 0x7523, None);
        assert!(SerialInterface::usb_matches(
            &stored,
            &usb(0x1a86, 0x7523, None)
        ));
        assert!(SerialInterface::usb_matches(
            &stored,
            &usb(0x1a86, 0x7523, Some("whatever"))
        ));
    }
}
