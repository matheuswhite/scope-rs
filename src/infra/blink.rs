use std::time::Duration;

use super::timer::{Timeout, Timer};

struct TimerOn;
struct TimerOff;

pub struct Blink<T: Clone> {
    on: T,
    off: T,
    current: T,
    timer_on: Timer,
    timer_off: Timer,
    total_blinks: usize,
    num_blinks: usize,
    total_timer: Timer,
}

impl<T: Clone> Blink<T> {
    pub fn new(duration: Duration, total_blinks: usize, on: T, off: T) -> Self {
        Self {
            on: on.clone(),
            off,
            current: on,
            timer_on: Timer::new(duration),
            timer_off: Timer::new(duration),
            total_blinks,
            num_blinks: 0,
            total_timer: Timer::new(
                duration * (total_blinks as u32) * 2 + Duration::from_millis(100),
            ),
        }
    }

    pub fn start(&mut self) {
        self.num_blinks = 0;
        self.total_timer.start();
        self.timer_on.start();
        self.current = self.on.clone();
    }

    pub fn tick(&mut self) {
        if self.timer_on.tick() {
            self.action(TimerOn);
        }

        if self.timer_off.tick() {
            self.action(TimerOff);
        }

        self.total_timer.tick();
    }

    pub fn get_current(&self) -> T {
        self.current.clone()
    }

    pub fn is_active(&self) -> bool {
        self.total_timer.is_active()
    }
}

impl<T: Clone> Timeout<TimerOn> for Blink<T> {
    fn action(&mut self, _id: TimerOn) {
        self.timer_off.start();
        self.current = self.off.clone();
    }
}

impl<T: Clone> Timeout<TimerOff> for Blink<T> {
    fn action(&mut self, _id: TimerOff) {
        self.num_blinks += 1;
        self.current = self.on.clone();

        if self.num_blinks < self.total_blinks {
            self.timer_on.start();
        }
    }
}
