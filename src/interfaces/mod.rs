use std::sync::mpsc::{Receiver, Sender};

use crate::{
    infra::task::Task,
    interfaces::{
        rtt_if::{RttCommand, RttConnections, RttInterface, RttSetup, RttShared},
        serial_if::{SerialCommand, SerialConnections, SerialInterface, SerialSetup, SerialShared},
    },
};

pub mod rtt_if;
pub mod serial_if;

pub type InterfaceTask = Task<InterfaceShared, InterfaceCommand>;

pub enum InterfaceCommand {
    Rtt(RttCommand),
    Serial(SerialCommand),
}

pub enum InterfaceShared {
    Rtt(RttShared),
    Serial(SerialShared),
}

pub enum InterfaceType {
    Rtt,
    Serial,
}

impl InterfaceTask {
    pub fn spawn_serial_interface(
        connections: SerialConnections,
        cmd_sender: Sender<InterfaceCommand>,
        cmd_receiver: Receiver<InterfaceCommand>,
        setup: SerialSetup,
    ) -> Self {
        Self::new(
            InterfaceShared::Serial(SerialShared::new(setup)),
            connections,
            SerialInterface::task,
            cmd_sender,
            cmd_receiver,
        )
    }

    pub fn spawn_rtt_interface(
        connections: RttConnections,
        cmd_sender: Sender<InterfaceCommand>,
        cmd_receiver: Receiver<InterfaceCommand>,
        setup: RttSetup,
    ) -> Self {
        Self::new(
            InterfaceShared::Rtt(RttShared::new(setup)),
            connections,
            RttInterface::task,
            cmd_sender,
            cmd_receiver,
        )
    }
}
