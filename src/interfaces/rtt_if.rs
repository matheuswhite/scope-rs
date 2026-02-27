use crate::{
    debug, error, info,
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
        } = connections;
        let mut line = vec![];
        let mut buffer = [0u8; 1024];
        let mut session = None;
        let mut rtt = None;
        let mut now = Instant::now();

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
                    ),
                    RttCommand::Disconnect => Self::disconnect(
                        shared.clone(),
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                    ),
                    RttCommand::Setup(setup) => Self::setup(
                        shared.clone(),
                        setup,
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
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
                    _ => unreachable!(),
                };
                let mode = sr_ref.mode;

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
                    _ => unreachable!(),
                };
                sr.channel
            };

            if let Some(output) = rtt_if.down_channel(channel)
                && let Ok(data_to_send) = tx.try_recv()
            {
                let Some(mut core) = session_obj.core(0).ok() else {
                    let _ = Self::disconnect(
                        shared.clone(),
                        &mut Some(session_obj),
                        &mut Some(rtt_if),
                        &logger,
                        &plugin_engine_cmd_sender,
                    );
                    Self::set_mode(shared.clone(), Some(RttMode::Reconnecting));
                    Self::wait(latency);
                    continue 'task_loop;
                };

                if output
                    .write(&mut core, data_to_send.message.as_slice())
                    .is_err()
                {
                    error!(logger, "Cannot send: {:?}", data_to_send.message);
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

            if !received_data {
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
            _ => unreachable!(),
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
                    "Failed to search at first 32KB of RAM: {}, trying to scan entire RAM...", err
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

    fn connect(
        shared: Arc<RwLock<InterfaceShared>>,
        session: &mut Option<Session>,
        rtt: &mut Option<Rtt>,
        logger: &Logger,
        plugin_engine_cmd_sender: &Sender<PluginEngineCommand>,
        last_address: &mut Option<u64>,
    ) -> Option<RttMode> {
        let sr = shared
            .read()
            .expect("Failed to acquire read lock on RTT shared state");
        let sr = match sr.deref() {
            InterfaceShared::Rtt(sr) => sr,
            _ => unreachable!(),
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
                    debug!(logger, "Probe speed: {} kHz", speed);
                    probe.attach(&target, Permissions::default()).ok()
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
    ) -> Option<RttMode> {
        let _ = session.take();
        let _ = rtt.take();
        let sr = shared
            .read()
            .expect("Failed to acquire read lock on RTT shared state");
        let sr = match sr.deref() {
            InterfaceShared::Rtt(sr) => sr,
            _ => unreachable!(),
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
    ) -> Option<RttMode> {
        let mut has_changes = false;
        let mut sw = shared
            .write()
            .expect("Failed to acquire write lock on RTT shared state");
        let sw_ref = match sw.deref_mut() {
            InterfaceShared::Rtt(sw) => sw,
            _ => unreachable!(),
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
        }
    }
}
