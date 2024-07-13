use std::time::{Duration, Instant};

pub struct Timer<F> {
    duration: Duration,
    action: Option<F>,
    now: Option<Instant>,
}

impl<F: FnMut()> Timer<F> {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            action: None,
            now: None,
        }
    }

    pub fn set_action(&mut self, action: F) {
        self.action = Some(action);
    }

    pub fn start(&mut self) {
        self.now = Some(Instant::now());
    }

    pub fn is_action_empty(&self) -> bool {
        self.action.as_ref().is_none()
    }

    pub fn tick(&mut self) {
        let Some(now) = self.now.as_ref() else {
            return;
        };

        if now.elapsed() < self.duration {
            return;
        }

        if let Some(action) = self.action.as_mut() {
            (action)();
        }

        self.now.take();
    }
}
