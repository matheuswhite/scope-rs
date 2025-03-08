use super::bytes::SliceExt;
use super::Serialize;
use crate::{error, info, inputs, success};
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
use std::thread::{sleep, yield_now};
use std::u16;
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

pub type GraphicsTask = Task<(), GraphicsCommand>;

pub struct GraphicsConnections {
    logger: Logger,
    logger_receiver: Receiver<LogMessage>,
    system_log_level: LogLevel,
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
    last_frame_size: Rect,
    is_true_color: bool,
    latency: u64,
    search_state: SearchState,
}

pub enum GraphicsCommand {
    SetLogLevel(LogLevel),
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
    NextSearch,
    PrevSearch,
    SearchChange,
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

#[derive(Default)]
struct SearchEntry {
    line: usize,
    column: usize,
}

struct SearchDrawEntry {
    column: usize,
    is_active: bool,
}

#[derive(Default)]
pub struct SearchState {
    entries: Vec<SearchEntry>,
    current: usize,
    total: usize,
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

    fn draw_history_normal_mode(
        private: &mut GraphicsConnections,
        frame: &mut Frame,
        rect: Rect,
        blink_color: Color,
    ) {
        private.last_frame_size = frame.size();
        if private.auto_scroll {
            private.scroll.0 = Self::max_main_axis(&private);
        }
        let scroll = private.scroll;

        let (coll, coll_size) = (
            private.history.range(scroll.0 as usize..).filter(|msg| {
                if let GraphicalMessage::Log(log) = msg {
                    log.level as u32 <= private.system_log_level as u32
                } else {
                    true
                }
            }),
            Self::history_length(private),
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
                GraphicalMessage::Tx { timestamp, message } => Self::line_from_message(
                    timestamp,
                    message,
                    if private.is_true_color {
                        Color::Rgb(12, 129, 123)
                    } else {
                        Color::Blue
                    },
                    scroll,
                ),
                GraphicalMessage::Rx { timestamp, message } => {
                    Self::line_from_message(timestamp, message, Color::Reset, scroll)
                }
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, rect);
    }

