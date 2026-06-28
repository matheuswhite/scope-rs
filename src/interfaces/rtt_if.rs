use crate::{
    debug, error, info,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
    },
    interfaces::{
        InterfaceCommand, InterfaceShared,
        file_transfer::{CHUNK_SIZE, FileTransfer},
    },
    plugin::engine::PluginEngineCommand,
    success, warning,
};
use chrono::Local;
use probe_rs::{
    Core, MemoryInterface, Permissions, Session,
    probe::list::Lister,
    rtt::{Rtt, ScanRegion},
};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender},
    },
    thread::{sleep, yield_now},
    time::{Duration, Instant},
};

pub struct RttShared {
    pub target: String,
    pub mode: RttMode,
    pub channel: usize,
}

pub struct RttConnections {
    logger: Logger,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Producer<Arc<TimedBytes>>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    latency: u64,
    last_address: Option<u64>,
    probe_speed_message: Option<String>,
    fail_to_attach_message: Option<String>,
}

#[derive(Default)]
pub struct RttSetup {
    pub target: Option<String>,
    pub channel: Option<usize>,
}

pub enum RttCommand {
    Connect,
    Disconnect,
    Exit,
    Setup(RttSetup),
    Read {
        address: u64,
        size: usize,
    },
    PluginRead {
        plugin_name: Arc<String>,
        method_id: u64,
        address: u64,
        size: usize,
    },
    SendFile {
        path: String,
    },
}

#[derive(Clone, Copy)]
pub enum RttMode {
    DoNotConnect,
    Reconnecting,
    Connected,
}

pub struct RttInterface;

impl RttShared {
    pub fn new(setup: RttSetup) -> Self {
        let target = setup.target.unwrap_or_default();
        let mode = if !target.is_empty() {
            RttMode::Reconnecting
        } else {
            RttMode::DoNotConnect
        };

        Self {
            target,
            channel: setup.channel.unwrap_or(0),
            mode,
        }
    }
}

impl RttInterface {
    const NEW_LINE_TIMEOUT_MS: u128 = 1_000;

    pub fn task(
        shared: Arc<RwLock<InterfaceShared>>,
        connections: RttConnections,
        cmd_receiver: Receiver<InterfaceCommand>,
    ) {
        let RttConnections {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
            latency,
            mut last_address,
            mut probe_speed_message,
            mut fail_to_attach_message,
        } = connections;
        let mut line = vec![];
        let mut buffer = [0u8; 1024];
        let mut session = None;
        let mut rtt = None;
        let mut now = Instant::now();
        let mut transfer: Option<FileTransfer> = None;

        'task_loop: loop {
            if let Ok(InterfaceCommand::Rtt(cmd)) = cmd_receiver.try_recv() {
                let new_mode = match cmd {
                    RttCommand::Connect => Self::connect(
                        shared.clone(),
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut last_address,
                        &mut probe_speed_message,
                        &mut fail_to_attach_message,
                    ),
                    RttCommand::Disconnect => Self::disconnect(
                        shared.clone(),
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut probe_speed_message,
                        &mut fail_to_attach_message,
                    ),
                    RttCommand::Setup(setup) => Self::setup(
                        shared.clone(),
                        setup,
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut probe_speed_message,
                        &mut fail_to_attach_message,
                    ),
                    RttCommand::Read { address, size } => {
                        match Self::read_memory(session.as_mut(), address, size) {
                            Ok(data) => {
                                info!(
                                    logger,
                                    "Read memory at {:#010X} ({} bytes): {:02X?}",
                                    address,
                                    size,
                                    data
                                );
                            }
                            Err(e) => {
                                error!(logger, "{}", e);
                            }
                        }
                        None
                    }
                    RttCommand::PluginRead {
                        plugin_name,
                        method_id,
                        address,
                        size,
                    } => {
                        let (err, data) = match Self::read_memory(session.as_mut(), address, size) {
                            Ok(data) => ("".to_string(), data),
                            Err(e) => (e, vec![]),
                        };

                        let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::RttReadResult {
                            plugin_name,
                            method_id,
                            err,
                            data,
                        });

                        None
                    }
                    RttCommand::SendFile { path } => {
                        Self::start_file_transfer(&shared, &mut transfer, &path, &logger);
                        None
                    }
                    RttCommand::Exit => break 'task_loop,
                };
                Self::set_mode(shared.clone(), new_mode);
            }

