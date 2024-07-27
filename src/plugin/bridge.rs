use tokio::sync::{broadcast, mpsc};

use super::messages::{PluginExternalRequest, PluginMethodMessage, PluginResponse};

pub struct PluginEngineGate {
    pub sender: broadcast::Sender<PluginMethodMessage<PluginResponse>>,
    pub receiver: mpsc::Receiver<PluginMethodMessage<PluginExternalRequest>>,
    pub method_call_gate: PluginMethodCallGate,
}

pub struct PluginMethodCallGate {
    pub sender: mpsc::Sender<PluginMethodMessage<PluginExternalRequest>>,
    pub receiver: broadcast::Receiver<PluginMethodMessage<PluginResponse>>,
}

impl PluginEngineGate {
    pub fn new(size: usize) -> Self {
        let (sender_req, receiver_req) = mpsc::channel(size);
        let (sender_rsp, receiver_rsp) = broadcast::channel(size);

        let method_call_gate = PluginMethodCallGate {
            sender: sender_req,
            receiver: receiver_rsp,
        };
        Self {
            sender: sender_rsp,
            receiver: receiver_req,
            method_call_gate,
        }
    }

    pub fn new_method_call_gate(&self) -> PluginMethodCallGate {
        PluginMethodCallGate {
            sender: self.method_call_gate.sender.clone(),
            receiver: self.sender.subscribe(),
        }
    }
}
