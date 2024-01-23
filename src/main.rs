#![deny(warnings)]

extern crate core;

use crate::command_bar::CommandBar;
use crate::serial::SerialIF;
use clap::Parser;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io;
use std::io::Stdout;
use std::path::PathBuf;
use tui::backend::{Backend, CrosstermBackend};
use tui::Terminal;

mod command_bar;
mod error_pop_up;
mod messages;
mod serial;
mod text;

type ConcreteBackend = CrosstermBackend<Stdout>;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    port: String,
    baudrate: u32,
    #[clap(short, long)]
    view_length: Option<usize>,
    #[clap(short, long)]
    cmd_file: Option<PathBuf>,
}

const CMD_FILEPATH: &str = "cmds.yaml";
const CAPACITY: usize = 2000;

fn main() -> Result<(), io::Error> {
    let cli = Cli::parse();

    let interface = SerialIF::new(&cli.port, cli.baudrate);

    let view_length = cli.view_length.unwrap_or(CAPACITY);
    let cmd_file = cli.cmd_file.unwrap_or(PathBuf::from(CMD_FILEPATH));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut command_bar = CommandBar::<ConcreteBackend>::new(interface, view_length)
        .with_command_file(cmd_file.as_path().to_str().unwrap());

    'main: loop {
        terminal.draw(|f| command_bar.draw(f))?;

        if command_bar
            .update(terminal.backend().size().unwrap())
            .is_err()
        {
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