            {
                let sr = shared
                    .read()
                    .expect("Failed to acquire read lock on RTT shared state");
                let sr_ref = match sr.deref() {
                    InterfaceShared::Rtt(sr) => sr,
                    _ => unreachable!(
                        "RttInterface should only be used with Rtt shared. This is a bug. Please, report it."
                    ),
                };
                let mode = sr_ref.mode;

                // A transfer only makes progress while connected; if the link
                // dropped, abort it rather than later resuming mid-stream into a
                // target that may have reset.
                if !matches!(mode, RttMode::Connected)
                    && let Some(t) = transfer.take()
                {
                    warning!(
                        logger,
                        "File transfer of \"{}\" aborted: RTT disconnected",
                        t.name()
                    );
                }

                match mode {
                    RttMode::DoNotConnect => {
                        Self::wait(latency);
                        continue 'task_loop;
                    }
                    RttMode::Reconnecting => {
                        let new_mode = Self::connect(
                            shared.clone(),
                            &mut session,
                            &mut rtt,
                            &logger,
                            &plugin_engine_cmd_sender,
                            &mut last_address,
                            &mut probe_speed_message,
                            &mut fail_to_attach_message,
                        );
                        drop(sr);
                        Self::set_mode(shared.clone(), new_mode);
                    }
                    RttMode::Connected => { /* Do nothing. It's already connected. */ }
                }
            }

            let Some(mut session_obj) = session.take() else {
                Self::wait(latency);
                continue 'task_loop;
            };

            let Some(mut rtt_if) = rtt.take() else {
                Self::wait(latency);
                continue 'task_loop;
            };

            let channel = {
                let sr = shared
                    .read()
                    .expect("Failed to acquire read lock on RTT shared state");
                let sr = match sr.deref() {
                    InterfaceShared::Rtt(sr) => sr,
                    _ => unreachable!(
                        "RttInterface should only be used with Rtt shared. This is a bug. Please, report it."
                    ),
                };
                sr.channel
            };

            let mut sent_file_bytes = false;
            if let Some(output) = rtt_if.down_channel(channel) {
                // Only consume a tx message once we have the down channel, so a
                // not-yet-ready channel doesn't drop queued bytes.
                let tx_msg = tx.try_recv().ok();

                if tx_msg.is_some() || transfer.is_some() {
                    let Some(mut core) = session_obj.core(0).ok() else {
                        let _ = Self::disconnect(
                            shared.clone(),
                            &mut Some(session_obj),
                            &mut Some(rtt_if),
                            &logger,
                            &plugin_engine_cmd_sender,
                            &mut probe_speed_message,
                            &mut fail_to_attach_message,
                        );
                        Self::set_mode(shared.clone(), Some(RttMode::Reconnecting));
                        Self::wait(latency);
                        continue 'task_loop;
                    };

                    if let Some(data_to_send) = tx_msg
                        && output
                            .write(&mut core, data_to_send.message.as_slice())
                            .is_err()
                    {
                        error!(logger, "Cannot send: {:?}", data_to_send.message);
                    }

                    // Stream the next chunk of an in-progress file transfer
                    // straight to the down channel (never through `tx`). The RTT
                    // buffer may accept fewer bytes than offered, so advance by
                    // the count actually written.
                    if transfer.is_some() {
                        let (done, written) = {
                            let t = transfer.as_mut().unwrap();
                            let chunk = t.next_chunk(CHUNK_SIZE);
                            match output.write(&mut core, chunk) {
                                Ok(written) => (t.advance(written, &logger), written),
                                Err(err) => {
                                    error!(logger, "Failed to send \"{}\": {}", t.name(), err);
                                    (true, 0)
                                }
                            }
                        };
                        sent_file_bytes = written > 0;
                        if done {
                            transfer = None;
                        }
                    }
                }
            }

