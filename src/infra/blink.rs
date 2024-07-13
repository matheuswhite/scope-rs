use std::time::Duration;

use super::timer::Timer;

pub struct Blink<T: Clone, F> {
    on: T,
    off: T,
    current: Option<T>,
    timer_on: Timer<F>,
    timer_off: Timer<F>,
    total_blinks: usize,
    num_blinks: usize,
}

impl<T: Clone, F: FnMut()> Blink<T, F> {
    pub fn new(duration: Duration, total_blinks: usize, on: T, off: T) -> Self {
        Self {
            on,
            off,
            current: None,
            timer_on: Timer::new(duration),
            timer_off: Timer::new(duration),
            total_blinks,
            num_blinks: 0,
        }
    }

    fn timer_on_timeout(&mut self) {
        self.timer_off.start();
        self.current = Some(self.off.clone());
    }

    fn timer_off_timeout(&mut self) {
        self.timer_on.start();
        self.current = Some(self.on.clone());
    }

    pub fn start(&mut self) {
        self.timer_on.start();
        self.current = Some(self.on.clone());

        self.timer_on.set_action(|| self.timer_on_timeout());
    }

    pub fn tick(&mut self) {
        self.timer_on.tick();
        self.timer_off.tick();
    }

    pub fn get_current(&self) -> Option<&T> {
        self.current.as_ref()
    }
}
