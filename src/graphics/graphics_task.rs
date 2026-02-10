use super::Serialize;
use crate::graphics::ansi::ANSI;
use crate::graphics::buffer::{Buffer, BufferLine, BufferPosition};
use crate::graphics::screen::{Screen, ScreenPosition};
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
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};
use std::thread::{sleep, yield_now};
use std::{
    cmp::{max, min},
    time::Duration,
};
use std::{
    io,
    sync::{
        Arc, RwLock,
        mpsc::{Receiver, Sender},
    },
};

pub type GraphicsTask = Task<(), GraphicsCommand>;

pub struct GraphicsConfig {
    pub storage_base_filename: String,
    pub capacity: usize,
    pub latency: u64,
}

pub struct GraphicsConnections {
    logger: Logger,
    logger_receiver: Receiver<LogMessage>,
    system_log_level: LogLevel,
    tx: Consumer<Arc<TimedBytes>>,
    rx: Consumer<Arc<TimedBytes>>,
    inputs_shared: Shared<InputsShared>,
    serial_shared: Shared<SerialShared>,
    typewriter: TypeWriter,
    recorder: Recorder,
    latency: u64,
    buffer: Buffer,
    screen: Screen,
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
    ChangeToNormalMode,
    ChangeToSearchMode,
    Exit,
    Redraw,
    Click(ScreenPosition),
    Move(ScreenPosition),
}

pub struct SaveStats {
    file_size: u128,
    is_recording: bool,
    filename: String,
    is_saving: bool,
    save_color: Color,
}

impl SaveStats {
    pub fn new(file_size: u128, filename: String, save_color: Color) -> Self {
        Self {
            file_size,
            is_recording: false,
            filename,
            is_saving: false,
            save_color,
        }
    }

    pub fn file_size(&self) -> u128 {
        self.file_size
    }

    pub fn is_recording(&self) -> bool {
        self.is_recording
    }

    pub fn is_saving(&self) -> bool {
        self.is_saving
    }

