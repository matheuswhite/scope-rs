#![deny(warnings)]

extern crate core;

use crate::command_bar::CommandBar;
use crate::serial::SerialIF;
use chrono::Local;
use clap::{Parser, Subcommand};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::Terminal;
use std::io;
use std::io::Stdout;
use std::path::PathBuf;

mod blink_color;
mod command_bar;
mod error_pop_up;
mod messages;
mod mpmc;
mod recorder;
mod rich_string;
mod serial;
mod task_bridge;
mod text;
mod typewriter;

pub type ConcreteBackend = CrosstermBackend<Stdout>;

#[derive(Subcommand)]
pub enum Commands {
    Serial {
        port: Option<String>,
        baudrate: Option<u32>,
    },
    Ble {
        name_device: String,
        mtu: u32,
    },
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[clap(short, long)]
    view_length: Option<usize>,
    #[clap(short, long)]
    cmd_file: Option<PathBuf>,
}

const CMD_FILEPATH: &str = "cmds.yaml";
const CAPACITY: usize = 2000;

async fn app() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    ctrlc::set_handler(|| { /* Do nothing on user ctrl+c */ })
        .expect("Error setting Ctrl-C handler");

    let cli = Cli::parse();

    let view_length = cli.view_length.unwrap_or(CAPACITY);
    let cmd_file = cli.cmd_file.unwrap_or(PathBuf::from(CMD_FILEPATH));
    let interface;

    match cli.command {
        Commands::Serial { port, baudrate } => {
            let port_select = port.unwrap_or("".to_string());
            let baudrate_select = baudrate.unwrap_or(0);

            interface = SerialIF::build_and_connect(&port_select, baudrate_select);
        }
        _ => {
            unimplemented!()
        }
    }

    enable_raw_mode().map_err(|_| "Cannot enable terminal raw mode".to_string())?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(|_| "Cannot enable alternate screen and mouse capture".to_string())?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        Terminal::new(backend).map_err(|_| "Cannot create terminal backend".to_string())?;

    let datetime = Local::now().format("%Y%m%d_%H%M%S");
    let mut command_bar = CommandBar::new(interface, view_length, format!("{}.txt", datetime))
        .with_command_file(cmd_file.as_path().to_str().unwrap());

    'main: loop {
        {
            let text_view = command_bar.get_text_view().await;
            let interface = command_bar.get_interface().await;

            terminal
                .draw(|f| command_bar.draw(f, &text_view, &interface))
                .map_err(|_| "Fail at terminal draw".to_string())?;
        }

        if command_bar
            .update(terminal.backend().size().unwrap())
            .await
            .is_err()
        {
            break 'main;
        }
    }

    disable_raw_mode().map_err(|_| "Cannot disable terminal raw mode".to_string())?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )
    .map_err(|_| "Cannot disable alternate screen and mouse capture".to_string())?;
    terminal
        .show_cursor()
        .map_err(|_| "Cannot show mouse cursor".to_string())?;

    Ok(())
}

#[tokio::main]
async fn main() {
    if let Err(err) = app().await {
        println!("[\x1b[31mERR\x1b[0m] {}", err);
    }
}
