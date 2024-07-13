#![deny(warnings)]
// TODO remove this allow after migration
#![allow(unused)]

extern crate core;

mod graphics;
mod infra;
mod inputs;
mod plugin;
mod serial;

use chrono::Local;
use graphics::graphics_task::{GraphicsConnections, GraphicsTask};
use infra::logger::Logger;
use infra::mpmc::Channel;
use inputs::inputs_task::{InputsConnections, InputsTask};
use plugin::plugin_engine::{PluginEngine, PluginEngineConnections};
use serial::serial_if::{SerialConnections, SerialInterface, SerialSetup};
use std::sync::mpsc::channel;
use std::sync::Arc;

fn app(capacity: usize) -> Result<(), String> {
    let (logger, logger_receiver) = Logger::new();
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

    let serial_connections = SerialConnections::new(
        logger.clone(),
        tx_channel_consumers.pop().unwrap(),
        rx_channel.clone().new_producer(),
    );
    let inputs_connections = InputsConnections::new(
        logger.clone(),
        tx_channel.clone().new_producer(),
        graphics_cmd_sender.clone(),
        serial_if_cmd_sender.clone(),
        plugin_engine_cmd_sender.clone(),
    );
    let plugin_engine_connections = PluginEngineConnections::new(
        logger.clone(),
        tx_channel.new_producer(),
        tx_channel_consumers.pop().unwrap(),
        rx_channel_consumers.pop().unwrap(),
    );

    let serial_if = SerialInterface::spawn_serial_interface(
        serial_connections,
        serial_if_cmd_sender,
        serial_if_cmd_receiver,
        SerialSetup::default(),
    );
    let inputs_task =
        InputsTask::spawn_inputs_task(inputs_connections, inputs_cmd_sender, inputs_cmd_receiver);

    let inputs_shared = inputs_task.shared_ref();
    let serial_shared = serial_if.shared_ref();

    let now_str = Local::now().format("%Y%m%d_%H%M%S");
    let storage_base_filename = format!("{}.txt", now_str);
    let graphics_connections = GraphicsConnections::new(
        logger.clone(),
        logger_receiver,
        tx_channel_consumers.pop().unwrap(),
        rx_channel_consumers.pop().unwrap(),
        inputs_shared,
        serial_shared,
        storage_base_filename,
        capacity,
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

fn main() {
    #[cfg(target_os = "windows")]
    ctrlc::set_handler(|| { /* Do nothing on user ctrl+c */ })
        .expect("Error setting Ctrl-C handler");

    if let Err(err) = app() {
        println!("[\x1b[31mERR\x1b[0m] {}", err);
    }
}
