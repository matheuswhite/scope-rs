use std::time::{Duration, Instant};

pub trait Timeout<Id> {
    fn action(&mut self, id: Id);
}

pub struct Timer {
    duration: Duration,
    now: Option<Instant>,
}

impl Timer {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            now: None,
        }
    }

    pub fn start(&mut self) {
        self.now = Some(Instant::now());
    }

    pub fn tick(&mut self) -> bool {
        let Some(now) = self.now.as_ref() else {
            return false;
        };

        if now.elapsed() < self.duration {
            return false;
        }

        self.now.take();

        true
    }

    pub fn is_active(&self) -> bool {
        self.now.is_some()
    }
}
