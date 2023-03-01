use crate::interface::DataOut;
use crate::view::View;
use chrono::{DateTime, Local};
use std::cmp::{max, max_by, min_by};
use std::marker::PhantomData;
use tui::backend::Backend;
use tui::layout::Rect;
use tui::style::{Color, Modifier, Style};
use tui::symbols::Marker;
use tui::text::Span;
use tui::widgets::{Axis, Block, Borders, Chart, Dataset};
use tui::Frame;

pub struct GraphView<B: Backend> {
    history: Vec<GraphData>,
    capacity: usize,
    _marker: PhantomData<B>,
}

impl<B: Backend> GraphView<B> {
    pub fn new(capacity: usize) -> Self {
        Self {
            history: vec![],
            capacity,
            _marker: PhantomData,
        }
    }

    fn get_labels<'a>(bounds: [f64; 2]) -> Vec<Span<'a>> {
        vec![
            Span::styled(
                format!("{:.3}", bounds[0]),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:.3}", bounds[1]),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]
    }
}

impl<B: Backend> GraphView<B> {
    pub fn get_data(&self, index: usize) -> Vec<(f64, f64)> {
        self.history
            .iter()
            .map(|GraphData { timestamp, points }| {
                let p = if let Some(p) = points.get(index) {
                    *p
                } else {
                    0.0
                };
                (*timestamp, p)
            })
            .collect()
    }

    pub fn get_y_top(&self) -> f64 {
        self.history.iter().fold(0.0, |top, x| {
            let max_point = x
                .points
                .clone()
                .into_iter()
                .max_by(|a, b| a.total_cmp(b))
                .unwrap_or(0.0);
            max_by(top, max_point, |a, b| a.total_cmp(b))
        })
    }

    pub fn get_y_bottom(&self) -> f64 {
        self.history.iter().fold(0.0, |bottom, x| {
            let min_point = x
                .points
                .clone()
                .into_iter()
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or(0.0);
            min_by(bottom, min_point, |a, b| a.total_cmp(b))
        })
    }

    pub fn num_collections(&self) -> usize {
        self.history.iter().fold(0, |n, x| max(n, x.points.len()))
    }
}

const MARKERS: [(Color, Marker); 12] = [
    (Color::Cyan, Marker::Dot),
    (Color::Yellow, Marker::Dot),
    (Color::Green, Marker::Dot),
    (Color::Red, Marker::Dot),
    (Color::Blue, Marker::Dot),
    (Color::Magenta, Marker::Dot),
    (Color::Cyan, Marker::Block),
    (Color::Yellow, Marker::Block),
    (Color::Green, Marker::Block),
    (Color::Red, Marker::Block),
    (Color::Blue, Marker::Block),
    (Color::Magenta, Marker::Block),
];

impl<B: Backend> View for GraphView<B> {
    type Backend = B;

    fn draw(&self, f: &mut Frame<Self::Backend>, rect: Rect, _scroll: (u16, u16)) {
        let x_limit = |data: Option<&GraphData>| {
            if let Some(data) = data {
                data.timestamp
            } else {
                Local::now().timestamp() as f64
            }
        };

        let window = rect.width as usize;
        let x_min = x_limit(self.history.first());
        let x_max = x_limit(self.history.last());
        let x_min = if self.history.len() > window {
            x_limit(self.history.get(self.history.len() - window))
        } else {
            x_min
        };

        let x_bounds = [x_min, x_max];
        let y_bounds = [self.get_y_bottom() * 1.2, self.get_y_top() * 1.2];
        let x_labels = GraphView::<B>::get_labels(x_bounds);
        let y_labels = GraphView::<B>::get_labels(y_bounds);

        let mut datas = vec![];
        for i in 0..self.num_collections() {
            datas.push(self.get_data(i));
        }

        let dataset = datas
            .iter()
            .enumerate()
            .map(|(i, data)| {
                let marker_idx = i % MARKERS.len();
                let marker = MARKERS[marker_idx].1;
                Dataset::default()
                    .name(format!(
                        "{} data{i}",
                        match marker {
                            Marker::Dot => "•",
                            Marker::Block => "▮",
                            Marker::Braille => "",
                        }
                    ))
                    .marker(marker)
                    .style(Style::default().fg(MARKERS[marker_idx].0))
                    .data(data)
            })
            .collect();
        let block = Block::default()
            .title(format!("[{:03}] Graph", self.history.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));
        let x_axis = Axis::default()
            .title("Seconds")
            .style(Style::default().fg(Color::Gray))
            .labels(x_labels)
            .bounds(x_bounds);
        let y_axis = Axis::default()
            .title("Content")
            .style(Style::default().fg(Color::Gray))
            .labels(y_labels)
            .bounds(y_bounds);
        let chart = Chart::new(dataset)
            .block(block)
            .x_axis(x_axis)
            .y_axis(y_axis);
        f.render_widget(chart, rect);
    }

    fn add_data_out(&mut self, data: DataOut) {
        if self.history.len() >= self.capacity {
            self.history.remove(0);
        }

        match data {
            DataOut::Data(timestamp, data) => {
                if let Some(graph_data) = GraphData::data(timestamp, data) {
                    self.history.push(graph_data);
                }
            }
            DataOut::ConfirmData(_, _) => {}
            DataOut::ConfirmCommand(_, _, _) => {}
            DataOut::FailData(_, _) => {}
            DataOut::FailCommand(_, _, _) => {}
        }
    }

    fn clear(&mut self) {
        self.history.clear();
    }

    fn toggle_auto_scroll(&mut self) {
        todo!()
    }

    fn max_main_axis(&self, _frame_size: (u16, u16)) -> usize {
        todo!()
    }

    fn save_snapshot(&mut self) {
        todo!()
    }

    fn toggle_snapshot_mode(&mut self) {
        todo!()
    }
}

struct GraphData {
    timestamp: f64,
    points: Vec<f64>,
}

impl GraphData {
    fn get_points(mut content: String) -> Vec<f64> {
        content.retain(|c| !c.is_whitespace());
        let splitted = content.split(',').collect::<Vec<&str>>();

        let mut points = vec![];
        for chunk in splitted {
            if let Ok(p) = chunk.parse::<f64>() {
                points.push(p);
            }
        }
        points
    }

    pub fn data(timestamp: DateTime<Local>, content: String) -> Option<Self> {
        let points = GraphData::get_points(content);

        if points.is_empty() {
            None
        } else {
            Some(Self {
                timestamp: timestamp.timestamp_millis() as f64 / 1000.0,
                points,
            })
        }
    }
}
