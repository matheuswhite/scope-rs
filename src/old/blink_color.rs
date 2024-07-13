use ratatui::style::Color;
use std::time::Duration;
use tokio::time::Instant;

pub struct BlinkColor {
    color: Color,
    duration: Duration,
    blinks: usize,
    action: BlinkAction,
}

enum BlinkAction {
    None,
    On { timeout: Instant, blink: usize },
    Off { timeout: Instant, blink: usize },
}

impl BlinkColor {
    pub fn new(color: Color, duration: Duration, blinks: usize) -> Self {
        Self {
            color,
            duration,
            blinks,
            action: BlinkAction::None,
        }
    }

    pub fn start(&mut self) {
        self.action = BlinkAction::On {
            timeout: Instant::now() + self.duration,
            blink: 1,
        }
    }

    pub fn get_color(&self) -> Option<Color> {
        match self.action {
            BlinkAction::None | BlinkAction::Off { .. } => None,
            BlinkAction::On { .. } => Some(self.color),
        }
    }

    pub fn update(&mut self) {
        match self.action {
            BlinkAction::None => {}
            BlinkAction::On { timeout, blink } => {
                if Instant::now() >= timeout {
                    if self.blinks <= blink {
                        self.action = BlinkAction::None;
                    } else {
                        self.action = BlinkAction::Off {
                            timeout: Instant::now() + self.duration,
                            blink,
                        };
                    }
                }
            }
            BlinkAction::Off { timeout, blink } => {
                if Instant::now() >= timeout {
                    if self.blinks <= blink {
                        self.action = BlinkAction::None;
                    } else {
                        self.action = BlinkAction::On {
                            timeout: Instant::now() + self.duration,
                            blink: blink + 1,
                        };
                    }
                }
            }
        }
    }
}
