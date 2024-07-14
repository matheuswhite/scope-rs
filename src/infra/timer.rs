use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub trait Timeout<Id> {
    fn action(&mut self);
}

pub struct Timer<Id: Default, T: Timeout<Id>> {
    duration: Duration,
    action: Option<Arc<Mutex<T>>>,
    now: Option<Instant>,
    marker: PhantomData<Id>,
}

impl<Id: Default, T: Timeout<Id>> Timer<Id, T> {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            action: None,
            now: None,
            marker: PhantomData,
        }
    }

    pub fn set_action(&mut self, action: Arc<Mutex<T>>) {
        self.action = Some(action);
    }

    pub fn start(&mut self) {
        self.now = Some(Instant::now());
    }

    pub fn tick(&mut self) {
        let Some(now) = self.now.as_ref() else {
            return;
        };

        if now.elapsed() < self.duration {
            return;
        }

        if let Some(action) = self.action.as_ref() {
            let mut action = action.lock().unwrap();
            action.action();
        }

        self.now.take();
    }
}
