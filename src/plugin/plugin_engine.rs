use crate::{
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::{Consumer, Producer},
        task::Task,
    },
    warning,
};
use std::{
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
    thread::yield_now,
};

pub type PluginEngine = Task<(), PluginEngineCommand>;

pub enum PluginEngineCommand {
    SetLogLevel(LogLevel),
    Exit,
}

#[allow(unused)]
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
        private: PluginEngineConnections,
        cmd_receiver: Receiver<PluginEngineCommand>,
    ) {
        'plugin_engine_loop: loop {
            if let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    PluginEngineCommand::SetLogLevel(_level) => {
                        warning!(private.logger, "Sorry, but we're building this feature...");
                    }
                    PluginEngineCommand::Exit => break 'plugin_engine_loop,
                }
            }

            yield_now();
        }
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
