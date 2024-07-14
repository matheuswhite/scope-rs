use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use super::timer::{Timeout, Timer};

#[derive(Default)]
struct TimerOn;
#[derive(Default)]
struct TimerOff;

pub struct Blink<T: Clone> {
    on: T,
    off: T,
    current: T,
    timer_on: Timer<TimerOn, Self>,
    timer_off: Timer<TimerOff, Self>,
    total_blinks: usize,
    num_blinks: usize,
}

impl<T: Clone> Blink<T> {
    pub fn new(duration: Duration, total_blinks: usize, on: T, off: T) -> Arc<Mutex<Self>> {
        let obj = Self {
            on: on.clone(),
            off,
            current: on,
            timer_on: Timer::new(duration),
            timer_off: Timer::new(duration),
            total_blinks,
            num_blinks: 0,
        };
        let obj = Arc::new(Mutex::new(obj));
        {
            let mut o = obj.lock().unwrap();
            o.timer_on.set_action(obj.clone());
            o.timer_off.set_action(obj.clone());
        }

        obj
    }

    pub fn start(&mut self) {
        self.num_blinks = 0;
        self.timer_on.start();
        self.current = self.on.clone();
    }

    pub fn tick(&mut self) {
        self.timer_on.tick();
        self.timer_off.tick();
    }

    pub fn get_current(&self) -> T {
        self.current.clone()
    }
}

impl<T: Clone> Timeout<TimerOn> for Blink<T> {
    fn action(&mut self) {
        self.timer_off.start();
        self.current = self.off.clone();
    }
}

impl<T: Clone> Timeout<TimerOff> for Blink<T> {
    fn action(&mut self) {
        self.num_blinks += 1;
        self.current = self.on.clone();

        if self.num_blinks < self.total_blinks {
            self.timer_on.start();
        }
    }
}
