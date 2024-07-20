use crate::{error, info, success};
use crate::{
    infra::{
        blink::Blink,
        logger::{LogLevel, LogMessage, Logger},
        messages::TimedBytes,
        mpmc::Consumer,
        recorder::Recorder,
        task::{Shared, Task},
        typewriter::TypeWriter,
    },
    inputs::inputs_task::InputsShared,
    serial::serial_if::{SerialMode, SerialShared},
    warning,
};
use chrono::{DateTime, Local};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{block::Title, Block, BorderType, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::collections::VecDeque;
use std::{
    cmp::{max, min},
    time::Duration,
};
use std::{
    io,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
};

use super::Serialize;

pub type GraphicsTask = Task<(), GraphicsCommand>;

pub struct GraphicsConnections {
    logger: Logger,
    logger_receiver: Receiver<LogMessage>,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    inputs_shared: Shared<InputsShared>,
    serial_shared: Shared<SerialShared>,
    history: VecDeque<GraphicalMessage>,
    typewriter: TypeWriter,
    recorder: Recorder,
    capacity: usize,
    auto_scroll: bool,
    scroll: (u16, u16),
    last_frame_height: u16,
}

pub enum GraphicsCommand {
    SaveData,
    RecordData,
    ScrollLeft,
    ScrollRight,
    ScrollUp,
    ScrollDown,
    JumpToStart,
    JumpToEnd,
    PageUp,
    PageDown,
    Clear,
    Exit,
}

enum GraphicalMessage {
    Log(LogMessage),
    Tx {
        timestamp: DateTime<Local>,
        message: Vec<(String, Color)>,
    },
    Rx {
        timestamp: DateTime<Local>,
        message: Vec<(String, Color)>,
    },
}

impl GraphicsTask {
    const COMMAND_BAR_HEIGHT: u16 = 3;

    pub fn spawn_graphics_task(
        connections: GraphicsConnections,
        cmd_sender: Sender<GraphicsCommand>,
        cmd_receiver: Receiver<GraphicsCommand>,
    ) -> Self {
        Self::new((), connections, Self::task, cmd_sender, cmd_receiver)
    }

    fn draw_history(
        private: &mut GraphicsConnections,
        frame: &mut Frame,
        rect: Rect,
        blink_color: Color,
    ) {
        private.last_frame_height = frame.size().height;
        let scroll = if private.auto_scroll {
            (Self::max_main_axis(&private), private.scroll.1)
        } else {
            private.scroll
        };

        let (coll, coll_size) = (
            private.history.range(scroll.0 as usize..),
            private.history.len(),
        );
        let border_type = if private.auto_scroll {
            BorderType::Thick
        } else {
            BorderType::Double
        };

        let block = if private.recorder.is_recording() {
            Block::default()
                .title(format!(
                    "[{:03}][ASCII] ◉ {}",
                    coll_size,
                    private.recorder.get_filename()
                ))
                .title(
                    Title::from(format!("[{}]", private.recorder.get_size()))
                        .alignment(Alignment::Right),
                )
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(Color::Yellow))
        } else {
            Block::default()
                .title(format!(
                    "[{:03}][ASCII] {}",
                    coll_size,
                    private.typewriter.get_filename()
                ))
                .title(
                    Title::from(format!("[{}]", private.typewriter.get_size()))
                        .alignment(Alignment::Right),
                )
                .borders(Borders::ALL)
                .border_type(border_type)
                .border_style(Style::default().fg(blink_color))
        };

        let text = coll
            .map(|msg| match msg {
                GraphicalMessage::Log(log_msg) => Self::line_from_log_message(log_msg, scroll),
                GraphicalMessage::Tx { timestamp, message } => {
                    Self::line_from_message(timestamp, message, Color::LightCyan, scroll)
                }
                GraphicalMessage::Rx { timestamp, message } => {
                    Self::line_from_message(timestamp, message, Color::Reset, scroll)
                }
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, rect);
    }

    pub fn draw_command_bar(
        inputs_shared: &Shared<InputsShared>,
        serial_shared: &Shared<SerialShared>,
        frame: &mut Frame,
        rect: Rect,
    ) {
        let (port, baudrate, flow_control, is_connected) = {
            let serial_shared = serial_shared
                .read()
                .expect("Cannot get serial lock for read");
            (
                serial_shared.port.clone(),
                serial_shared.baudrate,
                serial_shared.flow_control,
                matches!(serial_shared.mode, SerialMode::Connected),
            )
        };
        let (text, cursor, history_len, current_hint) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (
                inputs_shared.command_line.clone(),
                inputs_shared.cursor as u16,
                inputs_shared.history_len,
                inputs_shared.current_hint.clone(),
            )
        };
        let port = if port.is_empty() {
            "\"\"".to_string()
        } else {
            port
        };

        let cursor = (rect.x + cursor + 2, rect.y + 1);
        let bar_color = if is_connected {
            Color::Green
        } else {
            Color::Red
        };

        let block = Block::default()
            .title(format!(
                "[{:03}] Serial {}:{:04}bps{}",
                history_len,
                port,
                baudrate,
                match flow_control {
                    serialport::FlowControl::None => "",
                    serialport::FlowControl::Software => ":SW",
                    serialport::FlowControl::Hardware => ":HW",
                }
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(bar_color));

        let paragraph = Paragraph::new(Span::from({
            " ".to_string()
                + if let Some(hint) = current_hint.as_ref() {
                    hint
                } else {
                    &text
                }
        }))
        .style(Style::default().fg(if current_hint.is_some() {
            Color::DarkGray
        } else {
            Color::Reset
        }))
        .block(block);

        frame.render_widget(paragraph, rect);
        frame.set_cursor(cursor.0, cursor.1);
    }

    pub fn draw_autocomplete_list(
        inputs_shared: &Shared<InputsShared>,
        frame: &mut Frame,
        command_bar_y: u16,
    ) {
        let (autocomplete_list, pattern) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");

            (
                inputs_shared.autocomplete_list.clone(),
                inputs_shared.pattern.clone(),
            )
        };

        if autocomplete_list.is_empty() {
            return;
        }

        let max_entries = min(frame.size().height as usize / 2, autocomplete_list.len());
        let mut entries = autocomplete_list[..max_entries].to_vec();
        if entries.len() < autocomplete_list.len() {
            entries.push(Arc::new("...".to_string()));
        }

        let longest_entry_len = entries
            .iter()
            .fold(0u16, |len, x| max(len, x.chars().count() as u16));
        let area_size = (longest_entry_len + 5, entries.len() as u16 + 2);
        let area = Rect::new(
            frame.size().x + 2,
            command_bar_y - area_size.1,
            area_size.0,
            area_size.1,
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .style(Style::default().fg(Color::Magenta));
        let text = entries
            .iter()
            .map(|x| {
                let is_last =
                    (x == entries.last().unwrap()) && (entries.len() < autocomplete_list.len());

                Line::from(vec![
                    Span::styled(
                        format!(" {}", if !is_last { &pattern } else { "" }),
                        Style::default().fg(Color::Magenta),
                    ),
                    Span::styled(
                        x[pattern.len() - 1..].to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    pub fn task(
        _shared: Arc<RwLock<()>>,
        mut private: GraphicsConnections,
        cmd_receiver: Receiver<GraphicsCommand>,
    ) {
        enable_raw_mode().expect("Cannot enable terminal raw mode");
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .expect("Cannot enable alternate screen and mouse capture");
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).expect("Cannot create terminal backend");
        let mut blink = Blink::new(Duration::from_millis(200), 2, Color::Reset, Color::Black);
        let mut new_messages = vec![];
        let patterns = [
            ("\x1b[0m", Color::Reset),
            ("\x1b[30m", Color::Black),
            ("\x1b[31m", Color::Red),
            ("\x1b[32m", Color::Green),
            ("\x1b[33m", Color::Yellow),
            ("\x1b[34m", Color::Blue),
            ("\x1b[35m", Color::Magenta),
            ("\x1b[36m", Color::Cyan),
            ("\x1b[37m", Color::White),
        ];

        'draw_loop: loop {
            blink.tick();

            if let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    GraphicsCommand::SaveData => {
                        if private.recorder.is_recording() {
                            warning!(private.logger, "Cannot save file while recording.");
                            continue;
                        }

                        blink.start();
                        let filename = private.typewriter.get_filename();

                        match private.typewriter.flush() {
                            Ok(_) => success!(private.logger, "Content save on \"{}\"", filename),
                            Err(err) => {
                                error!(private.logger, "Cannot save on \"{}\": {}", filename, err)
                            }
                        }
                    }
                    GraphicsCommand::RecordData => {
                        let filename = private.recorder.get_filename();

                        if private.recorder.is_recording() {
                            private.recorder.stop_record();
                            success!(private.logger, "Content recoded on \"{}\"", filename);
                        } else {
                            match private.recorder.start_record() {
                                Ok(_) => info!(
                                    private.logger,
                                    "Recording content on \"{}\"...", filename
                                ),
                                Err(err) => error!(
                                    private.logger,
                                    "Cannot start record the content on \"{}\": {}", filename, err
                                ),
                            }
                        }
                    }
                    GraphicsCommand::Clear => {
                        private.auto_scroll = true;
                        private.scroll = (0, 0);
                        private.history.clear();
                    }
                    GraphicsCommand::ScrollLeft => {
                        if private.scroll.1 < 3 {
                            private.scroll.1 = 0;
                        } else {
                            private.scroll.1 -= 3;
                        }
                    }
                    GraphicsCommand::ScrollRight => private.scroll.1 += 3,
                    GraphicsCommand::ScrollUp => {
                        if Self::max_main_axis(&private) > 0 {
                            private.auto_scroll = false;
                        }

                        if private.scroll.0 < 3 {
                            private.scroll.0 = 0;
                        } else {
                            private.scroll.0 -= 3;
                        }
                    }
                    GraphicsCommand::ScrollDown => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.scroll.0 += 3;
                        private.scroll.0 = private.scroll.0.clamp(0, max_main_axis);

                        if private.scroll.0 == max_main_axis {
                            private.auto_scroll = true;
                        }
                    }
                    GraphicsCommand::JumpToStart => {
                        if Self::max_main_axis(&private) > 0 {
                            private.auto_scroll = false;
                        }

                        private.scroll.0 = 0;
                    }
                    GraphicsCommand::JumpToEnd => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.scroll.0 = max_main_axis;
                        private.auto_scroll = true;
                    }
                    GraphicsCommand::PageUp => {
                        if Self::max_main_axis(&private) > 0 {
                            private.auto_scroll = false;
                        }

                        let page_height = private.last_frame_height - 5;

                        if private.scroll.0 < page_height {
                            private.scroll.0 = 0;
                        } else {
                            private.scroll.0 -= page_height;
                        }
                    }
                    GraphicsCommand::PageDown => {
                        if Self::max_main_axis(&private) > 0 {
                            private.auto_scroll = false;
                        }

                        private.scroll.0 = 0;
                    }
                    GraphicsCommand::Exit => break 'draw_loop,
                }
            }

            while let Ok(rx_msg) = private.rx.try_recv() {
                if private.history.len() + new_messages.len() >= private.capacity {
                    private.history.remove(0);
                }

                new_messages.push(GraphicalMessage::Rx {
                    timestamp: rx_msg.timestamp,
                    message: Self::ansi_colors(&patterns, &rx_msg.message),
                });
            }

            while let Ok(tx_msg) = private.tx.try_recv() {
                if private.history.len() + new_messages.len() >= private.capacity {
                    private.history.remove(0);
                }

                new_messages.push(GraphicalMessage::Tx {
                    timestamp: tx_msg.timestamp,
                    message: Self::bytes_to_string(&tx_msg.message, Color::Black),
                });
            }

            while let Ok(log_msg) = private.logger_receiver.try_recv() {
                if private.history.len() + new_messages.len() >= private.capacity {
                    private.history.remove(0);
                }

                new_messages.push(GraphicalMessage::Log(log_msg));
            }

            if !new_messages.is_empty() {
                new_messages
                    .sort_by(|a, b| a.get_timestamp().partial_cmp(b.get_timestamp()).unwrap());
                if private.recorder.is_recording() {
                    if let Err(err) = private
                        .recorder
                        .add_bulk_content(new_messages.iter().map(|gm| gm.serialize()).collect())
                    {
                        error!(private.logger, "{}", err);
                    }
                }
                private.typewriter += new_messages.iter().map(|gm| gm.serialize()).collect();
                private.history.extend(new_messages.into_iter());
                new_messages = vec![];
            }

            terminal
                .draw(|f| {
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(f.size().height - Self::COMMAND_BAR_HEIGHT),
                            Constraint::Length(Self::COMMAND_BAR_HEIGHT),
                        ])
                        .split(f.size());

                    Self::draw_history(&mut private, f, chunks[0], blink.get_current());
                    Self::draw_command_bar(
                        &private.inputs_shared,
                        &private.serial_shared,
                        f,
                        chunks[1],
                    );
                    Self::draw_autocomplete_list(&private.inputs_shared, f, chunks[1].y);
                })
                .expect("Error to draw");
        }

        disable_raw_mode().expect("Cannot disable terminal raw mode");
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .expect("Cannot disable alternate screen and mouse capture");
        terminal.show_cursor().expect("Cannot show mouse cursor");
    }

    fn ansi_colors(patterns: &[(&'static str, Color)], msg: &[u8]) -> Vec<(String, Color)> {
        let mut output = vec![];
        let mut buffer = "".to_string();
        let mut color = Color::Reset;

        for byte in msg {
            buffer.push(*byte as char);

            if (*byte as char) != 'm' {
                continue;
            }

            'pattern_loop: for (pattern, new_color) in patterns {
                if buffer.contains(pattern) {
                    output.push((buffer.replace(pattern, ""), color));

                    buffer.clear();
                    color = *new_color;

                    break 'pattern_loop;
                }
            }
        }

        if !buffer.is_empty() {
            output.push((buffer, color));
        }

        output
            .into_iter()
            .flat_map(|(msg, color)| Self::bytes_to_string(msg.as_bytes(), color))
            .collect()
    }

    fn bytes_to_string(msg: &[u8], color: Color) -> Vec<(String, Color)> {
        let mut output = vec![];
        let mut buffer = "".to_string();
        let mut in_plain_text = true;
        let accent_color = if color == Color::Magenta {
            Color::DarkGray
        } else {
            Color::Magenta
        };

        for byte in msg {
            match *byte {
                x if 0x20 <= x && x <= 0x7E => {
                    if !in_plain_text {
                        output.push((buffer.drain(..).collect(), accent_color));
                        in_plain_text = true;
                    }

                    buffer.push(x as char);
                }
                x => {
                    if in_plain_text {
                        output.push((buffer.drain(..).collect(), color));
                        in_plain_text = false;
                    }

                    match x {
                        0x0a => buffer += "\\n",
                        0x0d => buffer += "\\r",
                        _ => buffer += &format!("\\x{:02x}", byte),
                    }
                }
            }
        }

        if !buffer.is_empty() {
            output.push((buffer, if in_plain_text { color } else { accent_color }));
        }

        output
    }

    fn timestamp_span(timestamp: &DateTime<Local>) -> Span {
        Span::styled(
            format!("{} ", timestamp.format("%H:%M:%S.%3f")),
            Style::default().fg(Color::DarkGray),
        )
    }

    fn line_from_log_message(
        LogMessage {
            timestamp,
            message,
            level,
        }: &LogMessage,
        (_scroll_y, scroll_x): (u16, u16),
    ) -> Line {
        let scroll_x = scroll_x as usize;
        let mut spans = vec![Self::timestamp_span(timestamp)];
        let (bg, fg) = match level {
            crate::infra::LogLevel::Error => (Color::Red, Color::White),
            crate::infra::LogLevel::Warning => (Color::Yellow, Color::Black),
            crate::infra::LogLevel::Success => (Color::LightGreen, Color::Black),
            crate::infra::LogLevel::Info => (Color::White, Color::Black),
            crate::infra::LogLevel::Debug => (Color::Reset, Color::Black),
        };

        if scroll_x < message.chars().count() {
            spans.push(Span::styled(
                &message[scroll_x..],
                Style::default().fg(fg).bg(bg),
            ));
        }

        Line::from(spans)
    }

    fn line_from_message<'a>(
        timestamp: &'a DateTime<Local>,
        message: &'a [(String, Color)],
        bg: Color,
        (_scroll_y, scroll_x): (u16, u16),
    ) -> Line<'a> {
        let scroll_x = scroll_x as usize;
        let mut offset = 0;
        let mut spans = vec![Self::timestamp_span(&timestamp)];

        for (msg, fg) in message {
            if scroll_x >= msg.len() + offset {
                offset += msg.len();
                continue;
            }

            let cropped_message = if scroll_x < (msg.len() + offset) && scroll_x >= offset {
                &msg[(scroll_x - offset)..]
            } else {
                &msg
            };
            offset += msg.len();

            spans.push(Span::styled(
                cropped_message,
                Style::default().fg(*fg).bg(bg),
            ));
        }

        Line::from(spans)
    }

    fn max_main_axis(private: &GraphicsConnections) -> u16 {
        let main_axis_length = private.last_frame_height - Self::COMMAND_BAR_HEIGHT - 2;
        let history_len = private.history.len() as u16;

        if history_len > main_axis_length {
            history_len - main_axis_length
        } else {
            0
        }
    }
}

