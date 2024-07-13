use std::sync::{
    mpsc::{Receiver, Sender},
    Arc, RwLock,
};

use crate::{
    graphics::graphics_task::GraphicsCommand,
    infra::{logger::Logger, messages::TimedBytes, mpmc::Producer, task::Task},
    plugin::plugin_engine::PluginEngineCommand,
    serial::serial_if::SerialCommand,
};

pub type InputsTask = Task<InputsShared, ()>;

#[derive(Default)]
pub struct InputsShared {
    pub command_line: String,
    pub cursor: usize,
    pub history_len: usize,
    pub current_hint: Option<String>,
    pub autocomplete_list: Vec<Arc<String>>,
    pub pattern: String,
}

pub struct InputsConnections {
    logger: Logger,
    tx: Producer<Arc<TimedBytes>>,
    graphics_cmd_sender: Sender<GraphicsCommand>,
    serial_if_cmd_sender: Sender<SerialCommand>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
}

impl InputsTask {
    pub fn spawn_inputs_task(
        inputs_connections: InputsConnections,
        inputs_cmd_sender: Sender<()>,
        inputs_cmd_receiver: Receiver<()>,
    ) -> Self {
        Self::new(
            InputsShared::default(),
            inputs_connections,
            Self::task,
            inputs_cmd_sender,
            inputs_cmd_receiver,
        )
    }

    pub fn task(
        _shared: Arc<RwLock<InputsShared>>,
        _private: InputsConnections,
        _inputs_cmd_receiver: Receiver<()>,
    ) {
        todo!()
    }
}

impl InputsConnections {
    pub fn new(
        logger: Logger,
        tx: Producer<Arc<TimedBytes>>,
        graphics_cmd_sender: Sender<GraphicsCommand>,
        serial_if_cmd_sender: Sender<SerialCommand>,
        plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    ) -> Self {
        Self {
            logger,
            tx,
            graphics_cmd_sender,
            serial_if_cmd_sender,
            plugin_engine_cmd_sender,
        }
    }
}
