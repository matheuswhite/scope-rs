extern crate core;

use crate::ble::BleIF;
use crate::command_bar::CommandBar;
use crate::interface::Interface;
use crate::loop_back::LoopBackIF;
use crate::serial::SerialIF;
use btleplug::api::bleuuid::uuid_from_u16;
use chrono::Local;
use clap::{Parser, Subcommand};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use std::io;
use std::io::Stdout;
use std::path::PathBuf;
use std::time::Duration;
use tui::backend::{Backend, CrosstermBackend};
use tui::Terminal;
use uuid::Uuid;

mod ble;
mod command_bar;
mod error_pop_up;
mod interface;
mod loop_back;
mod serial;
mod text;

type ConcreteBackend = CrosstermBackend<Stdout>;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    interface: InterfacesArgs,
    #[clap(short, long)]
    view_length: Option<usize>,
    #[clap(short, long)]
    cmd_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum InterfacesArgs {
    Serial { port: String, baudrate: u32 },
    Ble { name: String },
    Loopback {},
}

const CMD_FILEPATH: &str = "cmds.yaml";
const CAPACITY: usize = 2000;
const BLE_TX_UUID: Uuid = uuid_from_u16(0xAB02);
const BLE_RX_UUID: Uuid = uuid_from_u16(0xAB01);

fn main() -> Result<(), io::Error> {
    let cli = Cli::parse();

    let loopback_graph_fn = || {
        let now = 2.0 * std::f32::consts::PI * 1000.0;
        let now = Local::now().timestamp_millis() % now as i64;
        let now = now as f32 / 1000.0;
        format!(
            "{},{},\x07\x00\x01{},{},{}\x06,{} {}\n",
            f32::sin(now),
            f32::cos(now),
            f32::sin(now) + f32::cos(now),
            -f32::sin(now),
            -f32::cos(now),
            -f32::sin(now) + f32::cos(now),
            "Hello".repeat(10),
        )
    };

    let interface: Box<dyn Interface> = match &cli.interface {
        InterfacesArgs::Serial { port, baudrate } => Box::new(SerialIF::new(port, *baudrate)),
        InterfacesArgs::Ble { name } => Box::new(BleIF::new(BLE_TX_UUID, BLE_RX_UUID, &name)),
        InterfacesArgs::Loopback {} => Box::new(LoopBackIF::new(
            loopback_graph_fn,
            Duration::from_millis(50),
        )),
    };

    let view_length = cli.view_length.unwrap_or(CAPACITY);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut command_bar =
        CommandBar::<ConcreteBackend>::new(interface, view_length).with_command_file(CMD_FILEPATH);

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
