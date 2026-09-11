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
        rtt_elf,
    },
    plugin::engine::PluginEngineCommand,
    success, warning,
};
use chrono::Local;
use probe_rs::{
    Core, MemoryInterface, Permissions, Session,
    config::MemoryRegion,
    probe::list::Lister,
    rtt::{Rtt, ScanRegion},
};
use std::{
    ops::{Deref, DerefMut, Range},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender},
    },
    thread::{sleep, yield_now},
    time::{Duration, Instant},
};

/// Where to look for the RTT control block, as chosen by the user
/// (issue #248).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ControlBlock {
    /// Derive the scan windows from the target itself. The default, and the
    /// only option that needs nothing from the user.
    #[default]
    Scan,
    /// Read the `_SEGGER_RTT` symbol from this ELF and attach at that address
    /// (`--elf`). Re-read on every attach, so re-flashing a build that moved
    /// the block does not need a restart.
    Elf(PathBuf),
    /// Attach at exactly this address (`--addr`).
    Exact(u64),
}

pub struct RttShared {
    pub target: String,
    pub mode: RttMode,
    pub channel: usize,
    /// See [`ControlBlock`].
    pub control_block: ControlBlock,
    /// Probe clock in kHz, as passed to `set_speed` on every attach.
    pub probe_speed: u32,
}

pub struct RttConnections {
    logger: Logger,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Producer<Arc<TimedBytes>>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    latency: u64,
    headless: bool,
    last_address: Option<u64>,
    last_logged: LastLogged,
}

/// The last message logged for each failure the reconnect loop can hit, so a
/// problem is reported once instead of on every pass through the loop.
#[derive(Default)]
struct LastLogged {
    probe_speed: Option<String>,
    session_attach: Option<String>,
    rtt_attach: Option<String>,
}

#[derive(Default)]
pub struct RttSetup {
    pub target: Option<String>,
    pub channel: Option<usize>,
    pub control_block: Option<ControlBlock>,
    pub probe_speed: Option<u32>,
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
            control_block: setup.control_block.unwrap_or_default(),
            probe_speed: setup
                .probe_speed
                .unwrap_or(RttInterface::DEFAULT_PROBE_SPEED_KHZ),
        }
    }
}

