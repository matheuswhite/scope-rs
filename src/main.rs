use std::{io, thread};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread::sleep;
use std::time::Duration;
use chrono::{DateTime, format, Local};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, Event, KeyCode, self};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use tui::backend::{Backend, CrosstermBackend};
use tui::{Frame, Terminal};
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Paragraph, Wrap};

const PORT_NAME: &str = "COM8";
const BAUD_RATE: u32 = 115200;
const CMD_YAML_FILEPATH: &str = "cmds.yaml";

enum SentType {
    Ok,
    Fail,
    Complex(String),
}

enum Cmd {
    Input(KeyCode),
    Data(DateTime<Local>, String),
    Sent(DateTime<Local>, String, SentType),
}

enum SerialToSent {
    Simple(String),
    Complex(String, String),
}

fn serial_task(serial_rx: Receiver<SerialToSent>, data_tx: Sender<Cmd>) {
    let mut serial = serialport::new(PORT_NAME, BAUD_RATE)
        .timeout(Duration::from_millis(100))
        .open()
        .expect("Failed to open serial port");

    let mut line = String::new();
    let mut buffer = [0u8; 1];

    loop {
        if let Ok(data_to_send) = serial_rx.try_recv() {
            let (data_to_send, sent_type) = match data_to_send {
                SerialToSent::Simple(data_to_send) => {
                    match serial.write(data_to_send.clone().as_bytes()) {
                        Ok(_) => (data_to_send, SentType::Ok),
                        Err(_) => (data_to_send, SentType::Fail),
                    }
                }
                SerialToSent::Complex(name, data_to_send) => {
                    match serial.write(data_to_send.clone().as_bytes()) {
                        Ok(_) => (data_to_send, SentType::Complex(name)),
                        Err(_) => (data_to_send, SentType::Fail),
                    }
                }
            };

            data_tx.send(Cmd::Sent(Local::now(), data_to_send, sent_type))
                .expect("Cannot sent data write feedback");
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
            Err(e) => eprint!("{e:?}")
        }
    }
}

#[derive(Clone)]
enum SerialData {
    Received(DateTime<Local>, String),
    Sent(DateTime<Local>, String, Color),
}

impl Into<String> for SerialData {
    fn into(self) -> String {
        match self {
            SerialData::Received(_, str) => str,
            SerialData::Sent(_, str, _) => str,
        }
    }
}

fn decode_ansi_color(text: &str) -> Vec<(String, Color)> {
    if text.is_empty() {
        return vec![];
    }

    let splitted = text.split("\x1B[").collect::<Vec<_>>();
    let mut res = vec![];

    let pattern_n_color = [
        ("0m", Color::White),
        ("30m", Color::Black),
        ("0;30m", Color::Black),
        ("31m", Color::Red),
        ("0;31m", Color::Red),
        ("32m", Color::Green),
        ("0;32m", Color::Green),
        ("33m", Color::Yellow),
        ("0;33m", Color::Yellow),
        ("34m", Color::Blue),
        ("0;34m", Color::Blue),
        ("35m", Color::Magenta),
        ("0;35m", Color::Magenta),
        ("36m", Color::Cyan),
        ("0;36m", Color::Cyan),
        ("37m", Color::Gray),
        ("0;37m", Color::Gray),
    ];

    for splitted_str in splitted.iter() {
        if splitted_str.is_empty() {
            continue;
        }

        if pattern_n_color.iter().all(|(pattern, color)| {
            if splitted_str.starts_with(pattern) {
                let final_str = splitted_str.to_string().replace(pattern, "").trim().to_string();
                if final_str.is_empty() {
                    return true;
                }

                res.push((final_str, *color));
                return false;
            }

            true
        }) && !splitted_str.starts_with("0m") {
            res.push((splitted_str.to_string(), Color::White));
        }
    }

    res
}

fn how_many_lines(text: &str, initial_offset: usize, view_width: usize) -> usize {
    match initial_offset + text.len() {
        v if v < view_width => return 1,
        v if v == view_width => return 2,
        _ => {},
    }

    1 + how_many_lines(&text[(view_width - initial_offset)..], 0, view_width)
}

fn calc_scroll_pos(n_lines: u16, height: u16) -> u16 {
    if n_lines <= height {
        0
    } else {
        n_lines - height
    }
}

