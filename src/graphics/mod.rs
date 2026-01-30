pub mod bytes;
pub mod graphics_task;
pub mod selection;

pub trait Serialize {
    fn serialize(&self) -> String;
}
