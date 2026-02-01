pub mod ansi;
pub mod buffer;
pub mod graphics_task;
pub mod palette;
pub mod screen;
pub mod special_char;

pub trait Serialize {
    fn serialize(&self) -> String;
}
