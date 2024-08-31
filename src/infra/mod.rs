pub mod blink;
pub mod logger;
pub mod messages;
pub mod mpmc;
pub mod recorder;
pub mod task;
pub mod timer;
pub mod typewriter;

pub use logger::LogLevel;

fn into_byte_format(size: u128) -> String {
    let (size, unit) = match size {
        x if x < 1024 => return format!("{} Bytes", size),
        x if x < 1024 * 1024 => (size as f32 / 1024.0, "KB"),
        x if x < 1024 * 1024 * 1024 => (size as f32 / (1024.0 * 1024.0), "MB"),
        _ => (size as f32 / (1024.0 * 1024.0 * 1024.0), "GB"),
    };

    format!("{:.1} {}", size, unit)
}
