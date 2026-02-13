#![deny(warnings)]

extern crate core;

mod graphics;
mod infra;
mod inputs;
mod list;
mod plugin;
mod serial;

use crate::infra::tags::TagList;
use chrono::Local;
use clap::{Parser, Subcommand};
use graphics::graphics_task::{GraphicsConnections, GraphicsTask};
use infra::logger::Logger;
use infra::mpmc::Channel;
use inputs::inputs_task::{InputsConnections, InputsTask};
use list::list_serial_ports;
use plugin::engine::{PluginEngine, PluginEngineConnections};
use serial::serial_if::{SerialConnections, SerialInterface, SerialSetup};
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::mpsc::channel;

const DEFAULT_CAPACITY: usize = 2000;
const DEFAULT_TAG_FILE: &str = "tags.yml";

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[clap(short, long)]
    capacity: Option<usize>,
    #[clap(short, long)]
    tag_file: Option<PathBuf>,
    #[clap(short, long)]
    latency: Option<u64>,
}

#[derive(Subcommand)]
pub enum Commands {
    Serial {
        port: Option<String>,
        baudrate: Option<u32>,
    },
    List {
        #[clap(short, long)]
        verbose: bool,
    },
    Ble {
        name_device: String,
        mtu: u32,
    },
}

fn app(
    capacity: usize,
    tag_file: PathBuf,
    port: Option<String>,
    baudrate: Option<u32>,
    latency: u64,
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

    let _ = serial_if_cmd_sender.send(serial::serial_if::SerialCommand::Setup(SerialSetup {
        port,
        baudrate,
        ..SerialSetup::default()
    }));

    let serial_connections = SerialConnections::new(
        logger.clone().with_source("serial".to_string()),
        tx_channel_consumers.pop().unwrap(),
        rx_channel.clone().new_producer(),
        plugin_engine_cmd_sender.clone(),
        latency,
    );
    let inputs_connections = InputsConnections::new(
        logger.clone().with_source("inputs".to_string()),
        tx_channel.clone().new_producer(),
        graphics_cmd_sender.clone(),
        serial_if_cmd_sender.clone(),
        plugin_engine_cmd_sender.clone(),
        rx_channel.clone().new_producer(),
    );

    let serial_if = SerialInterface::spawn_serial_interface(
        serial_connections,
        serial_if_cmd_sender,
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
    );

    let inputs_task = InputsTask::spawn_inputs_task(
        inputs_connections,
        inputs_cmd_sender,
        inputs_cmd_receiver,
        tag_list,
    );

    let inputs_shared = inputs_task.shared_ref();
    let serial_shared = serial_if.shared_ref();

    let now_str = Local::now().format("%Y%m%d_%H%M%S");
    let storage_base_filename = format!("{}.txt", now_str);
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
    let text_view = GraphicsTask::spawn_graphics_task(
        graphics_connections,
        graphics_cmd_sender,
        graphics_cmd_receiver,
    );
    let plugin_engine = PluginEngine::spawn_plugin_engine(
        plugin_engine_connections,
        plugin_engine_cmd_sender,
        plugin_engine_cmd_receiver,
    );

    serial_if.join();
    inputs_task.join();
    text_view.join();
    plugin_engine.join();

    Ok(())
}

fn main() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    ctrlc::set_handler(|| { /* Do nothing on user ctrl+c */ })
        .expect("Error setting Ctrl-C handler");

    let cli = Cli::parse();

    let (port, baudrate) = match cli.command {
        Commands::Serial { port, baudrate } => (port, baudrate),
        Commands::Ble { .. } => {
            return Err(
                "Sorry! We're developing BLE interface and it's not available yet".to_string(),
            );
        }
        Commands::List { verbose } => {
            return list_serial_ports(verbose);
        }
    };

    let capacity = cli.capacity.unwrap_or(DEFAULT_CAPACITY);
    let tag_file = cli.tag_file.unwrap_or(PathBuf::from(DEFAULT_TAG_FILE));

    if let Err(err) = app(
        capacity,
        tag_file,
        port,
        baudrate,
        cli.latency.unwrap_or(500).clamp(0, 100_000),
    ) {
        eprintln!("[\x1b[31mERR\x1b[0m] {}", err);
        exit(1);
    }

    println!("See you later ^^");
    Ok(())
}
