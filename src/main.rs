use std::{io, thread};
use std::cmp::max;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::sleep;
use std::time::Duration;
use chrono::{DateTime, Local, Utc};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, self};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use tui::backend::{Backend, CrosstermBackend};
use tui::{Frame, Terminal};
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Modifier, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Paragraph, Wrap};

const PORT_NAME: &'static str = "COM8";
const BAUD_RATE: u32 = 115200;
const TIME_ZONE_OFFSET: i8 = -3;

enum Cmd {
    Input(KeyCode),
    Data(DateTime<Local>, String),
}

fn serial_task(serial_rx: Receiver<String>, data_tx: Sender<Cmd>) {
    let mut serial = serialport::new(PORT_NAME, BAUD_RATE)
        .open()
        .expect("Failed to open serial port");

    let mut line = String::new();
    let mut buffer = [0u8; 1];

    loop {
        if let Ok(data_to_send) = serial_rx.try_recv() {
            serial.write(data_to_send.as_bytes())
                .expect("Cannot write data to serial");
        }

        match serial.read(&mut buffer) {
            Ok(_) => {
                if buffer[0] == '\n' as u8 {
                    data_tx.send(Cmd::Data(Local::now(), line.clone()))
                        .expect("Cannot forward message read from serial");
                    line.clear();
                } else {
                    line.push(buffer[0] as char);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => eprint!("{:?}", e)
        }
    }
}

#[derive(Clone)]
enum SerialData {
    Received(DateTime<Local>, String),
    Sent(DateTime<Local>, String),
}

fn tui_ui<B: Backend>(f: &mut Frame<B>, paragraph: Vec<SerialData>, command_line: String) {
    let bottom_bar_height = 3;
    let monitor_height = f.size().height - bottom_bar_height;
    let scroll = if paragraph.len() as u16 > monitor_height - 2 {
        paragraph.len() as u16 + 2 - monitor_height
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(f.size().height - bottom_bar_height),
            Constraint::Length(bottom_bar_height),
        ].as_ref())
        .split(f.size());

    /* Monitor */
    let block = Block::default()
        .title(format!("Monitor [{:03}]", paragraph.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let text = paragraph
        .iter()
        .map(|x| {
            match x {
                // TODO Decode ANSI colors
                SerialData::Received(timestamp, line) => Spans::from(
                    Span::styled(
                        format!("[{}] {}", timestamp.format("%d/%m/%Y %H:%M:%S"), line),
                        Style::default()
                            .fg(Color::White),
                    )
                ),
                SerialData::Sent(timestamp, line) => Spans::from(
                    Span::styled(
                        format!("[{}] {}", timestamp.format("%d/%m/%Y %H:%M:%S"), line),
                        Style::default()
                            .bg(Color::Cyan)
                            .fg(Color::Black),
                    )
                ),
            }
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));
    f.render_widget(paragraph, chunks[0]);

    /* Command */
    let block = Block::default().title(format!("Command")).borders(Borders::ALL);
    let paragraph = Paragraph::new(Span::from(command_line))
        .block(block);
    f.render_widget(paragraph, chunks[1]);
}

fn tui_task(serial_tx: Sender<String>, data_rx: Receiver<Cmd>) -> Result<(), io::Error> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut lines = vec![];
    let mut command_line = String::new();

    'tui_loop: loop {
        if let Ok(cmd) = data_rx.try_recv() {
            match cmd {
                Cmd::Input(code) => {
                    match code {
                        KeyCode::Char(c) => command_line.push(c),
                        KeyCode::Enter => {
                            // TODO Transform messages starts with / to command loaded from YAML file

                            serial_tx.send(command_line.clone()).unwrap();
                            lines.push(SerialData::Sent(Local::now(), command_line.clone()));
                            command_line.clear();
                        }
                        KeyCode::Backspace => {
                            command_line.pop();
                        }
                        KeyCode::Esc => {
                            break 'tui_loop;
                        }
                        _ => {}
                    }
                }
                Cmd::Data(timestamp, line) => lines.push(SerialData::Received(timestamp, line.clone())),
            }
        }

        terminal.draw(|f|
            tui_ui(f, lines.clone(), command_line.clone())
        )?;

        if lines.len() > 100 {
            lines.remove(0);
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}

fn fake_serial_task(serial_rx: Receiver<String>, data_tx: Sender<Cmd>) {
    let mut counter = 0;

    loop {
        data_tx.send(Cmd::Data(Local::now(), format!("Hello {}", counter))).unwrap();
        counter += 1;
        sleep(Duration::from_millis(1000));
    }
}

fn main() {
    let (serial_tx, serial_rx) = channel();
    let (data_tx, data_rx) = channel();
    let data_tx2 = data_tx.clone();

    thread::spawn(move || {
        // serial_task(serial_rx, data_tx);
        fake_serial_task(serial_rx, data_tx);
    });

    let tui_task_handler = thread::spawn(move || {
        tui_task(serial_tx, data_rx).unwrap();
    });

    thread::spawn(move || {
        loop {
            if let Event::Key(key) = event::read().unwrap() {
                data_tx2.send(Cmd::Input(key.code)).unwrap();
            }
        }
    });

    tui_task_handler.join().unwrap();
}