            let mut received_data = false;
            if let Some(input) = rtt_if.up_channel(channel) {
                let Some(mut core) = session_obj.core(0).ok() else {
                    let _ = Self::disconnect(
                        shared.clone(),
                        &mut Some(session_obj),
                        &mut Some(rtt_if),
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut probe_speed_message,
                        &mut fail_to_attach_message,
                    );
                    Self::set_mode(shared.clone(), Some(RttMode::Reconnecting));
                    Self::wait(latency);
                    continue 'task_loop;
                };

                match input.read(&mut core, &mut buffer) {
                    Ok(size) => {
                        if size > 0 {
                            received_data = true;
                            let mut parts = buffer[..size].split(|byte| *byte == b'\n').rev();
                            let last = parts.next().unwrap_or(&[]);
                            let parts = parts.rev();

                            for part in parts {
                                let mut part = part.to_vec();
                                part.push(b'\n');

                                rx.produce(Arc::new(TimedBytes {
                                    timestamp: Local::now(),
                                    message: part,
                                }));

                                now = Instant::now();
                            }

                            if last.len() > 0 {
                                line.extend_from_slice(last);
                                now = Instant::now();
                            }
                        }
                    }
                    Err(e) => {
                        warning!(logger, "Fail to read: {}", e);
                    }
                }
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

            rtt = Some(rtt_if);
            session = Some(session_obj);

            // Throttle only when we made no progress this iteration. An active
            // transfer streaming bytes runs without the per-iteration wait (so
            // it isn't capped at one chunk per `latency`), but a full down
            // channel that accepted nothing still backs off — no busy-spin.
            if !received_data && !sent_file_bytes {
                Self::wait(latency);
            }
        }
    }

    fn set_mode(shared: Arc<RwLock<InterfaceShared>>, mode: Option<RttMode>) {
        let Some(mode) = mode else {
            return;
        };

        let mut sw = shared
            .write()
            .expect("Failed to acquire write lock on RTT shared state");
        let sw = match sw.deref_mut() {
            InterfaceShared::Rtt(sw) => sw,
            _ => unreachable!(
                "RttInterface should only be used with Rtt shared. This is a bug. Please, report it."
            ),
        };
        sw.mode = mode;
    }

    fn wait(latency: u64) {
        if latency > 0 {
            sleep(Duration::from_millis(latency));
        } else {
            yield_now();
        }
    }

    /// Arm a file transfer. Requires an active connection (so the user gets
    /// immediate feedback instead of a transfer that silently waits) and refuses
    /// to overlap with one already running; the file read and kickoff log are
    /// handled by [`FileTransfer::load`].
    fn start_file_transfer(
        shared: &Arc<RwLock<InterfaceShared>>,
        transfer: &mut Option<FileTransfer>,
        path: &str,
        logger: &Logger,
    ) {
        let connected = {
            let sr = shared
                .read()
                .expect("Failed to acquire read lock on RTT shared state");
            matches!(sr.deref(), InterfaceShared::Rtt(sr) if matches!(sr.mode, RttMode::Connected))
        };
        if !connected {
            error!(logger, "Cannot send \"{}\": RTT is not connected", path);
            return;
        }

        if let Some(t) = transfer {
            error!(
                logger,
                "Cannot send \"{}\": already sending \"{}\"",
                path,
                t.name()
            );
            return;
        }

        *transfer = FileTransfer::load(path, logger);
    }

    fn rtt_attach(core: &mut Core, last_address: &mut Option<u64>, logger: &Logger) -> Option<Rtt> {
        let rtt = if let Some(addr) = last_address {
            Rtt::attach_at(core, *addr)
        } else {
            let first_32kb_in_ram = ScanRegion::range(0x2000_0000..0x2000_8000);
            let res = Rtt::attach_region(core, &first_32kb_in_ram);
            if let Err(err) = &res
                && !matches!(err, probe_rs::rtt::Error::MultipleControlBlocksFound(_))
            {
                debug!(
                    logger,
                    "Failed to search at first 32KB of RAM, trying to scan entire RAM..."
                );
                Rtt::attach(core)
            } else {
                res
            }
        };

        match rtt {
            Ok(rtt) => {
                *last_address = Some(rtt.ptr());
                Some(rtt)
            }
            Err(probe_rs::rtt::Error::MultipleControlBlocksFound(instances)) => {
                warning!(
                    logger,
                    "Multiple RTT control blocks found ({}); selecting first at address {:#010X}",
                    instances.len(),
                    instances[0]
                );
                let res = Rtt::attach_at(core, instances[0]).ok();
                *last_address = Some(instances[0]);
                res
            }
            Err(_) => None,
        }
    }

    fn log_probe_speed(logger: &Logger, probe_speed_message: &mut Option<String>, speed: u32) {
        let message = format!("Probe speed: {} kHz", speed);
        if probe_speed_message.as_ref() != Some(&message) {
            debug!(logger, "{}", message);
            *probe_speed_message = Some(message);
        }
    }

    fn log_fail_to_attach(
        logger: &Logger,
        fail_to_attach_message: &mut Option<String>,
        res: &Result<Session, probe_rs::Error>,
    ) {
        if let Err(err) = res {
            let message = format!("Failed to attach to target: {}", err);
            if fail_to_attach_message.as_ref() != Some(&message) {
                error!(logger, "{}", message);
                *fail_to_attach_message = Some(message);
            }
        }
    }

    fn connect(
        shared: Arc<RwLock<InterfaceShared>>,
        session: &mut Option<Session>,
        rtt: &mut Option<Rtt>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
        last_address: &mut Option<u64>,
        probe_speed_message: &mut Option<String>,
        fail_to_attach_message: &mut Option<String>,
    ) -> Option<RttMode> {
        let sr = shared
            .read()
            .expect("Failed to acquire read lock on RTT shared state");
        let sr = match sr.deref() {
            InterfaceShared::Rtt(sr) => sr,
            _ => unreachable!(
                "RttInterface::connect should only be called with Rtt shared. This is a bug. Please, report it."
            ),
        };

        if let RttMode::Connected = sr.mode {
            return None;
        }

        let target = sr.target.clone();

        let lister = Lister::new();
        let probes = lister.list_all();
        let Some(new_session) =
            probes
                .get(0)
                .and_then(|probe| probe.open().ok())
                .and_then(|mut probe| {
                    let Ok(speed) = probe.set_speed(4_000) else {
                        error!(logger, "Failed to set probe speed");
                        return None;
                    };
                    Self::log_probe_speed(logger, probe_speed_message, speed);
                    let res = probe.attach(&target, Permissions::default());
                    Self::log_fail_to_attach(logger, fail_to_attach_message, &res);
                    res.ok()
                })
        else {
            let _ = rtt.take();
            let _ = session.take();
            return match sr.mode {
                RttMode::Reconnecting => None,
                _ => Some(RttMode::Reconnecting),
            };
        };
        *session = Some(new_session);

        let Some(new_rtt) = session
            .as_mut()
            .and_then(|s| s.core(0).ok())
            .and_then(|mut core| {
                debug!(logger, "Attaching to RTT...");
                let res = Self::rtt_attach(&mut core, last_address, logger);
                res
            })
        else {
            let _ = rtt.take();
            let _ = session.take();
            return match sr.mode {
                RttMode::Reconnecting => None,
                _ => Some(RttMode::Reconnecting),
            };
        };
        *rtt = Some(new_rtt);

        success!(
            logger,
            "Connected at \"{}\" on channel {}",
            sr.target,
            sr.channel
        );
        let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::RttConnected {
            target: sr.target.clone(),
            channel: sr.channel,
        });
        Some(RttMode::Connected)
    }

    fn disconnect(
        shared: Arc<RwLock<InterfaceShared>>,
        session: &mut Option<Session>,
        rtt: &mut Option<Rtt>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
        probe_speed_message: &mut Option<String>,
        fail_to_attach_message: &mut Option<String>,
    ) -> Option<RttMode> {
        let _ = fail_to_attach_message.take();
        let _ = probe_speed_message.take();
        let _ = session.take();
        let _ = rtt.take();
        let sr = shared
            .read()
            .expect("Failed to acquire read lock on RTT shared state");
        let sr = match sr.deref() {
            InterfaceShared::Rtt(sr) => sr,
            _ => unreachable!(
                "RttInterface::disconnect should only be called with Rtt shared. This is a bug. Please, report it."
            ),
        };

        if let RttMode::Connected = sr.mode {
            warning!(
                logger,
                "Disconnected from \"{}\" on channel {}",
                sr.target,
                sr.channel
            );
            let _ = plugin_engine_cmd_sender.send(PluginEngineCommand::RttDisconnected {
                target: sr.target.clone(),
                channel: sr.channel,
            });
        }

        match sr.mode {
            RttMode::DoNotConnect => None,
            _ => Some(RttMode::DoNotConnect),
        }
    }

    fn setup(
        shared: Arc<RwLock<InterfaceShared>>,
        setup: RttSetup,
        session: &mut Option<Session>,
        rtt: &mut Option<Rtt>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
        probe_speed_message: &mut Option<String>,
        fail_to_attach_message: &mut Option<String>,
    ) -> Option<RttMode> {
        let mut has_changes = false;
        let mut sw = shared
            .write()
            .expect("Failed to acquire write lock on RTT shared state");
        let sw_ref = match sw.deref_mut() {
            InterfaceShared::Rtt(sw) => sw,
            _ => unreachable!(
                "RttInterface::setup should only be called with Rtt shared. This is a bug. Please, report it."
            ),
        };

        if let Some(target) = setup.target {
            sw_ref.target = target;
            has_changes = true;
        }

        if let Some(channel) = setup.channel {
            sw_ref.channel = channel;
            has_changes = true;
        }

        let last_mode = sw_ref.mode;
        if has_changes {
            drop(sw);
            let _ = Self::disconnect(
                shared.clone(),
                session,
                rtt,
                logger,
                plugin_engine_cmd_sender,
                probe_speed_message,
                fail_to_attach_message,
            );

            match last_mode {
                RttMode::Reconnecting => None,
                _ => Some(RttMode::Reconnecting),
            }
        } else {
            None
        }
    }

    fn read_memory(
        session: Option<&mut Session>,
        address: u64,
        size: usize,
    ) -> Result<Vec<u8>, String> {
        let Some(session) = session else {
            return Err("Cannot read memory: not connected".to_string());
        };

        let mut core = match session.core(0) {
            Ok(core) => core,
            Err(e) => {
                return Err(format!("Failed to get core: {}", e));
            }
        };

        if size > 1024 {
            return Err(format!(
                "Requested read size {} exceeds maximum of 1024",
                size
            ));
        }

        let mut buffer = vec![0u8; size];
        if let Err(e) = core.read(address, &mut buffer) {
            return Err(format!("Failed to read memory at {:#010X}: {}", address, e));
        }

        Ok(buffer)
    }
}

impl RttConnections {
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
            last_address: None,
            probe_speed_message: None,
            fail_to_attach_message: None,
        }
    }
}
