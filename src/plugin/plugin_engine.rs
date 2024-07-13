use std::sync::{
    mpsc::{Receiver, Sender},
    Arc, RwLock,
};

use crate::infra::{
    logger::Logger,
    messages::TimedBytes,
    mpmc::{Consumer, Producer},
    task::Task,
};

pub type PluginEngine = Task<(), PluginEngineCommand>;

pub enum PluginEngineCommand {
    Exit,
}

pub struct PluginEngineConnections {
    logger: Logger,
    tx_producer: Producer<Arc<TimedBytes>>,
    tx_consumer: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
}

impl PluginEngine {
    pub fn spawn_plugin_engine(
        connections: PluginEngineConnections,
        sender: Sender<PluginEngineCommand>,
        receiver: Receiver<PluginEngineCommand>,
    ) -> Self {
        Self::new((), connections, Self::task, sender, receiver)
    }

    pub fn task(
        _shared: Arc<RwLock<()>>,
        _private: PluginEngineConnections,
        _receiver: Receiver<PluginEngineCommand>,
    ) {
        todo!()
    }
}

impl PluginEngineConnections {
    pub fn new(
        logger: Logger,
        tx_producer: Producer<Arc<TimedBytes>>,
        tx_consumer: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
    ) -> Self {
        Self {
            logger,
            tx_producer,
            tx_consumer,
            rx,
        }
    }
}
