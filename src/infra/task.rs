use std::{
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
    thread::{self, JoinHandle},
};

pub struct Task<S, M> {
    shared: Arc<RwLock<S>>,
    cmd_sender: Sender<M>,
    handler: JoinHandle<()>,
}

pub struct Shared<S> {
    shared: Arc<RwLock<S>>,
}

impl<S, M> Task<S, M>
where
    M: Send + 'static,
    S: Send + Sync + 'static,
{
    pub fn new<P: Send + 'static>(
        shared: S,
        private: P,
        run_fn: fn(Arc<RwLock<S>>, P, Receiver<M>),
        cmd_sender: Sender<M>,
        cmd_receiver: Receiver<M>,
    ) -> Self {
        let shared = Arc::new(RwLock::new(shared));
        let shared_clone = shared.clone();

        let handler = thread::spawn(move || {
            run_fn(shared_clone, private, cmd_receiver);
        });

        Self {
            shared,
            cmd_sender,
            handler,
        }
    }

    pub fn join(self) {
        let _ = self.handler.join();
    }

    pub fn shared_ref(&self) -> Shared<S> {
        Shared {
            shared: self.shared.clone(),
        }
    }

    pub fn cmd_sender(&self) -> Sender<M> {
        self.cmd_sender.clone()
    }
}

impl<S> Shared<S> {
    pub fn read(
        &self,
    ) -> Result<
        std::sync::RwLockReadGuard<'_, S>,
        std::sync::PoisonError<std::sync::RwLockReadGuard<'_, S>>,
    > {
        self.shared.read()
    }

    pub fn try_read(
        &self,
    ) -> Result<
        std::sync::RwLockReadGuard<'_, S>,
        std::sync::TryLockError<std::sync::RwLockReadGuard<'_, S>>,
    > {
        self.shared.try_read()
    }
}
