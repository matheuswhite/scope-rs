use crate::interface::DataOut;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::Frame;

pub trait View {
    type Backend: Backend;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect);
    fn add_data_out(&mut self, data: DataOut);
    fn clear(&mut self);
    fn up_scroll(&mut self);
    fn down_scroll(&mut self);
    fn left_scroll(&mut self);
    fn right_scroll(&mut self);
    fn save_snapshot(&mut self);
    fn toggle_snapshot_mode(&mut self);
    fn set_frame_height(&mut self, frame_height: u16);
    fn update_scroll(&mut self);
}