    fn draw_history_search_mode(
        private: &mut GraphicsConnections,
        frame: &mut Frame,
        rect: Rect,
        blink_color: Color,
        pattern: &str,
    ) {
        private.last_frame_size = frame.size();
        private.auto_scroll = false;
        let scroll = private.scroll;

        let (coll, coll_size) = (
            private.history.range(scroll.0 as usize..).filter(|msg| {
                if let GraphicalMessage::Log(log) = msg {
                    log.level as u32 <= private.system_log_level as u32
                } else {
                    true
                }
            }),
            Self::history_length(private),
        );
        let border_type = BorderType::Double;

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

        let current_active = private.search_state.current;
        let text = coll
            .enumerate()
            .map(|(current_line, msg)| match msg {
                GraphicalMessage::Log(log_msg) => Self::line_for_search_mode(
                    &log_msg.timestamp,
                    log_msg.message.to_owned(),
                    scroll,
                    vec![],
                    pattern,
                ),
                GraphicalMessage::Tx { timestamp, message } => Self::line_for_search_mode(
                    timestamp,
                    message
                        .iter()
                        .map(|(msg, _)| msg.to_owned())
                        .collect::<Vec<_>>()
                        .join(""),
                    scroll,
                    vec![],
                    pattern,
                ),
                GraphicalMessage::Rx { timestamp, message } => Self::line_for_search_mode(
                    timestamp,
                    message
                        .iter()
                        .map(|(msg, _)| msg.to_owned())
                        .collect::<Vec<_>>()
                        .join(""),
                    scroll,
                    private
                        .search_state
                        .entries
                        .iter()
                        .enumerate()
                        .filter_map(|(i, SearchEntry { line, column })| {
                            if current_line == *line {
                                Some(SearchDrawEntry {
                                    column: *column,
                                    is_active: i == current_active,
                                })
                            } else {
                                None
                            }
                        })
                        .collect(),
                    pattern,
                ),
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        frame.render_widget(paragraph, rect);
    }

    fn draw_history(
        private: &mut GraphicsConnections,
        frame: &mut Frame,
        rect: Rect,
        blink_color: Color,
    ) {
        let (input_mode, pattern) = {
            let inputs_shared = private
                .inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (inputs_shared.mode, inputs_shared.search_buffer.clone())
        };

        match input_mode {
            inputs::inputs_task::InputMode::Normal => {
                Self::draw_history_normal_mode(private, frame, rect, blink_color)
            }
            inputs::inputs_task::InputMode::Search => {
                Self::draw_history_search_mode(private, frame, rect, blink_color, &pattern)
            }
        }
    }

    pub fn draw_command_bar_normal_mode(
        inputs_shared: &Shared<InputsShared>,
        serial_shared: &Shared<SerialShared>,
        frame: &mut Frame,
        rect: Rect,
        latency: u64,
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

        let latency = if latency >= 1_000 {
            format!("{}ms", latency / 1_000)
        } else if latency > 0 {
            format!("{}us", latency)
        } else {
            "---".to_string()
        };

        let block = Block::default()
            .title(format!(
                "[{:03}][{}] Serial {}:{:04}bps{}",
                history_len,
                latency,
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

    fn draw_command_bar_search_mode(
        inputs_shared: &Shared<InputsShared>,
        search_state: &SearchState,
        rect: Rect,
        frame: &mut Frame,
        is_case_sensitive: bool,
    ) {
        let (text, cursor) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (
                inputs_shared.search_buffer.clone(),
                inputs_shared.search_cursor as u16,
            )
        };

        let cursor = (rect.x + cursor + 2, rect.y + 1);

        let current = if search_state.total > 0 {
            format!("{}", search_state.current + 1)
        } else {
            "--".to_string()
        };
        let total = if search_state.total > 0 {
            format!("{}", search_state.total)
        } else {
            "--".to_string()
        };

        let block = Block::default()
            .title(format!(
                "[{}][{}/{}] Search Mode",
                if is_case_sensitive { "Aa" } else { "--" },
                current,
                total
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(Color::Yellow));

        let paragraph = Paragraph::new(Span::from(" ".to_string() + &text))
            .style(Style::default().fg(if search_state.total == 0 {
                Color::Red
            } else {
                Color::Reset
            }))
            .block(block);

        frame.render_widget(paragraph, rect);
        frame.set_cursor(cursor.0, cursor.1);
    }

    pub fn draw_command_bar(
        inputs_shared: &Shared<InputsShared>,
        serial_shared: &Shared<SerialShared>,
        frame: &mut Frame,
        rect: Rect,
        latency: u64,
        search_state: &SearchState,
    ) {
        let (input_mode, is_case_sensitive) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (inputs_shared.mode, inputs_shared.is_case_sensitive)
        };

        match input_mode {
            inputs::inputs_task::InputMode::Normal => Self::draw_command_bar_normal_mode(
                inputs_shared,
                serial_shared,
                frame,
                rect,
                latency,
            ),
            inputs::inputs_task::InputMode::Search => Self::draw_command_bar_search_mode(
                inputs_shared,
                search_state,
                rect,
                frame,
                is_case_sensitive,
            ),
        }
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
            (b"\x1b[0m".as_slice(), Color::Reset),
            (b"\x1b[30m", Color::Black),
            (b"\x1b[32m", Color::Green),
            (b"\x1b[1;32m", Color::Green),
            (b"\x1b[31m", Color::Red),
            (b"\x1b[1;31m", Color::Red),
            (b"\x1b[33m", Color::Yellow),
            (b"\x1b[1;33m", Color::Yellow),
            (b"\x1b[34m", Color::Blue),
            (b"\x1b[35m", Color::Magenta),
            (b"\x1b[36m", Color::Cyan),
            (b"\x1b[37m", Color::White),
        ];

        'draw_loop: loop {
            blink.tick();

            if let Ok(cmd) = cmd_receiver.try_recv() {
                match cmd {
                    GraphicsCommand::SetLogLevel(level) => {
                        private.system_log_level = level;
                        success!(private.logger, "Log setted to {:?}", level);
                    }
                    GraphicsCommand::SaveData => {
                        if private.recorder.is_recording() {
                            warning!(private.logger, "Cannot save file while recording.");
                            /* don't yield here, because we need to put this warning message on display */
                            continue 'draw_loop;
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

                        let page_height = private.last_frame_size.height - 5;

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
                    GraphicsCommand::NextSearch => {
                        if private.search_state.total > 0 {
                            private.search_state.current += 1;
                            private.search_state.current %= private.search_state.total;
                            private.scroll.0 = private.search_state.entries
                                [private.search_state.current]
                                .line as u16;
                            private.scroll.1 = private.search_state.entries
                                [private.search_state.current]
                                .column as u16;

                            private.scroll.1 = private
                                .scroll
                                .1
                                .saturating_sub(private.last_frame_size.width / 2);
                            private.scroll.0 = private
                                .scroll
                                .0
                                .saturating_sub(private.last_frame_size.height / 2);
                        }
                    }
                    GraphicsCommand::PrevSearch => {
                        if private.search_state.total > 0 {
                            let new_current = private.search_state.current as isize - 1;
                            if new_current < 0 {
                                private.search_state.current = private.search_state.total - 1;
                            } else {
                                private.search_state.current = new_current as usize;
                            }

                            private.scroll.0 = private.search_state.entries
                                [private.search_state.current]
                                .line as u16;
                            private.scroll.1 = private.search_state.entries
                                [private.search_state.current]
                                .column as u16;

                            private.scroll.1 = private
                                .scroll
                                .1
                                .saturating_sub(private.last_frame_size.width / 2);
                            private.scroll.0 = private
                                .scroll
                                .0
                                .saturating_sub(private.last_frame_size.height / 2);
                        }
                    }
                    GraphicsCommand::SearchChange => {
                        let (search_buffer, is_case_sensitive) = {
                            let input_sr = private
                                .inputs_shared
                                .read()
                                .expect("Cannot get input lock for read");
                            (input_sr.search_buffer.clone(), input_sr.is_case_sensitive)
                        };

                        Self::update_search_state(
                            &mut private.search_state,
                            &private.history,
                            search_buffer,
                            is_case_sensitive,
                        );
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
                    message: Self::bytes_to_string(&tx_msg.message, Color::White),
                });
            }

            while let Ok(LogMessage {
                timestamp,
                message,
                level,
            }) = private.logger_receiver.try_recv()
            {
                if private.history.len() + new_messages.len() >= private.capacity {
                    private.history.remove(0);
                }

                let message = message.split("\n").collect::<Vec<_>>();
                let message_len = message.len();
                let log_msg_splited = message
                    .into_iter()
                    .filter(|msg| !msg.is_empty())
                    .enumerate()
                    .map(|(i, msg)| {
                        GraphicalMessage::Log(LogMessage {
                            timestamp,
                            message: if i == 0 {
                                msg.to_owned()
                            } else {
                                "  ".to_string() + msg
                            }
                            .replace('\r', "")
                            .replace('\t', "    ")
                                + if i < (message_len - 1) { "\r\n" } else { "" },
                            level,
                        })
                    })
                    .collect::<Vec<_>>();

                new_messages.extend(log_msg_splited);
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
                        private.latency,
                        &private.search_state,
                    );
                    Self::draw_autocomplete_list(&private.inputs_shared, f, chunks[1].y);
                })
                .expect("Error to draw");

            if private.latency > 0 {
                sleep(Duration::from_micros(private.latency));
            } else {
                yield_now();
            }
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

    fn update_search_state(
        search_state: &mut SearchState,
        history: &VecDeque<GraphicalMessage>,
        pattern: String,
        is_case_sensitive: bool,
    ) {
        if pattern.is_empty() {
            *search_state = SearchState::default();
            return;
        }

        let mut entries = vec![];

        let pattern = if !is_case_sensitive {
            pattern.to_lowercase()
        } else {
            pattern
        };

        for (line, message) in history.iter().enumerate() {
            let GraphicalMessage::Rx { message, .. } = message else {
                continue;
            };

            let message = message
                .iter()
                .map(|(msg, _)| msg.to_owned())
                .collect::<Vec<_>>()
                .join("");

            let mut message = if !is_case_sensitive {
                message.to_lowercase()
            } else {
                message
            };

            while let Some(column) = message.find(&pattern) {
                entries.push(SearchEntry { line, column });
                let _: String = message.drain(..column + pattern.len()).collect();
            }
        }

        search_state.current = 0;
        search_state.total = entries.len();
        search_state.entries = entries;
    }

    fn ansi_colors(patterns: &[(&[u8], Color)], msg: &[u8]) -> Vec<(String, Color)> {
        let msg = msg
            .to_vec()
            .replace(b"\x1b[m", b"")
            .replace(b"\x1b[8D", b"")
            .replace(b"\x1b[J", b"");
        let mut output = vec![];
        let mut buffer = vec![];
        let mut color = Color::Reset;

        for byte in msg {
            buffer.push(byte);

            if (byte as char) != 'm' {
                continue;
            }

            'pattern_loop: for (pattern, new_color) in patterns {
                if buffer.contains(pattern) {
                    output.push((buffer.replace(pattern, b""), color));

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
            .flat_map(|(msg, color)| Self::bytes_to_string(&msg, color))
            .collect()
    }

    fn bytes_to_string(msg: &[u8], color: Color) -> Vec<(String, Color)> {
        let mut output = vec![];
        let mut buffer = "".to_string();
        let mut in_plain_text = true;
        let accent_color = if color == Color::Yellow {
            Color::DarkGray
        } else {
            Color::Yellow
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
            crate::infra::LogLevel::Debug => (Color::Reset, Color::DarkGray),
        };

        let end_index = if message.ends_with("\r\n") {
            message.len() - 2
        } else {
            message.len()
        };

        if scroll_x < message.chars().count() {
            spans.push(Span::styled(
                &message[scroll_x..end_index],
                Style::default().fg(fg).bg(bg),
            ));

            if message.ends_with("\r\n") {
                spans.push(Span::styled(
                    "\\r\\n",
                    Style::default().fg(Color::Yellow).bg(bg),
                ));
            }
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

    fn line_for_search_mode<'a>(
        timestamp: &'a DateTime<Local>,
        mut message: String,
        (_scroll_y, scroll_x): (u16, u16),
        filtered_search_entries: Vec<SearchDrawEntry>,
        pattern: &str,
    ) -> Line<'a> {
        let scroll_x = scroll_x as usize;
        let mut spans = vec![Self::timestamp_span(&timestamp)];
        let pattern_len = pattern.len();

        let mut message_and_color = vec![];
        for SearchDrawEntry { column, is_active } in filtered_search_entries {
            if column >= message.len() {
                message_and_color.push(("#".to_string(), Color::White, Color::Red));
            } else {
                message_and_color.push((
                    message.drain(..column).collect::<String>(),
                    Color::DarkGray,
                    Color::Reset,
                ));
            }
            message_and_color.push((
                message.drain(..pattern_len).collect(),
                if is_active {
                    Color::Black
                } else {
                    Color::Yellow
                },
                if is_active {
                    Color::Yellow
                } else {
                    Color::Reset
                },
            ));
        }

        if !message.is_empty() {
            message_and_color.push((message, Color::DarkGray, Color::Reset));
        }

        let mut offset = 0;
        for (msg, fg, bg) in message_and_color {
            if scroll_x >= msg.len() + offset {
                offset += msg.len();
                continue;
            }

            let msg_len = msg.len();
            let cropped_message = if scroll_x < (msg.len() + offset) && scroll_x >= offset {
                msg[(scroll_x - offset)..].to_owned()
            } else {
                msg
            };
            offset += msg_len;

            spans.push(Span::styled(
                cropped_message,
                Style::default().fg(fg).bg(bg),
            ));
        }

        Line::from(spans)
    }

    fn history_length(private: &GraphicsConnections) -> usize {
        private
            .history
            .iter()
            .filter(|msg| {
                if let GraphicalMessage::Log(log) = msg {
                    log.level as u32 <= private.system_log_level as u32
                } else {
                    true
                }
            })
            .count()
    }

    fn max_main_axis(private: &GraphicsConnections) -> u16 {
        let main_axis_length = private.last_frame_size.height - Self::COMMAND_BAR_HEIGHT - 2;
        let history_len = Self::history_length(private) as u16;

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
        is_true_color: bool,
        latency: u64,
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
            last_frame_size: Rect::new(0, 0, u16::MAX, u16::MAX),
            system_log_level: LogLevel::Debug,
            is_true_color,
            latency,
            search_state: SearchState {
                entries: vec![],
                current: 0,
                total: 0,
            },
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
