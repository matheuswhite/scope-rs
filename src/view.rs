use crate::interface::DataOut;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::Frame;

pub trait View {
    type Backend: Backend;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect, scroll: (u16, u16));
    fn add_data_out(&mut self, data: DataOut);
    fn clear(&mut self);
    fn toggle_auto_scroll(&mut self);
    fn max_main_axis(&self, frame_size: (u16, u16)) -> usize;
    fn save_snapshot(&mut self);
    fn toggle_snapshot_mode(&mut self);
}
