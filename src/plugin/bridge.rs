use std::collections::HashMap;

use tokio::sync::mpsc;

use super::messages::{PluginExternalRequest, PluginMethodMessage, PluginResponse};

pub struct PluginEngineGate {
    pub receiver: mpsc::Receiver<PluginMethodMessage<PluginExternalRequest>>,
    request_sender: mpsc::Sender<PluginMethodMessage<PluginExternalRequest>>,
    calls: HashMap<u64, mpsc::UnboundedSender<PluginResponse>>,
    next_id: u64,
}

/// One method call's end of the engine: the channel it sends its requests on,
/// and its own channel for the responses.
pub struct PluginMethodCallGate {
    pub id: u64,
    pub sender: mpsc::Sender<PluginMethodMessage<PluginExternalRequest>>,
    pub receiver: mpsc::UnboundedReceiver<PluginResponse>,
}

impl PluginEngineGate {
    pub fn new(size: usize) -> Self {
        let (request_sender, receiver) = mpsc::channel(size);

        Self {
            receiver,
            request_sender,
            calls: HashMap::new(),
            next_id: 0,
        }
    }

    pub fn new_method_call_gate(&mut self) -> PluginMethodCallGate {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let (sender, receiver) = mpsc::unbounded_channel();
        self.calls.insert(id, sender);

        PluginMethodCallGate {
            id,
            sender: self.request_sender.clone(),
            receiver,
        }
    }

    /// Sends `response` to the method call `method_id`, if it still runs.
    pub fn respond(&self, method_id: u64, response: PluginResponse) {
        if let Some(call) = self.calls.get(&method_id) {
            let _ = call.send(response);
        }
    }

    /// Forgets the method call `method_id`, and every call whose gate was
    /// dropped without running.
    pub fn finished(&mut self, method_id: u64) {
        self.calls.remove(&method_id);
        self.calls.retain(|_, call| !call.is_closed());
    }
}
