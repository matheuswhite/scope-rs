use crate::{
    infra::{
        logger::{LogLevel, LogMessage, Logger},
        messages::TimedBytes,
        mpmc::Consumer,
        recorder::Recorder,
        task::{Shared, Task},
        timer::Timer,
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
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{block::Title, Block, BorderType, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::{
    cmp::{max, min},
    time::Duration,
};
use std::{
    fs::File,
    io,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
};

use super::rich_string::RichText;

pub type GraphicsTask = Task<(), GraphicsCommand>;

pub struct GraphicsConnections {
    logger: Logger,
    logger_receiver: Receiver<LogMessage>,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    inputs_shared: Shared<InputsShared>,
    serial_shared: Shared<SerialShared>,
    history: Vec<GraphicalMessage>,
    typewriter: TypeWriter,
    recorder: Recorder,
    capacity: usize,
    auto_scroll: bool,
    scroll: (u16, u16),
}

pub enum GraphicsCommand {
    SaveData,
    RecordData,
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
        private: &GraphicsConnections,
        frame: &mut Frame,
        rect: Rect,
        blink_color: Option<Color>,
    ) {
        let scroll = if private.auto_scroll {
            (
                Self::max_main_axis(frame.size().height, &private.history),
                private.scroll.1,
            )
        } else {
            private.scroll
        };

        let (coll, coll_size) = (
            &private.history[(scroll.0 as usize)..],
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
                .border_style(Style::default().fg(blink_color.unwrap_or(Color::Reset)))
        };

        let text = coll
            .iter()
            .map(|msg| match msg {
                GraphicalMessage::Log(log_msg) => Self::line_from_log_message(log_msg, scroll),
                GraphicalMessage::Tx { timestamp, message } => {
                    Self::line_from_message(timestamp, message, Color::Cyan, scroll)
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
        let (port, baudrate, is_connected) = {
            let serial_shared = serial_shared
                .read()
                .expect("Cannot get serial lock for read");
            (
                serial_shared.port.clone(),
                serial_shared.baudrate,
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

        let cursor = (rect.x + cursor + 2, rect.y + 1);
        let bar_color = if is_connected {
            Color::Green
        } else {
            Color::Red
        };

        let block = Block::default()
            .title(format!(
                "[{:03}] Serial {}:{}bps",
                history_len, port, baudrate
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
        shared: Arc<RwLock<()>>,
        mut private: GraphicsConnections,
        cmd_receiver: Receiver<GraphicsCommand>,
    ) {
        enable_raw_mode().expect("Cannot enable terminal raw mode");
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .expect("Cannot enable alternate screen and mouse capture");
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).expect("Cannot create terminal backend");

        let mut blink_color = None;
        let mut blink_amounts = 0;
        let mut blink_max_amounts = 2;
        let mut blink_timer_on = Timer::new(Duration::from_millis(200), || {});
        let mut blink_timer_off = Timer::new(Duration::from_millis(200), || {});

        'draw_loop: loop {
            blink_timer_on.tick();
            blink_timer_off.tick();

            if let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    GraphicsCommand::SaveData => {
                        if private.recorder.is_recording() {
                            warning!(private.logger, "Cannot save file while recording.");
                            continue;
                        }

                        blink_timer_on.start();
                    }
                    GraphicsCommand::RecordData => {
                        if private.recorder.is_recording() {
                            private.recorder.stop_record();
                        } else {
                            private.recorder.start_record();
                        }
                    }
                    GraphicsCommand::Exit => break 'draw_loop,
                }
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

                    Self::draw_history(&private, f, chunks[0], blink_color);
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

    fn timestamp_span(timestamp: &DateTime<Local>) -> Span {
        Span::styled(
            format!("{}", timestamp.format("%H:%M:%S.%3f")),
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
            crate::infra::LogLevel::Warning => (Color::Yellow, Color::DarkGray),
            crate::infra::LogLevel::Success => (Color::LightGreen, Color::DarkGray),
            crate::infra::LogLevel::Info => (Color::White, Color::DarkGray),
            crate::infra::LogLevel::Debug => (Color::Reset, Color::DarkGray),
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

            let cropped_message = if scroll_x < (msg.len() + offset) {
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

    fn max_main_axis(frame_height: u16, history: &Vec<GraphicalMessage>) -> u16 {
        let main_axis_length = frame_height - Self::COMMAND_BAR_HEIGHT - 2;
        let history_len = history.len() as u16;

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
            history: vec![],
            typewriter: TypeWriter::new(storage_base_filename.clone()),
            recorder: Recorder::new(storage_base_filename).expect("Cannot create Recorder"),
            capacity,
            auto_scroll: true,
            scroll: (0, 0),
        }
    }
}
