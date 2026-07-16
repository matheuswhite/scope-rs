pub mod ansi;
pub mod buffer;
pub mod graphics_task;
pub mod message_filter;
pub mod palette;
pub mod screen;
pub mod selection;
pub mod special_char;

pub trait Serialize {
    fn serialize(&self) -> String;
}
