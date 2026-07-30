#![deny(warnings)]

extern crate core;

mod graphics;
mod infra;
mod inputs;
mod interfaces;
mod list;
mod plugin;
mod selector;

use crate::infra::tags::TagList;
use crate::interfaces::rtt_if::{RttCommand, RttConnections, RttSetup};
use crate::interfaces::serial_if::SerialCommand;
use crate::interfaces::{InterfaceCommand, InterfaceTask, InterfaceType};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::aot::{Shell, generate};
use graphics::graphics_task::{GraphicsConnections, GraphicsTask};
use infra::config::Config;
use infra::logger::Logger;
use infra::mpmc::Channel;
use infra::session;
use inputs::inputs_task::{InputsConnections, InputsTask};
use inputs::keymap::Keymap;
use interfaces::serial_if::{SerialConnections, SerialSetup};
use list::list_serial_ports;
use plugin::engine::{PluginEngine, PluginEngineConnections};
use std::io::{IsTerminal, stdin, stdout};
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::mpsc::channel;

const DEFAULT_CAPACITY: usize = 2000;
const DEFAULT_TAG_FILE: &str = "tags.yml";

#[derive(Parser)]
// The name is spelled out because `clap_derive` would otherwise take it from
// `CARGO_PKG_NAME` (`scope-monitor`), and the completion scripts would then be
// registered for a command nobody runs — see `Commands::Completions`.
#[command(name = "scope", author, version, about, long_about = None)]
#[command(after_help = "Tip: run `scope completions --help` to enable <Tab> \
completion for scope in your shell.")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Number of scrollback lines kept in memory. Falls back to `capacity` in
    /// config.toml, then to 2000.
    #[clap(short, long)]
    capacity: Option<usize>,
    /// Path to the YAML file whose entries resolve `@name` tags typed in the
    /// command bar. Falls back to `tag_file` in config.toml, then to `tags.yml`.
    /// Used verbatim: `~` and environment variables are not expanded.
    #[clap(short, long)]
    tag_file: Option<PathBuf>,
    /// Polling latency in microseconds, clamped to 0..=100000. Defaults to 100;
    /// 0 yields the thread instead of sleeping.
    #[clap(short, long)]
    latency: Option<u64>,
    /// Base name for the session record file. Defaults to a timestamp.
    #[clap(short, long)]
    name: Option<String>,
    /// Run without the TUI: a transparent stdin/stdout bridge to the wire.
    /// Received bytes print straight to stdout; typed keys are sent raw. Hit
    /// Ctrl+K for the scope command bar (Esc returns to the bridge); quit with
    /// Ctrl+K then Ctrl+Q, or the !exit command.
    #[clap(long)]
    headless: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Open a serial port. Without arguments, pick one interactively.
    Serial {
        /// Serial port to open, e.g. /dev/ttyUSB0 or COM3. Run `scope list` to
        /// see what is available.
        port: Option<String>,
        /// Baud rate to open the port at, e.g. 115200.
        baudrate: Option<u32>,
    },
    /// List the available serial ports.
    List {
        /// Show one table row per USB port with its serial number, PID, VID and
        /// manufacturer. Non-USB ports are omitted from this view.
        #[clap(short, long)]
        verbose: bool,
    },
    /// Connect to a BLE device (not yet implemented).
    Ble {
        /// Advertised name of the BLE device to connect to.
        name_device: String,
        /// ATT MTU to negotiate with the device.
        mtu: u32,
    },
    /// Attach to an RTT target via probe-rs. Without arguments, pick one
    /// interactively.
    Rtt {
        /// Target chip name as probe-rs spells it, e.g. STM32F303.
        target: Option<String>,
        /// RTT channel to attach to. Defaults to 0.
        channel_num: Option<usize>,
    },
    /// Print a shell completion script for `scope` to stdout.
    ///
    /// Install it once so that `scope se<TAB>` completes to `scope serial`:
    ///
    ///   bash        scope completions bash > ~/.local/share/bash-completion/completions/scope
    ///               (macOS: write it to a file and `source` that from ~/.bash_profile)
    ///   zsh         scope completions zsh > ~/.zfunc/_scope
    ///               (~/.zshrc needs `fpath=(~/.zfunc $fpath)` before `compinit`)
    ///   fish        scope completions fish > ~/.config/fish/completions/scope.fish
    ///   powershell  scope completions powershell >> $PROFILE
    ///
    /// Then restart your shell. See the README for the full instructions.
    #[command(verbatim_doc_comment)]
    Completions {
        /// Shell to generate the completion script for.
        #[clap(value_enum)]
        shell: Shell,
    },
}

