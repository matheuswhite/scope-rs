pub mod bytes;
pub mod graphics_task;

pub trait Serialize {
    fn serialize(&self) -> String;
}
