use crate::interface::DataOut;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::Frame;

pub trait View {
    type Backend: Backend;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect);
    fn add_data_out(&mut self, data: DataOut);
    fn clear(&mut self);
}