fn app_serial(
    capacity: usize,
    tag_file: PathBuf,
    port: Option<String>,
    baudrate: Option<u32>,
    latency: u64,
    name: Option<String>,
    headless: bool,
    keymap: Keymap,
) -> Result<(), String> {
    let tag_list = TagList::new(tag_file.clone()).map_err(|err| {
        format!(
            "Failed to read or parse tag file at {}: {}",
            tag_file.display(),
            err
        )
    })?;

    let (logger, logger_receiver) = Logger::new("main".to_string());
    let mut tx_channel = Channel::default();
    let mut rx_channel = Channel::default();

    let mut tx_channel_consumers = (0..3)
        .map(|_| tx_channel.new_consumer())
        .collect::<Vec<_>>();
    let mut rx_channel_consumers = (0..2)
        .map(|_| rx_channel.new_consumer())
        .collect::<Vec<_>>();

    let rx_channel = Arc::new(rx_channel);
    let tx_channel = Arc::new(tx_channel);

    let (serial_if_cmd_sender, serial_if_cmd_receiver) = channel();
    let (inputs_cmd_sender, inputs_cmd_receiver) = channel();
    let (graphics_cmd_sender, graphics_cmd_receiver) = channel();
    let (plugin_engine_cmd_sender, plugin_engine_cmd_receiver) = channel();

    let _ = serial_if_cmd_sender.send(InterfaceCommand::Serial(SerialCommand::Setup(
        SerialSetup {
            port,
            baudrate,
            ..SerialSetup::default()
        },
    )));

    let serial_connections = SerialConnections::new(
        logger.clone().with_source("serial".to_string()),
        tx_channel_consumers.pop().unwrap(),
        rx_channel.clone().new_producer(),
        plugin_engine_cmd_sender.clone(),
        latency,
        headless,
    );
    let inputs_connections = InputsConnections::new(
        logger.clone().with_source("inputs".to_string()),
        tx_channel.clone().new_producer(),
        graphics_cmd_sender.clone(),
        serial_if_cmd_sender.clone(),
        plugin_engine_cmd_sender.clone(),
        rx_channel.clone().new_producer(),
        InterfaceType::Serial,
        headless,
        keymap,
    );

    let serial_if = InterfaceTask::spawn_serial_interface(
        serial_connections,
        serial_if_cmd_sender.clone(),
        serial_if_cmd_receiver,
        SerialSetup::default(),
    );
    let serial_shared = serial_if.shared_ref();

    let plugin_engine_connections = PluginEngineConnections::new(
        logger.clone().with_source("plugin".to_string()),
        tx_channel.new_producer(),
        tx_channel_consumers.pop().unwrap(),
        rx_channel_consumers.pop().unwrap(),
        serial_shared,
        latency,
        InterfaceType::Serial,
        serial_if_cmd_sender,
    );

    let inputs_task = InputsTask::spawn_inputs_task(
        inputs_connections,
        inputs_cmd_sender,
        inputs_cmd_receiver,
        tag_list,
    );

    let inputs_shared = inputs_task.shared_ref();

    // The display task fills the same slot in both modes (same GraphicsCommand
    // channel, same tx/rx consumers), so the rest of the wiring is identical.
    let display = if headless {
        let headless_connections = graphics::headless::HeadlessConnections::new(
            logger_receiver,
            tx_channel_consumers.pop().unwrap(),
            rx_channel_consumers.pop().unwrap(),
            inputs_shared,
            latency,
        );
        graphics::headless::spawn_headless_task(
            headless_connections,
            graphics_cmd_sender,
            graphics_cmd_receiver,
        )
    } else {
        let serial_shared = serial_if.shared_ref();
        let storage_base_filename = session::record_filename(name.as_deref());
        let graphics_config = graphics::graphics_task::GraphicsConfig {
            storage_base_filename,
            capacity,
            latency,
        };
        let graphics_connections = GraphicsConnections::new(
            logger.clone().with_source("graphics".to_string()),
            logger_receiver,
            tx_channel_consumers.pop().unwrap(),
            rx_channel_consumers.pop().unwrap(),
            inputs_shared,
            serial_shared,
            graphics_config,
        );
        GraphicsTask::spawn_graphics_task(
            graphics_connections,
            graphics_cmd_sender,
            graphics_cmd_receiver,
        )
    };
    let plugin_engine = PluginEngine::spawn_plugin_engine(
        plugin_engine_connections,
        plugin_engine_cmd_sender,
        plugin_engine_cmd_receiver,
    );

    serial_if.join();
    inputs_task.join();
    display.join();
    plugin_engine.join();

    Ok(())
}

