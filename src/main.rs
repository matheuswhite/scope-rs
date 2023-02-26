use crate::command_bar::CommandBar;
use crate::graph::GraphView;
use crate::loop_back::LoopBackIF;
use crate::text::TextView;
use crate::view::View;
use chrono::Local;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io;
use std::io::Stdout;
use std::time::Duration;
use tui::backend::CrosstermBackend;
use tui::Terminal;

mod command_bar;
mod error_pop_up;
mod graph;
mod interface;
mod loop_back;
mod serial;
mod text;
mod view;

type ConcreteBackend = CrosstermBackend<Stdout>;

const CMD_FILEPATH: &str = "cmds.yaml";
const CAPACITY: usize = 250;

fn main() -> Result<(), io::Error> {
    let views: Vec<Box<dyn View<Backend = ConcreteBackend>>> = vec![
        Box::new(TextView::new(CAPACITY)),
        Box::new(GraphView::new(CAPACITY)),
    ];

    let interface = Box::new(LoopBackIF::new(
        || {
            let now = 2.0 * std::f32::consts::PI * 1000.0;
            let now = Local::now().timestamp_millis() % now as i64;
            let now = now as f32 / 1000.0;
            format!(
                "{},{},{},{},{},{}",
                f32::sin(now),
                f32::cos(now),
                f32::sin(now) + f32::cos(now),
                -f32::sin(now),
                -f32::cos(now),
                -f32::sin(now) + f32::cos(now),
            )
        },
        Duration::from_millis(100),
    ));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut command_bar =
        CommandBar::<ConcreteBackend>::new(interface, views).with_command_file(CMD_FILEPATH);

    'main: loop {
        terminal.draw(|f| command_bar.draw(f))?;

        if command_bar.update().is_err() {
            break 'main;
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
