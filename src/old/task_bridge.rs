use std::future::Future;
use std::sync::Arc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, MutexGuard};

pub struct TaskBridge<Shared, Synchronized, ToTask, FromTask>
where
    Shared: Send + 'static,
    ToTask: Send + 'static,
    FromTask: Send + 'static,
{
    shared: Arc<Mutex<Shared>>,
    synchronized: Synchronized,
    to_task: UnboundedSender<ToTask>,
    from_task: UnboundedReceiver<FromTask>,
}

impl<Shared, Synchronized, ToTask, FromTask> TaskBridge<Shared, Synchronized, ToTask, FromTask>
where
    Shared: Send + 'static,
    ToTask: Send + 'static,
    FromTask: Send + 'static,
{
    pub fn new<T>(shared: Shared, synchronized: Synchronized) -> Self
    where
        T: Task<Shared, ToTask, FromTask>,
    {
        let shared = Arc::new(Mutex::new(shared));
        let (to_task, to_task_rx) = tokio::sync::mpsc::unbounded_channel();
        let (from_task_tx, from_task) = tokio::sync::mpsc::unbounded_channel();

        let shared2 = shared.clone();

        tokio::spawn(async move {
            T::run(shared2, to_task_rx, from_task_tx).await;
        });

        Self {
            shared,
            synchronized,
            to_task,
            from_task,
        }
    }

    pub fn synchronized(&self) -> &Synchronized {
        &self.synchronized
    }

    pub async fn shared(&self) -> MutexGuard<Shared> {
        self.shared.lock().await
    }

    pub fn send(&self, data: ToTask) {
        self.to_task.send(data).unwrap()
    }

    pub fn try_recv(&mut self) -> Result<FromTask, TryRecvError> {
        self.from_task.try_recv()
    }
}

pub trait Task<Shared, ToTask, FromTask> {
    fn run(
        shared: Arc<Mutex<Shared>>,
        from_bridge: UnboundedReceiver<ToTask>,
        to_bridge: UnboundedSender<FromTask>,
    ) -> impl Future<Output = ()> + Send;
}
