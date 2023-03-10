use std::marker::PhantomData;
use std::time::{Duration, Instant};
use tui::backend::Backend;
use tui::layout::{Alignment, Rect};
use tui::style::{Color, Style};
use tui::text::Span;
use tui::widgets::{Block, Borders, Clear, Paragraph};
use tui::Frame;

pub struct ErrorPopUp<B: Backend> {
    message: String,
    spwan_time: Instant,
    _marker: PhantomData<B>,
}

impl<B: Backend> ErrorPopUp<B> {
    const TIMEOUT: Duration = Duration::from_millis(5000);

    pub fn new(message: String) -> Self {
        Self {
            message,
            _marker: PhantomData,
            spwan_time: Instant::now(),
        }
    }
}

impl<B: Backend> ErrorPopUp<B> {
    pub fn draw(&self, f: &mut Frame<B>, command_bar_y: u16) {
        let area_size = (self.message.chars().count() as u16 + 4, 3);
        let area = Rect::new(
            (f.size().width - area_size.0) / 2,
            command_bar_y - area_size.1 + 1,
            area_size.0,
            area_size.1,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::LightRed));
        let paragraph = Paragraph::new(Span::from(self.message.clone()))
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }

    pub fn is_timeout(&self) -> bool {
        self.spwan_time.elapsed() >= ErrorPopUp::<B>::TIMEOUT
    }
}
