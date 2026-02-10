pub mod blink;
pub mod logger;
pub mod messages;
pub mod mpmc;
pub mod recorder;
pub mod task;
pub mod timer;
pub mod typewriter;

pub use logger::LogLevel;

pub struct ByteFormat(pub String);

impl From<u128> for ByteFormat {
    fn from(size: u128) -> Self {
        let (size, unit) = match size {
            x if x < 1024 => return ByteFormat(format!("{} Bytes", size)),
            x if x < 1024 * 1024 => (size as f32 / 1024.0, "KB"),
            x if x < 1024 * 1024 * 1024 => (size as f32 / (1024.0 * 1024.0), "MB"),
            _ => (size as f32 / (1024.0 * 1024.0 * 1024.0), "GB"),
        };

        ByteFormat(format!("{:.1} {}", size, unit))
    }
}

#[allow(unused)]
macro_rules! dump {
    ($filename:literal, $content:expr) => {
        #[cfg(debug_assertions)]
        std::fs::write($filename, $content).expect(&format!("Failed to write to {}", $filename));
    };
}

pub(crate) use dump;
