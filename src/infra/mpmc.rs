use std::sync::{mpsc, Arc};

pub type Id = usize;

#[derive(Default)]
pub struct Channel<T: Clone> {
    channels: Vec<(Id, mpsc::Sender<T>)>,
}

pub struct Consumer<T: Clone> {
    id: Id,
    receiver: mpsc::Receiver<T>,
}

pub struct Producer<T: Clone> {
    channel: Arc<Channel<T>>,
}

impl<T: Clone> Channel<T> {
    pub fn new_consumer(&mut self) -> Consumer<T> {
        let id = self.channels.len() + 1;
        let (sender, receiver) = mpsc::channel();
        self.channels.push((id, sender));

        Consumer { id, receiver }
    }

    pub fn new_producer(self: Arc<Self>) -> Producer<T> {
        Producer { channel: self }
    }

    fn send_data(self: Arc<Self>, data: T, id: Option<Id>) {
        self.channels
            .iter()
            .filter(|(item_id, _sender)| *item_id != id.unwrap_or(0))
            .for_each(|(_id, sender)| {
                let _ = sender.send(data.clone());
            });
    }
}

impl<T: Clone> Consumer<T> {
    #[allow(unused)]
    pub fn id(&self) -> Id {
        self.id
    }

    #[allow(unused)]
    pub fn recv(&self) -> Result<T, mpsc::RecvError> {
        self.receiver.recv()
    }

    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        self.receiver.try_recv()
    }
}

impl<T: Clone> Producer<T> {
    pub fn produce(&self, data: T) {
        self.channel.clone().send_data(data, None)
    }

    #[allow(unused)]
    pub fn produce_without_loopback(&self, data: T, id: Id) {
        self.channel.clone().send_data(data, Some(id))
    }
}