fn tui_ui<B: Backend>(f: &mut Frame<B>, paragraph: Vec<SerialData>, command_line: String) {
    let bottom_bar_height = 3;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(f.size().height - bottom_bar_height),
            Constraint::Length(bottom_bar_height),
        ].as_ref())
        .split(f.size());

    let frame_width = f.size().width as usize - 2;
    let frame_height = f.size().height - 5;
    let timestamp_width = format!("{} ", Local::now().format("%d/%m/%Y %H:%M:%S")).len();
    let n_lines = paragraph.iter().fold(0, |x, serial_data| {
        let line: String = serial_data.clone().into();
        x + how_many_lines(&line, timestamp_width, frame_width)
    });

    let scroll = calc_scroll_pos(n_lines as u16, frame_height);

    /* Monitor */
    let block = Block::default()
        .title(format!("[{:03}] Serial {}:{}bps", paragraph.len(), PORT_NAME, BAUD_RATE))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    let text = paragraph
        .iter()
        .map(|x| {
            let received_span = |timestamp: &DateTime<Local>, line, color| {
                Span::styled(
                    format!("[{}] {}", timestamp.format("%d/%m/%Y %H:%M:%S"), line),
                    Style::default().fg(color),
                )
            };

            let sent_span = |timestamp: &DateTime<Local>, line, color| {
                Spans::from(vec![
                    Span::styled(format!("[{}] ", timestamp.format("%d/%m/%Y %H:%M:%S")), Style::default()
                        .fg(color)),
                    Span::styled(format!(" {line} "), Style::default()
                        .bg(color)
                        .fg(Color::Black)),
                ])
            };

            match x {
                SerialData::Received(timestamp, line) => {
                    let decoded_line = decode_ansi_color(line);
                    let mut span_vec = vec![];

                    for (line, color) in decoded_line.iter() {
                        span_vec.push(received_span(timestamp, line, *color));
                    }

                    Spans::from(span_vec)
                }
                SerialData::Sent(timestamp, line, color) => {
                    sent_span(timestamp, line, *color)
                }
            }
        })
        .collect::<Vec<_>>();
    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(paragraph, chunks[0]);

    /* Command */
    let cursor_pos = (chunks[1].x + command_line.len() as u16 + 1, chunks[1].y + 1);
    let block = Block::default().title("Command").borders(Borders::ALL);
    let paragraph = Paragraph::new(Span::from(command_line)).block(block);
    f.render_widget(paragraph, chunks[1]);
    f.set_cursor(cursor_pos.0, cursor_pos.1);
}

fn tui_task(serial_tx: Sender<SerialToSent>, data_rx: Receiver<Cmd>, yaml_cmds: BTreeMap<String, String>) -> Result<(), io::Error> {
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
                            if command_line.starts_with('/') {
                                let key = command_line.strip_prefix('/').unwrap();
                                if yaml_cmds.contains_key(key) {
                                    let data_to_send = yaml_cmds.get(key).unwrap().clone();
                                    serial_tx.send(SerialToSent::Complex(command_line.clone(), data_to_send)).unwrap();
                                } else {
                                    lines.push(SerialData::Sent(Local::now(), format!("Command <{}> not found!", command_line.clone()), Color::LightRed));
                                }
                            } else if command_line.starts_with('!') {
                                match command_line.strip_prefix('!').unwrap() {
                                    "clear" => lines.clear(),
                                    _ => lines.push(SerialData::Sent(Local::now(), format!("Command <{command_line}> invalid"), Color::LightMagenta)),
                                }
                            } else {
                                serial_tx.send(SerialToSent::Simple(command_line.clone())).unwrap();
                            }
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
                Cmd::Sent(timestamp, mut line, sent_type) => {
                    let color = match sent_type {
                        SentType::Ok => Color::LightCyan,
                        SentType::Fail => Color::LightRed,
                        SentType::Complex(name) => {
                            line = format!("<{name}> {line}");
                            Color::LightGreen
                        }
                    };
                    lines.push(SerialData::Sent(timestamp, line, color))
                }
            }
        }

        terminal.draw(|f|
            tui_ui(f, lines.clone(), command_line.clone())
        )?;

        if lines.len() > 50 {
            lines.remove(0);
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}

#[allow(unused)]
fn fake_serial_task(serial_rx: Receiver<SerialToSent>, data_tx: Sender<Cmd>) {
    let mut counter = 0;

    loop {
        // data_tx.send(Cmd::Data(Local::now(), format!("Hello{counter}"))).unwrap();
        data_tx.send(Cmd::Data(Local::now(), "Hello".repeat(30))).unwrap();
        counter += 1;
        sleep(Duration::from_millis(100));
    }
}

fn main() {
    let (serial_tx, serial_rx) = channel();
    let (data_tx, data_rx) = channel();
    let data_tx2 = data_tx.clone();

    let yaml_content = std::fs::read(CMD_YAML_FILEPATH).unwrap();
    let yaml_cmds: BTreeMap<String, String> = serde_yaml::from_str(std::str::from_utf8(yaml_content.as_slice()).unwrap()).unwrap();

    thread::spawn(move || {
        serial_task(serial_rx, data_tx);
        // fake_serial_task(serial_rx, data_tx);
    });

    let tui_task_handler = thread::spawn(move || {
        tui_task(serial_tx, data_rx, yaml_cmds).unwrap();
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