impl RttInterface {
    const NEW_LINE_TIMEOUT_MS: u128 = 1_000;
    /// Probe clock used when `--speed` is not given — what the code hard-coded
    /// before it was configurable (issue #248).
    pub const DEFAULT_PROBE_SPEED_KHZ: u32 = 4_000;
    /// How much of a RAM region is probed when looking for the control block.
    /// The block is conventionally at the very start of a region, so a small
    /// leading window finds it while costing almost nothing to read.
    const SCAN_WINDOW: u64 = 32 * 1024;

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
            headless,
            mut last_address,
            mut last_logged,
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
                        &mut last_logged,
                    ),
                    RttCommand::Disconnect => Self::disconnect(
                        shared.clone(),
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut last_logged,
                    ),
                    RttCommand::Setup(setup) => Self::setup(
                        shared.clone(),
                        setup,
                        &mut session,
                        &mut rtt,
                        &logger,
                        &plugin_engine_cmd_sender,
                        &mut last_logged,
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
                            &mut last_logged,
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
                            &mut last_logged,
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
                        &mut last_logged,
                    );
                    Self::set_mode(shared.clone(), Some(RttMode::Reconnecting));
                    Self::wait(latency);
                    continue 'task_loop;
                };

                match input.read(&mut core, &mut buffer) {
                    Ok(size) => {
                        if size > 0 {
                            received_data = true;

                            if headless {
                                // Forward the whole read chunk immediately, no
                                // newline framing, so prompts and ANSI appear
                                // live (see the serial path for the rationale).
                                rx.produce(Arc::new(TimedBytes {
                                    timestamp: Local::now(),
                                    message: buffer[..size].to_vec(),
                                }));
                                now = Instant::now();
                            } else {
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

    /// The leading window of each of `ranges`, clamped so a region smaller than
    /// [`Self::SCAN_WINDOW`] is never read past its end.
    fn scan_windows(ranges: &[Range<u64>]) -> Vec<Range<u64>> {
        ranges
            .iter()
            .filter(|range| range.start < range.end)
            .map(|range| range.start..range.end.min(range.start + Self::SCAN_WINDOW))
            .collect()
    }

    /// Where to look for the control block on a fresh attach.
    ///
    /// This used to be a hard-coded `0x20000000..0x20008000` — the conventional
    /// Cortex-M SRAM base, which is simply not where every part keeps its RAM.
    /// On a target whose RAM sits elsewhere the cheap probe could never hit, so
    /// *every* attach fell through to sweeping the whole RAM: issue #248
    /// reported 3.2s per attach on an i.MX RT1021, whose application RAM is
    /// OCRAM at `0x20200000`.
    ///
    /// The windows now come from the target, in this order:
    ///
    /// 1. `--addr`, or `_SEGGER_RTT` read from `--elf`: exact, no scan at all.
    /// 2. The `rtt_scan_regions` of the probe-rs target description, when it
    ///    names them — the chip's own definition beats any convention.
    /// 3. The first [`Self::SCAN_WINDOW`] bytes of every RAM region in the
    ///    target's memory map. "The control block lives at the start of a RAM
    ///    region" is near-universal — Zephyr forces it with linker sort key
    ///    `aaa`, and SEGGER's own examples do the same — so a handful of small
    ///    reads hit on essentially any target, and they stay cheap because the
    ///    count of RAM regions is what grows, not the bytes per region.
    fn scan_region(
        core: &mut Core,
        control_block: &ControlBlock,
        target_regions: &ScanRegion,
        logger: &Logger,
    ) -> ScanRegion {
        let ram = core
            .memory_regions()
            .filter_map(MemoryRegion::as_ram_region)
            .map(|region| region.range.clone())
            .collect::<Vec<_>>();

        Self::scan_region_in(control_block, target_regions, &ram, logger)
    }

    /// The half of [`Self::scan_region`] that needs no live core: `ram` is the
    /// target's RAM ranges, already read off the memory map.
    fn scan_region_in(
        control_block: &ControlBlock,
        target_regions: &ScanRegion,
        ram: &[Range<u64>],
        logger: &Logger,
    ) -> ScanRegion {
        match control_block {
            ControlBlock::Exact(address) => return ScanRegion::Exact(*address),
            ControlBlock::Elf(path) => match rtt_elf::control_block_address(path) {
                Ok(address) => {
                    debug!(
                        logger,
                        "RTT control block at {:#010X}, from {}",
                        address,
                        path.display()
                    );
                    return ScanRegion::Exact(address);
                }
                // Losing the ELF costs speed, not the connection.
                Err(err) => warning!(
                    logger,
                    "Cannot read the RTT address from the ELF ({}); scanning the target instead",
                    err
                ),
            },
            ControlBlock::Scan => {}
        }

        if let ScanRegion::Ranges(ranges) = target_regions
            && !ranges.is_empty()
        {
            return ScanRegion::Ranges(ranges.clone());
        }

        ScanRegion::Ranges(Self::scan_windows(ram))
    }

    fn rtt_attach(
        core: &mut Core,
        last_address: &mut Option<u64>,
        control_block: &ControlBlock,
        target_regions: &ScanRegion,
        logger: &Logger,
        rtt_attach_message: &mut Option<String>,
    ) -> Option<Rtt> {
        let rtt = if let Some(addr) = last_address {
            Rtt::attach_at(core, *addr)
        } else {
            let region = Self::scan_region(core, control_block, target_regions, logger);
            // An address the user pinned down is taken at face value: falling
            // back to a full sweep would spend exactly the time they asked to
            // save, and hide the fact that the address is wrong.
            let is_exact = matches!(region, ScanRegion::Exact(_));
            let res = Rtt::attach_region(core, &region);

            if let Err(err) = &res
                && !is_exact
                && !matches!(err, probe_rs::rtt::Error::MultipleControlBlocksFound(_))
            {
                debug!(
                    logger,
                    "No control block at the start of a RAM region, scanning all of it..."
                );
                Rtt::attach(core)
            } else {
                res
            }
        };

        match rtt {
            Ok(rtt) => {
                *last_address = Some(rtt.ptr());
                *rtt_attach_message = None;
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
            Err(err) => {
                // Deduplicated like the session-attach error: the reconnect loop
                // would otherwise repeat it forever, and a wrong `--addr` is
                // exactly the case that needs to be readable.
                let message = format!("Failed to attach to RTT: {}", err);
                if rtt_attach_message.as_ref() != Some(&message) {
                    error!(logger, "{}", message);
                    *rtt_attach_message = Some(message);
                }
                None
            }
        }
    }

    fn log_probe_speed(logger: &Logger, last_logged: &mut LastLogged, speed: u32) {
        let message = format!("Probe speed: {} kHz", speed);
        if last_logged.probe_speed.as_ref() != Some(&message) {
            debug!(logger, "{}", message);
            last_logged.probe_speed = Some(message);
        }
    }

    fn log_fail_to_attach(
        logger: &Logger,
        last_logged: &mut LastLogged,
        res: &Result<Session, probe_rs::Error>,
    ) {
        if let Err(err) = res {
            let message = format!("Failed to attach to target: {}", err);
            if last_logged.session_attach.as_ref() != Some(&message) {
                error!(logger, "{}", message);
                last_logged.session_attach = Some(message);
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
        last_logged: &mut LastLogged,
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
        let control_block = sr.control_block.clone();
        let probe_speed = sr.probe_speed;

        let lister = Lister::new();
        let probes = lister.list_all();
        let Some(new_session) =
            probes
                .get(0)
                .and_then(|probe| probe.open().ok())
                .and_then(|mut probe| {
                    let Ok(speed) = probe.set_speed(probe_speed) else {
                        error!(logger, "Failed to set probe speed");
                        return None;
                    };
                    Self::log_probe_speed(logger, last_logged, speed);
                    let res = probe.attach(&target, Permissions::default());
                    Self::log_fail_to_attach(logger, last_logged, &res);
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

        // The target description can name the RTT scan windows itself; read it
        // off the session before the core borrow takes it.
        let target_regions = session
            .as_ref()
            .map(|s| s.target().rtt_scan_regions.clone())
            .unwrap_or_default();

        let Some(new_rtt) = session
            .as_mut()
            .and_then(|s| s.core(0).ok())
            .and_then(|mut core| {
                debug!(logger, "Attaching to RTT...");
                Self::rtt_attach(
                    &mut core,
                    last_address,
                    &control_block,
                    &target_regions,
                    logger,
                    &mut last_logged.rtt_attach,
                )
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
        last_logged: &mut LastLogged,
    ) -> Option<RttMode> {
        *last_logged = LastLogged::default();
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
        last_logged: &mut LastLogged,
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

        if let Some(control_block) = setup.control_block {
            sw_ref.control_block = control_block;
            has_changes = true;
        }

        if let Some(probe_speed) = setup.probe_speed {
            sw_ref.probe_speed = probe_speed;
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
                last_logged,
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
        headless: bool,
    ) -> Self {
        Self {
            logger,
            tx,
            rx,
            plugin_engine_cmd_sender,
            latency,
            headless,
            last_address: None,
            last_logged: LastLogged::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn logger() -> Logger {
        Logger::new("test".to_string()).0
    }

    /// `ScanRegion` is not `PartialEq`, so assertions compare this instead.
    #[derive(Debug, PartialEq)]
    enum Shape {
        Ram,
        Exact(u64),
        Ranges(Vec<Range<u64>>),
    }

    fn shape(region: &ScanRegion) -> Shape {
        match region {
            ScanRegion::Ram => Shape::Ram,
            ScanRegion::Exact(address) => Shape::Exact(*address),
            ScanRegion::Ranges(ranges) => Shape::Ranges(ranges.clone()),
        }
    }

    /// The windows the memory-map fallback is expected to produce.
    fn imxrt1020_windows() -> Shape {
        Shape::Ranges(RttInterface::scan_windows(&imxrt1020_ram()))
    }

    /// The memory map probe-rs reports for the MIMXRT1020 of issue #248: OCRAM
    /// first (the one holding the block, 0x410 in), then ITCM and DTCM.
    fn imxrt1020_ram() -> Vec<Range<u64>> {
        vec![
            0x2020_0000..0x2024_0000,
            0x0000_0000..0x0004_0000,
            0x2000_0000..0x2004_0000,
        ]
    }

    #[test]
    fn every_ram_region_gets_a_leading_window() {
        assert_eq!(
            RttInterface::scan_windows(&imxrt1020_ram()),
            vec![
                0x2020_0000..0x2020_8000,
                0x0000_0000..0x0000_8000,
                0x2000_0000..0x2000_8000,
            ],
            "768KiB of RAM is probed as three 32KiB windows"
        );
    }

    #[test]
    fn a_region_smaller_than_the_window_is_not_read_past_its_end() {
        // A 2KiB scratch region: reading 32KiB there would run off the end.
        let windows = RttInterface::scan_windows(&[0x2000_0000..0x2000_0800]);

        assert_eq!(windows, vec![0x2000_0000..0x2000_0800]);
    }

    #[test]
    fn empty_and_degenerate_regions_are_dropped() {
        assert!(RttInterface::scan_windows(&[]).is_empty());
        // start == end, and a reversed range: neither names any memory.
        assert!(
            RttInterface::scan_windows(&[0x2000_0000..0x2000_0000, 0x2000_8000..0x2000_0000])
                .is_empty()
        );
    }

    #[test]
    fn an_exact_address_wins_over_every_scan() {
        let region = RttInterface::scan_region_in(
            &ControlBlock::Exact(0x2020_0410),
            &ScanRegion::Ranges(vec![0x2000_0000..0x2000_8000]),
            &imxrt1020_ram(),
            &logger(),
        );

        assert_eq!(shape(&region), Shape::Exact(0x2020_0410));
    }

    #[test]
    fn the_target_description_wins_over_the_memory_map() {
        // A chip that names its own RTT windows: that beats our convention.
        let named = vec![0x2020_0000..0x2020_1000];
        let region = RttInterface::scan_region_in(
            &ControlBlock::Scan,
            &ScanRegion::Ranges(named.clone()),
            &imxrt1020_ram(),
            &logger(),
        );

        assert_eq!(shape(&region), Shape::Ranges(named));
    }

    #[test]
    fn the_memory_map_is_used_when_the_target_names_nothing() {
        // `ScanRegion::Ram` is probe-rs's default for a chip with no
        // `rtt_scan_ranges` of its own, and means "sweep all of it".
        for target_regions in [ScanRegion::Ram, ScanRegion::Ranges(vec![])] {
            let region = RttInterface::scan_region_in(
                &ControlBlock::Scan,
                &target_regions,
                &imxrt1020_ram(),
                &logger(),
            );

            assert_eq!(shape(&region), imxrt1020_windows());
        }
    }

    #[test]
    fn an_unreadable_elf_falls_back_to_scanning() {
        // Losing the ELF must cost speed, not the connection.
        let region = RttInterface::scan_region_in(
            &ControlBlock::Elf(PathBuf::from("/nonexistent/zephyr.elf")),
            &ScanRegion::Ram,
            &imxrt1020_ram(),
            &logger(),
        );

        assert_eq!(shape(&region), imxrt1020_windows());
    }

    #[test]
    fn the_default_probe_speed_is_unchanged() {
        // The speed the code hard-coded before `--speed` existed.
        let shared = RttShared::new(RttSetup::default());

        assert_eq!(shared.probe_speed, 4_000);
        assert_eq!(shared.control_block, ControlBlock::Scan);
    }

    #[test]
    fn setup_values_override_the_defaults() {
        let shared = RttShared::new(RttSetup {
            target: Some("MIMXRT1020".to_string()),
            channel: Some(2),
            control_block: Some(ControlBlock::Exact(0x2020_0410)),
            probe_speed: Some(8_000),
        });

        assert_eq!(shared.channel, 2);
        assert_eq!(shared.probe_speed, 8_000);
        assert_eq!(shared.control_block, ControlBlock::Exact(0x2020_0410));
    }
}