fn app_rtt(
    capacity: usize,
    tag_file: PathBuf,
    target: Option<String>,
    channel_num: Option<usize>,
    latency: u64,
    name: Option<String>,
    headless: bool,
    keymap: Keymap,
) -> Result<(), String> {
    let tag_list = TagList::new(tag_file.clone()).map_err(|err| {
        format!(
            "Failed to read or parse tag file at {}: {}",
            tag_file.display(),
            err
        )
    })?;

    let (logger, logger_receiver) = Logger::new("main".to_string());
    let mut tx_channel = Channel::default();
    let mut rx_channel = Channel::default();

    let mut tx_channel_consumers = (0..3)
        .map(|_| tx_channel.new_consumer())
        .collect::<Vec<_>>();
    let mut rx_channel_consumers = (0..2)
        .map(|_| rx_channel.new_consumer())
        .collect::<Vec<_>>();

    let rx_channel = Arc::new(rx_channel);
    let tx_channel = Arc::new(tx_channel);

    let (rtt_if_cmd_sender, rtt_if_cmd_receiver) = channel();
    let (inputs_cmd_sender, inputs_cmd_receiver) = channel();
    let (graphics_cmd_sender, graphics_cmd_receiver) = channel();
    let (plugin_engine_cmd_sender, plugin_engine_cmd_receiver) = channel();

    let _ = rtt_if_cmd_sender.send(InterfaceCommand::Rtt(RttCommand::Setup(RttSetup {
        target,
        channel: channel_num,
        ..RttSetup::default()
    })));

    let rtt_connections = RttConnections::new(
        logger.clone().with_source("rtt".to_string()),
        tx_channel_consumers.pop().unwrap(),
        rx_channel.clone().new_producer(),
        plugin_engine_cmd_sender.clone(),
        latency,
        headless,
    );
    let inputs_connections = InputsConnections::new(
        logger.clone().with_source("inputs".to_string()),
        tx_channel.clone().new_producer(),
        graphics_cmd_sender.clone(),
        rtt_if_cmd_sender.clone(),
        plugin_engine_cmd_sender.clone(),
        rx_channel.clone().new_producer(),
        InterfaceType::Rtt,
        headless,
        keymap,
    );

    let rtt_if = InterfaceTask::spawn_rtt_interface(
        rtt_connections,
        rtt_if_cmd_sender.clone(),
        rtt_if_cmd_receiver,
        RttSetup::default(),
    );
    let rtt_shared = rtt_if.shared_ref();

    let plugin_engine_connections = PluginEngineConnections::new(
        logger.clone().with_source("plugin".to_string()),
        tx_channel.new_producer(),
        tx_channel_consumers.pop().unwrap(),
        rx_channel_consumers.pop().unwrap(),
        rtt_shared,
        latency,
        InterfaceType::Rtt,
        rtt_if_cmd_sender,
    );

    let inputs_task = InputsTask::spawn_inputs_task(
        inputs_connections,
        inputs_cmd_sender,
        inputs_cmd_receiver,
        tag_list,
    );

    let inputs_shared = inputs_task.shared_ref();

    let display = if headless {
        let headless_connections = graphics::headless::HeadlessConnections::new(
            logger_receiver,
            tx_channel_consumers.pop().unwrap(),
            rx_channel_consumers.pop().unwrap(),
            inputs_shared,
            latency,
        );
        graphics::headless::spawn_headless_task(
            headless_connections,
            graphics_cmd_sender,
            graphics_cmd_receiver,
        )
    } else {
        let rtt_shared = rtt_if.shared_ref();
        let storage_base_filename = session::record_filename(name.as_deref());
        let graphics_config = graphics::graphics_task::GraphicsConfig {
            storage_base_filename,
            capacity,
            latency,
        };
        let graphics_connections = GraphicsConnections::new(
            logger.clone().with_source("graphics".to_string()),
            logger_receiver,
            tx_channel_consumers.pop().unwrap(),
            rx_channel_consumers.pop().unwrap(),
            inputs_shared,
            rtt_shared,
            graphics_config,
        );
        GraphicsTask::spawn_graphics_task(
            graphics_connections,
            graphics_cmd_sender,
            graphics_cmd_receiver,
        )
    };
    let plugin_engine = PluginEngine::spawn_plugin_engine(
        plugin_engine_connections,
        plugin_engine_cmd_sender,
        plugin_engine_cmd_receiver,
    );

    rtt_if.join();
    inputs_task.join();
    display.join();
    plugin_engine.join();

    Ok(())
}

/// Whether the icon-mode picker can run: it draws a TUI and reads keys, so it
/// only makes sense when both stdin and stdout are a real terminal. Piped or
/// headless-scripted runs fall through and start disconnected, as before.
fn is_interactive() -> bool {
    stdin().is_terminal() && stdout().is_terminal()
}