impl GraphicsConnections {
    pub fn new(
        logger: Logger,
        logger_receiver: Receiver<LogMessage>,
        tx: Consumer<Arc<TimedBytes>>,
        rx: Consumer<Arc<TimedBytes>>,
        inputs_shared: Shared<InputsShared>,
        serial_shared: Shared<SerialShared>,
        storage_base_filename: String,
        capacity: usize,
    ) -> Self {
        Self {
            logger,
            logger_receiver,
            tx,
            rx,
            inputs_shared,
            serial_shared,
            history: VecDeque::new(),
            typewriter: TypeWriter::new(storage_base_filename.clone()),
            recorder: Recorder::new(storage_base_filename).expect("Cannot create Recorder"),
            capacity,
            auto_scroll: true,
            scroll: (0, 0),
            last_frame_height: u16::MAX,
        }
    }
}

impl Serialize for GraphicalMessage {
    fn serialize(&self) -> String {
        match self {
            GraphicalMessage::Log(log) => {
                let log_level = match log.level {
                    LogLevel::Error => "ERR",
                    LogLevel::Warning => "WRN",
                    LogLevel::Success => " OK",
                    LogLevel::Info => "INF",
                    LogLevel::Debug => "DBG",
                };

                format!(
                    "[{}][{}] {}",
                    log.timestamp.format("%H:%M:%S.%3f"),
                    log_level,
                    log.message
                )
            }
            GraphicalMessage::Tx { timestamp, message } => {
                let msg = message.iter().fold(String::new(), |acc, x| acc + &x.0);

                format!("[{}][ =>] {}", timestamp.format("%H:%M:%S.%3f"), msg)
            }
            GraphicalMessage::Rx { timestamp, message } => {
                let msg = message.iter().fold(String::new(), |acc, x| acc + &x.0);
                format!("[{}][ <=] {}", timestamp.format("%H:%M:%S.%3f"), msg)
            }
        }
    }
}

impl GraphicalMessage {
    pub fn get_timestamp(&self) -> &DateTime<Local> {
        match self {
            GraphicalMessage::Log(LogMessage { timestamp, .. }) => timestamp,
            GraphicalMessage::Tx { timestamp, .. } => timestamp,
            GraphicalMessage::Rx { timestamp, .. } => timestamp,
        }
    }
}
