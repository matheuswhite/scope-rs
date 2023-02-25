use crate::interface::DataOut;
use crate::view::View;
use chrono::{DateTime, Local};
use std::marker::PhantomData;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Paragraph};
use tui::Frame;

pub struct TextView<B: Backend> {
    history: Vec<ViewData>,
    _marker: PhantomData<B>,
}

impl<B: Backend> TextView<B> {
    pub fn new() -> Self {
        Self {
            history: vec![],
            _marker: PhantomData,
        }
    }
}

impl<B: Backend> View for TextView<B> {
    type Backend = B;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect) {
        // TODO Add scroll

        let block = Block::default()
            .title(format!("[{:03}] Text UTF-8", self.history.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::White));
        let text = self
            .history
            .iter()
            .map(|x| {
                let tm_fg = if x.bg != Color::Reset { x.bg } else { x.fg };

                Spans::from(vec![
                    Span::styled(
                        format!("[{}] ", x.timestamp.format("%d/%m/%Y %H:%M:%S")),
                        Style::default().fg(tm_fg),
                    ),
                    Span::styled(
                        format!(
                            "{}{} ",
                            if x.bg != Color::Reset { " " } else { "" },
                            x.content
                        ),
                        Style::default().bg(x.bg).fg(x.fg),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block).scroll((0, 0));
        f.render_widget(paragraph, rect);
    }

    fn add_data_out(&mut self, data: DataOut) {
        // TODO Remove old datas

        self.history.push(match data {
            DataOut::Data(timestamp, data) => ViewData::if_data(timestamp, data),
            DataOut::ConfirmData(timestamp, data) => ViewData::user_data(timestamp, data),
            DataOut::ConfirmCommand(timestamp, cmd_name, data) => {
                ViewData::user_command(timestamp, cmd_name, data)
            }
            DataOut::FailData(timestamp, data) => ViewData::fail_data(timestamp, data),
            DataOut::FailCommand(timestamp, cmd_name, _data) => {
                ViewData::fail_command(timestamp, cmd_name)
            }
        });
    }

    fn clear(&mut self) {
        self.history.clear();
    }
}

struct ViewData {
    timestamp: DateTime<Local>,
    content: String,
    fg: Color,
    bg: Color,
}

impl ViewData {
    fn if_data(timestamp: DateTime<Local>, content: String) -> Self {
        Self {
            timestamp,
            content,
            fg: Color::White,
            bg: Color::Reset,
        }
    }

    fn user_data(timestamp: DateTime<Local>, content: String) -> Self {
        Self {
            timestamp,
            content,
            fg: Color::Black,
            bg: Color::LightCyan,
        }
    }

    fn user_command(timestamp: DateTime<Local>, cmd_name: String, content: String) -> Self {
        Self {
            timestamp,
            content: format!("</{cmd_name}> {content}"),
            fg: Color::Black,
            bg: Color::LightGreen,
        }
    }

    fn fail_data(timestamp: DateTime<Local>, content: String) -> Self {
        Self {
            timestamp,
            content: format!("Cannot send \"{content}\""),
            fg: Color::White,
            bg: Color::LightRed,
        }
    }

    fn fail_command(timestamp: DateTime<Local>, cmd_name: String) -> Self {
        Self {
            timestamp,
            content: format!("Cannot send </{cmd_name}>"),
            fg: Color::White,
            bg: Color::LightRed,
        }
    }
}

// fn decode_ansi_color(text: &str) -> Vec<(String, Color)> {
//     if text.is_empty() {
//         return vec![];
//     }
//
//     let splitted = text.split("\x1B[").collect::<Vec<_>>();
//     let mut res = vec![];
//
//     let pattern_n_color = [
//         ("0m", Color::White),
//         ("30m", Color::Black),
//         ("0;30m", Color::Black),
//         ("31m", Color::Red),
//         ("0;31m", Color::Red),
//         ("32m", Color::Green),
//         ("0;32m", Color::Green),
//         ("33m", Color::Yellow),
//         ("0;33m", Color::Yellow),
//         ("34m", Color::Blue),
//         ("0;34m", Color::Blue),
//         ("35m", Color::Magenta),
//         ("0;35m", Color::Magenta),
//         ("36m", Color::Cyan),
//         ("0;36m", Color::Cyan),
//         ("37m", Color::Gray),
//         ("0;37m", Color::Gray),
//     ];
//
//     for splitted_str in splitted.iter() {
//         if splitted_str.is_empty() {
//             continue;
//         }
//
//         if pattern_n_color.iter().all(|(pattern, color)| {
//             if splitted_str.starts_with(pattern) {
//                 let final_str = splitted_str
//                     .to_string()
//                     .replace(pattern, "")
//                     .trim()
//                     .to_string();
//                 if final_str.is_empty() {
//                     return true;
//                 }
//
//                 res.push((final_str, *color));
//                 return false;
//             }
//
//             true
//         }) && !splitted_str.starts_with("0m")
//         {
//             res.push((splitted_str.to_string(), Color::White));
//         }
//     }
//
//     res
// }
//
// fn how_many_lines(text: &str, initial_offset: usize, view_width: usize) -> usize {
//     match initial_offset + text.len() {
//         v if v < view_width => return 1,
//         v if v == view_width => return 2,
//         _ => {}
//     }
//
//     1 + how_many_lines(&text[(view_width - initial_offset)..], 0, view_width)
// }
//
// fn calc_scroll_pos(n_lines: u16, height: u16) -> u16 {
//     if n_lines <= height {
//         0
//     } else {
//         n_lines - height
//     }
// }