/// Resolve the serial port/baud, prompting via the icon-mode picker when either
/// is missing and we have an interactive terminal. `Ok(None)` means the user
/// quit the picker before starting the app.
fn resolve_serial(
    port: Option<String>,
    baudrate: Option<u32>,
) -> Result<Option<(Option<String>, Option<u32>)>, String> {
    if (port.is_some() && baudrate.is_some()) || !is_interactive() {
        return Ok(Some((port, baudrate)));
    }

    match selector::select_serial(port.clone(), baudrate)? {
        selector::Outcome::Selected((port, baud)) => Ok(Some((Some(port), Some(baud)))),
        // Skip keeps whatever the CLI gave (possibly nothing) → start disconnected.
        selector::Outcome::Skip => Ok(Some((port, baudrate))),
        selector::Outcome::Quit => Ok(None),
    }
}

/// Resolve the RTT target/channel, prompting via the icon-mode picker when the
/// target is missing and we have an interactive terminal. `Ok(None)` means the
/// user quit the picker before starting the app.
fn resolve_rtt(
    target: Option<String>,
    channel_num: Option<usize>,
) -> Result<Option<(Option<String>, Option<usize>)>, String> {
    if target.is_some() || !is_interactive() {
        return Ok(Some((target, channel_num)));
    }

    match selector::select_rtt(target, channel_num)? {
        selector::Outcome::Selected((target, channel)) => Ok(Some((Some(target), Some(channel)))),
        selector::Outcome::Skip => Ok(Some((None, channel_num))),
        selector::Outcome::Quit => Ok(None),
    }
}

fn main() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    ctrlc::set_handler(|| { /* Do nothing on user ctrl+c */ })
        .expect("Error setting Ctrl-C handler");

    let cli = Cli::parse();

    // Emitting a completion script is pure output, and the shell evaluates it on
    // every start-up — so it is handled before the fallible setup below and
    // returns straight away: no `config.toml` (a typo there would break the
    // user's prompt, not just scope), no keymap, no picker, and crucially not
    // the `See you later ^^` epilogue, which the shell would try to run.
    // Emitting a completion script is pure output, and the shell evaluates it on
    // every start-up — so it is handled before the fallible setup below and
    // returns straight away: no `config.toml` (a typo there would break the
    // user's prompt, not just scope), no keymap, no picker, and crucially not
    // the `See you later ^^` epilogue, which the shell would try to run.
    if let Commands::Completions { shell } = &cli.command {
        let mut cmd = Cli::command();
        generate(*shell, &mut cmd, "scope", &mut stdout());
        return Ok(());
    }

    let latency = cli.latency.unwrap_or(100).clamp(0, 100_000);

    // Everything that can fail fatally — loading `~/.config/scope/config.toml`,
    // sanitizing `--name`, and running the chosen command — funnels into this
    // single `result` so it's reported through the one `[ERR]` formatter below.
    // Each setting resolves as CLI flag > config file > built-in default, and a
    // malformed config is fatal so a typo is reported rather than silently
    // ignored.
    let result = (|| {
        let config = Config::load()?;
        let capacity = cli.capacity.or(config.capacity).unwrap_or(DEFAULT_CAPACITY);
        let tag_file = cli
            .tag_file
            .or(config.tag_file)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_TAG_FILE));
        // Shortcuts have no CLI flag, so precedence is config.toml > default.
        // A bad key string, unknown action, reserved key or duplicate binding
        // is fatal, joining the single `[ERR]` funnel below.
        let keymap = Keymap::from_config(config.shortcuts.as_ref())?;
        let name = cli
            .name
            .as_deref()
            .map(session::sanitize_name)
            .transpose()?;
        let headless = cli.headless;

        match cli.command {
            Commands::Serial { port, baudrate } => match resolve_serial(port, baudrate)? {
                Some((port, baudrate)) => app_serial(
                    capacity, tag_file, port, baudrate, latency, name, headless, keymap,
                ),
                // User quit the picker before connecting.
                None => Ok(()),
            },
            Commands::Ble { .. } => {
                Err("Sorry! We're developing BLE interface and it's not available yet".to_string())
            }
            Commands::List { verbose } => list_serial_ports(verbose),
            Commands::Rtt {
                target,
                channel_num,
            } => match resolve_rtt(target, channel_num)? {
                Some((target, channel_num)) => app_rtt(
                    capacity,
                    tag_file,
                    target,
                    channel_num,
                    latency,
                    name,
                    headless,
                    keymap,
                ),
                None => Ok(()),
            },
            // Handled right after `Cli::parse()`, before this closure, so a
            // completion script never depends on the config loading.
            Commands::Completions { .. } => unreachable!(),
        }
    })();

    if let Err(err) = result {
        eprintln!("[\x1b[31mERR\x1b[0m] {}", err);
        exit(1);
    }

    println!("See you later ^^");
    Ok(())
}