    pub fn save_color(&self) -> Color {
        self.save_color
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn convert_to_typewriter(self, typewriter: &TypeWriter) -> Self {
        Self {
            file_size: typewriter.get_size(),
            filename: typewriter.get_filename(),
            is_recording: false,
            ..self
        }
    }

    pub fn convert_to_recorder(self, recorder: &Recorder) -> Self {
        Self {
            file_size: recorder.get_size(),
            filename: recorder.get_filename(),
            is_recording: true,
            ..self
        }
    }
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
        (search_current, search_total): (usize, usize),
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

        let current = if search_total > 0 {
            format!("{}", search_current + 1)
        } else {
            "--".to_string()
        };
        let total = if search_total > 0 {
            format!("{}", search_total)
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
            .style(Style::default().fg(if search_total == 0 {
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
        search_indexes: Option<(usize, usize)>,
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
                search_indexes.unwrap_or((0, 0)),
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
        let mut need_redraw = true;
        let mut save_stats = SaveStats::new(
            private.typewriter.get_size(),
            private.typewriter.get_filename(),
            blink.get_current(),
        );

        'draw_loop: loop {
            blink.tick();

            save_stats.is_recording = private.recorder.is_recording();

            if blink.is_active() {
                need_redraw = true;
                save_stats.is_saving = true;
                save_stats.save_color = blink.get_current();
            } else {
                save_stats.is_saving = false;
                save_stats.save_color = Color::Reset;
            }

            while let Ok(cmd) = cmd_receiver.try_recv() {
                need_redraw = true;

                match cmd {
                    GraphicsCommand::Redraw => { /* just to trigger a redraw */ }
                    GraphicsCommand::Click(start_pos) => {
                        private.screen.set_selection(start_pos);
                    }
                    GraphicsCommand::Move(end_pos) => {
                        private.screen.set_selection_end(end_pos);
                    }
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
                            success!(private.logger, "Content recorded to \"{}\"", filename);
                            save_stats = save_stats.convert_to_typewriter(&private.typewriter);
                        } else {
                            match private.recorder.start_record() {
                                Ok(_) => {
                                    info!(
                                        private.logger,
                                        "Recording content on \"{}\"...", filename
                                    );
                                    save_stats = save_stats.convert_to_recorder(&private.recorder);
                                }
                                Err(err) => error!(
                                    private.logger,
                                    "Cannot start record the content on \"{}\": {}", filename, err
                                ),
                            }
                        }
                    }
                    GraphicsCommand::Clear => {
                        private.screen.clear();
                        private.buffer.clear();
                    }
                    GraphicsCommand::ScrollLeft => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.scroll_horizontal(-3, max_main_axis as usize);
                    }
                    GraphicsCommand::ScrollRight => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.scroll_horizontal(3, max_main_axis as usize);
                    }
                    GraphicsCommand::ScrollUp => {
                        let max_main_axis = Self::max_main_axis(&private);
                        if max_main_axis > 0 {
                            private.screen.disable_auto_scroll();
                        }

                        private.screen.scroll_vertical(-3, max_main_axis as usize);
                    }
                    GraphicsCommand::ScrollDown => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.scroll_vertical(3, max_main_axis as usize);
                    }
                    GraphicsCommand::JumpToStart => {
                        if Self::max_main_axis(&private) > 0 {
                            private.screen.disable_auto_scroll();
                        }

                        private.screen.jump_to_start();
                    }
                    GraphicsCommand::JumpToEnd => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.jump_to_end(max_main_axis as usize);
                    }
                    GraphicsCommand::PageUp => {
                        let max_main_axis = Self::max_main_axis(&private);
                        if max_main_axis > 0 {
                            private.screen.disable_auto_scroll();
                        }

                        let page_height = private.screen.size().height.saturating_sub(5) as isize;

                        private
                            .screen
                            .scroll_vertical(-page_height, max_main_axis as usize);
                    }
                    GraphicsCommand::PageDown => {
                        let max_main_axis = Self::max_main_axis(&private);
                        if max_main_axis > 0 {
                            private.screen.disable_auto_scroll();
                        }

                        let page_height = private.screen.size().height.saturating_sub(5) as isize;

                        private
                            .screen
                            .scroll_vertical(page_height, max_main_axis as usize);
                    }
                    GraphicsCommand::NextSearch => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.jump_to_next_search(max_main_axis as usize);
                    }
                    GraphicsCommand::PrevSearch => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private
                            .screen
                            .jump_to_previous_search(max_main_axis as usize);
                    }
                    GraphicsCommand::SearchChange => {
                        let (search_buffer, is_case_sensitive) = {
                            let input_sr = private
                                .inputs_shared
                                .read()
                                .expect("Cannot get input lock for read");
                            (input_sr.search_buffer.clone(), input_sr.is_case_sensitive)
                        };

                        Self::update_search_state(&mut private, search_buffer, is_case_sensitive);
                    }
                    GraphicsCommand::ChangeToNormalMode => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private.screen.change_mode_to_normal(max_main_axis as usize);
                    }
                    GraphicsCommand::ChangeToSearchMode => {
                        let shared = private
                            .inputs_shared
                            .read()
                            .expect("Cannot get input lock for read");
                        let query = shared.search_buffer.clone();
                        let is_case_sensitive = shared.is_case_sensitive;

                        private
                            .screen
                            .change_mode_to_search(query, is_case_sensitive);
                    }
                    GraphicsCommand::Exit => break 'draw_loop,
                }
            }

            while let Ok(rx_msg) = private.rx.try_recv() {
                new_messages.push(BufferLine::new_rx(rx_msg.timestamp, rx_msg.message.clone()));
            }

            while let Ok(tx_msg) = private.tx.try_recv() {
                new_messages.push(BufferLine::new_tx(tx_msg.timestamp, tx_msg.message.clone()));
            }

            while let Ok(LogMessage {
                timestamp,
                message,
                level,
            }) = private.logger_receiver.try_recv()
            {
                let message = message.split("\n").collect::<Vec<_>>();
                let message_len = message.len();
                let log_msg_splited = message
                    .into_iter()
                    .filter(|msg| !msg.is_empty())
                    .enumerate()
                    .map(|(i, msg)| {
                        let message = if i == 0 {
                            msg.to_string()
                        } else {
                            "  ".to_string() + msg
                        }
                        .replace('\r', "")
                        .replace('\t', "    ")
                            + if i < (message_len - 1) { "\r\n" } else { "" };

                        BufferLine::new_log(timestamp, level, message.as_bytes().to_vec())
                    })
                    .collect::<Vec<_>>();

                new_messages.extend(log_msg_splited);
            }

            if !new_messages.is_empty() {
                need_redraw = true;
                new_messages.sort_by(|a, b| a.timestamp().partial_cmp(&b.timestamp()).unwrap());
                if private.recorder.is_recording()
                    && let Err(err) = private
                        .recorder
                        .add_bulk_content(new_messages.iter().map(|gm| gm.serialize()).collect())
                {
                    error!(private.logger, "{}", err);
                }
                private.typewriter += new_messages.iter().map(|gm| gm.serialize()).collect();
                private.buffer += new_messages;
                private.screen.update_after_new_lines(&private.buffer);
                save_stats.file_size = private.typewriter.get_size();
                new_messages = vec![];

                let (search_buffer, is_case_sensitive) = {
                    let input_sr = private
                        .inputs_shared
                        .read()
                        .expect("Cannot get input lock for read");
                    (input_sr.search_buffer.clone(), input_sr.is_case_sensitive)
                };

                Self::update_search_state(&mut private, search_buffer, is_case_sensitive);
            }

            if need_redraw {
                need_redraw = false;
                terminal
                    .draw(|f| {
                        let size = f.size();
                        let screen_size = Rect {
                            height: size.height.saturating_sub(Self::COMMAND_BAR_HEIGHT),
                            ..size
                        };
                        private.screen.set_size(screen_size);

                        let chunks = Layout::default()
                            .direction(Direction::Vertical)
                            .constraints([
                                Constraint::Length(screen_size.height),
                                Constraint::Length(Self::COMMAND_BAR_HEIGHT),
                            ])
                            .split(f.size());

                        private.screen.draw(
                            &private.buffer,
                            &save_stats,
                            f,
                            private.system_log_level,
                        );
                        Self::draw_command_bar(
                            &private.inputs_shared,
                            &private.serial_shared,
                            f,
                            chunks[1],
                            private.latency,
                            private.screen.search_indexes(),
                        );
                        Self::draw_autocomplete_list(&private.inputs_shared, f, chunks[1].y);
                    })
                    .expect("Error to draw");
                continue;
            }

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
        private: &mut GraphicsConnections,
        pattern: String,
        is_case_sensitive: bool,
    ) {
        let decoder = private.screen.decoder();
        let mode = private.screen.mode_mut();

        let pattern = if !is_case_sensitive {
            pattern.to_lowercase()
        } else {
            pattern
        };

        mode.set_query(pattern.clone(), is_case_sensitive);

        if pattern.is_empty() {
            return;
        }

        for message in private.buffer.iter() {
            let line = message.line;
            let message = message.decode(decoder).message;
            let message = ANSI::remove_encoding(message);

            let mut message = if !is_case_sensitive {
                message.to_lowercase()
            } else {
                message
            };

            let mut offset_x = 0;
            while let Some(column) = message.find(&pattern) {
                mode.add_entry(BufferPosition {
                    line,
                    column: column + offset_x,
                });
                let remaining = message.drain(..column + pattern.len()).collect::<String>();
                offset_x += remaining.len();
            }
        }

        private.screen.mode_mut().update_current();

        let max_main_axis = Self::max_main_axis(private);
        private
            .screen
            .jump_to_current_search(max_main_axis as usize);
    }

    fn max_main_axis(private: &GraphicsConnections) -> u16 {
        let buffer_len = private.buffer.len() as u16;
        let screen_height = private.screen.size().height.saturating_sub(2);

        buffer_len.saturating_sub(screen_height)
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
        config: GraphicsConfig,
    ) -> Self {
        Self {
            logger,
            logger_receiver,
            tx,
            rx,
            inputs_shared,
            serial_shared,
            buffer: Buffer::new(config.capacity),
            screen: Screen::default(),
            typewriter: TypeWriter::new(config.storage_base_filename.clone()),
            recorder: Recorder::new(config.storage_base_filename).expect("Cannot create Recorder"),
            system_log_level: LogLevel::Debug,
            latency: config.latency,
        }
    }
}
