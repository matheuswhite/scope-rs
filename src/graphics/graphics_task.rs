use super::Serialize;
use crate::graphics::ansi::ANSI;
use crate::graphics::buffer::{Buffer, BufferLine, BufferPosition};
use crate::graphics::message_filter::MessageFilter;
use crate::graphics::screen::{Screen, ScreenPosition};
use crate::graphics::special_char::{SpecialCharItem, ToSpecialChar};
use crate::inputs::inputs_task::InputMode;
use crate::interfaces::InterfaceShared;
use crate::interfaces::rtt_if::RttMode;
use crate::{error, info, inputs, success};
use crate::{
    infra::{
        backup::{Backup, backup_path},
        blink::Blink,
        logger::{LogLevel, LogMessage, Logger},
        messages::TimedBytes,
        mpmc::Consumer,
        recorder::Recorder,
        task::{Shared, Task},
        timer::Timer,
        typewriter::TypeWriter,
    },
    inputs::inputs_task::InputsShared,
    interfaces::serial_if::SerialMode,
    warning,
};
use arboard::Clipboard;
use crossterm::{
    event::{DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, block::Title},
};
use std::ops::Deref;
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
    interface_shared: Shared<InterfaceShared>,
    typewriter: TypeWriter,
    recorder: Recorder,
    backup: Backup,
    latency: u64,
    // The full received/sent/log history, kept regardless of the current
    // filter. `buffer` below is the filtered view derived from it and is what
    // the screen renders; rebuilding it from here is how clearing or changing
    // the filter brings previously-hidden lines back.
    full_buffer: Buffer,
    buffer: Buffer,
    message_filter: MessageFilter,
    screen: Screen,
    clipboard: Option<Clipboard>,
}

pub enum GraphicsCommand {
    SetLogLevel(LogLevel),
    SaveData,
    RecordData,
    Rename(String),
    SetFilter { pattern: String, exclude: bool },
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
    CopyToClipboard,
    ToggleBookmark(ScreenPosition),
    NextBookmark,
    PrevBookmark,
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
        interface_shared: &Shared<InterfaceShared>,
        frame: &mut Frame,
        rect: Rect,
        latency: u64,
        filter_label: &str,
    ) {
        let (title, is_connected) = {
            let interface_shared = interface_shared
                .read()
                .expect("Cannot get interface lock for read");

            match interface_shared.deref() {
                InterfaceShared::Rtt(rtt_shared) => {
                    let target = if rtt_shared.target.is_empty() {
                        "\"\"".to_string()
                    } else {
                        rtt_shared.target.clone()
                    };

                    (
                        format!("RTT {} [{}]", target, rtt_shared.channel),
                        matches!(rtt_shared.mode, RttMode::Connected),
                    )
                }
                InterfaceShared::Serial(serial_shared) => {
                    let port = if serial_shared.port.is_empty() {
                        "\"\"".to_string()
                    } else {
                        serial_shared.port.clone()
                    };

                    (
                        format!(
                            "Serial {}:{:04}bps{}",
                            port,
                            serial_shared.baudrate,
                            match serial_shared.flow_control {
                                serialport::FlowControl::None => "",
                                serialport::FlowControl::Software => ":SW",
                                serialport::FlowControl::Hardware => ":HW",
                            }
                        ),
                        matches!(serial_shared.mode, SerialMode::Connected),
                    )
                }
            }
        };

        let (text, cursor, history_len, current_hint, tag_list) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (
                inputs_shared.command_line.clone(),
                inputs_shared.cursor as u16,
                inputs_shared.history_len,
                inputs_shared.current_hint.clone(),
                inputs_shared.tag_list.clone(),
            )
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
            .title(format!("[{:03}][{}] {}", history_len, latency, title))
            .title(Title::from(filter_label.to_string()).alignment(Alignment::Right))
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(bar_color));

        let hint_is_some = current_hint.is_some();
        let hint = Span::styled(
            current_hint.map(|s| format!(" {}", s)).unwrap_or_default(),
            Style::default().fg(Color::DarkGray),
        );
        let hint = Line::from(hint);

        let text = format!(" {}", text)
            .to_special_char(|string| tag_list.tag_filter(string))
            .map(|item| match item {
                SpecialCharItem::Plain(s) => Span::from(s),
                SpecialCharItem::Special(s, _column) => {
                    Span::styled(s, Style::default().fg(Color::Cyan))
                }
            })
            .collect::<Vec<_>>();
        let text = Line::from(text);

        let paragraph = Paragraph::new(if hint_is_some { hint } else { text }).block(block);
        frame.render_widget(paragraph, rect);
        frame.set_cursor(cursor.0, cursor.1);
    }

    fn draw_command_bar_search_mode(
        inputs_shared: &Shared<InputsShared>,
        (search_current, search_total): (usize, usize),
        rect: Rect,
        frame: &mut Frame,
        is_case_sensitive: bool,
        is_regex: bool,
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
                "[{}][{}][{}/{}] Search Mode",
                if is_case_sensitive { "Aa" } else { "--" },
                if is_regex { ".*" } else { "  " },
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
        interface_shared: &Shared<InterfaceShared>,
        frame: &mut Frame,
        rect: Rect,
        latency: u64,
        search_indexes: Option<(usize, usize)>,
        filter_label: &str,
    ) {
        let (input_mode, is_case_sensitive, is_regex) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");
            (
                inputs_shared.mode,
                inputs_shared.is_case_sensitive,
                inputs_shared.is_regex,
            )
        };

        match input_mode {
            inputs::inputs_task::InputMode::Normal => Self::draw_command_bar_normal_mode(
                inputs_shared,
                interface_shared,
                frame,
                rect,
                latency,
                filter_label,
            ),
            inputs::inputs_task::InputMode::Search => Self::draw_command_bar_search_mode(
                inputs_shared,
                search_indexes.unwrap_or((0, 0)),
                rect,
                frame,
                is_case_sensitive,
                is_regex,
            ),
        }
    }

    pub fn draw_autocomplete_list(
        inputs_shared: &Shared<InputsShared>,
        frame: &mut Frame,
        command_bar_y: u16,
    ) {
        let (autocomplete_list, pattern, input_mode, cursor, command, selected) = {
            let inputs_shared = inputs_shared
                .read()
                .expect("Cannot get inputs lock for read");

            (
                inputs_shared.tag_list.autocomplete_list(),
                inputs_shared.tag_list.pattern(),
                inputs_shared.mode,
                inputs_shared.cursor,
                inputs_shared.command_line.clone(),
                inputs_shared.tag_list.selected(),
            )
        };

        if autocomplete_list.is_empty() || pattern.is_empty() || input_mode != InputMode::Normal {
            return;
        }

        // Window the list so the highlighted entry is always on screen, keeping
        // as many earlier entries visible as fit; a trailing `...` marks that
        // more entries exist below the window.
        let cap = min(frame.size().height as usize / 2, autocomplete_list.len()).max(1);
        let start = selected
            .saturating_sub(cap - 1)
            .min(autocomplete_list.len() - cap);
        let window = &autocomplete_list[start..start + cap];
        let has_more_below = start + cap < autocomplete_list.len();

        let longest_entry_len = window
            .iter()
            .fold(0u16, |len, x| max(len, x.chars().count() as u16));
        let row_count = window.len() as u16 + if has_more_below { 1 } else { 0 };
        let area_size = (longest_entry_len + 5, row_count + 2);
        let max_x = frame.size().x
            + frame
                .size()
                .width
                .saturating_sub(area_size.0)
                .saturating_sub(2);
        let latest_at = command
            .chars()
            .take(cursor)
            .enumerate()
            .filter(|(_, c)| *c == '@')
            .map(|(i, _)| i)
            .last()
            .unwrap_or(0);
        let area_x = latest_at.clamp(2, max_x as usize);
        let area_y = command_bar_y.saturating_sub(area_size.1);
        let area = Rect::new(area_x as u16, area_y, area_size.0, area_size.1);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .style(Style::default().fg(Color::Cyan));
        let inner_width = area_size.0.saturating_sub(2) as usize;
        let skip_chars = pattern.chars().count().saturating_sub(1);
        let mut text = window
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let suffix = x.as_str().chars().skip(skip_chars).collect::<String>();

                if start + i == selected {
                    // Highlighted entry: a full-width cyan bar so the current
                    // selection is unmistakable.
                    let content = format!(" {}{}", pattern, suffix);
                    let pad = inner_width.saturating_sub(content.chars().count());
                    Line::from(Span::styled(
                        format!("{}{}", content, " ".repeat(pad)),
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled(format!(" {}", pattern), Style::default().fg(Color::Cyan)),
                        Span::styled(suffix, Style::default().fg(Color::DarkGray)),
                    ])
                }
            })
            .collect::<Vec<_>>();
        if has_more_below {
            text.push(Line::from(Span::styled(
                " ...",
                Style::default().fg(Color::DarkGray),
            )));
        }
        let paragraph = Paragraph::new(text).block(block);

        frame.render_widget(Clear, area);
        frame.render_widget(paragraph, area);
    }

    fn handle_copy_to_clipboard(
        private: &mut GraphicsConnections,
        copy_blink: &mut Blink<Color>,
    ) -> Result<(), String> {
        let clipboard = private
            .clipboard
            .as_mut()
            .ok_or_else(|| "Clipboard not available in system".to_string())?;

        let Some(selection) = private.screen.selection() else {
            return Ok(());
        };

        let content = private
            .buffer
            .get_selection_content(selection, private.screen.decoder());
        if content.is_empty() {
            return Ok(());
        }

        clipboard
            .set_text(content.clone())
            .map_err(|err| format!("Failed to copy to clipboard: {}", err))?;

        copy_blink.start();

        Ok(())
    }

    pub fn task(
        _shared: Arc<RwLock<()>>,
        mut private: GraphicsConnections,
        cmd_receiver: Receiver<GraphicsCommand>,
    ) {
        enable_raw_mode().expect("Cannot enable terminal raw mode");
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )
        .expect("Cannot enable alternate screen, mouse capture and bracketed paste");
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).expect("Cannot create terminal backend");
        let mut save_blink = Blink::new(Duration::from_millis(200), 2, Color::Reset, Color::Black);
        let mut copy_blink = Blink::new(Duration::from_millis(150), 2, Color::Green, Color::Black);
        let mut new_messages = vec![];
        let mut need_redraw = true;
        // The terminal can be cleared from outside the app (e.g. Cmd+K in Zed's
        // terminal), which leaves ratatui's diff buffer out of sync so only
        // changed cells get repainted, blanking the rest. Periodically force a
        // full repaint so an externally-cleared screen heals on its own.
        let full_redraw_period = Duration::from_secs(1);
        let mut full_redraw_timer = Timer::new(full_redraw_period);
        full_redraw_timer.start();
        let mut save_stats = SaveStats::new(
            private.typewriter.get_size(),
            private.typewriter.get_filename(),
            save_blink.get_current(),
        );

        'draw_loop: loop {
            save_blink.tick();
            copy_blink.tick();

            // When the periodic timer fires, force a full repaint (see above).
            let mut force_full_redraw = false;
            if full_redraw_timer.tick() {
                full_redraw_timer.start();
                force_full_redraw = true;
                need_redraw = true;
            }

            save_stats.is_recording = private.recorder.is_recording();

            if save_blink.is_active() {
                need_redraw = true;
                save_stats.is_saving = true;
                save_stats.save_color = save_blink.get_current();
            } else if copy_blink.is_active() {
                need_redraw = true;
                save_stats.is_saving = true;
                save_stats.save_color = copy_blink.get_current();
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
                    GraphicsCommand::ToggleBookmark(pos) => {
                        private.screen.toggle_bookmark(&private.buffer, pos);
                    }
                    GraphicsCommand::NextBookmark => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private
                            .screen
                            .jump_to_next_bookmark(&private.buffer, max_main_axis as usize);
                    }
                    GraphicsCommand::PrevBookmark => {
                        let max_main_axis = Self::max_main_axis(&private);

                        private
                            .screen
                            .jump_to_previous_bookmark(&private.buffer, max_main_axis as usize);
                    }
                    GraphicsCommand::CopyToClipboard => {
                        if let Err(res) =
                            Self::handle_copy_to_clipboard(&mut private, &mut copy_blink)
                        {
                            error!(private.logger, "{}", res);
                        }
                    }
                    GraphicsCommand::SetLogLevel(level) => {
                        private.system_log_level = level;
                        success!(private.logger, "Log set to {:?}", level);
                    }
                    GraphicsCommand::SaveData => {
                        if private.recorder.is_recording() {
                            warning!(private.logger, "Cannot save file while recording.");
                            /* don't yield here, because we need to put this warning message on display */
                            continue 'draw_loop;
                        }

                        save_blink.start();
                        let filename = private.typewriter.get_filename();

                        match private.typewriter.flush() {
                            Ok(_) => success!(private.logger, "Content saved on \"{}\"", filename),
                            Err(err) => {
                                error!(private.logger, "Cannot save on \"{}\": {}", filename, err)
                            }
                        }
                    }
                    GraphicsCommand::Rename(name) => {
                        let filename = format!("{}.txt", name);
                        // A recording in progress keeps writing to its current
                        // file; remember it so the status bar stays accurate.
                        let recording_file = private
                            .recorder
                            .is_recording()
                            .then(|| private.recorder.get_filename());

                        match private.typewriter.rename(filename.clone()) {
                            Ok(_) => {
                                // Keep the recorder's base in sync (passing the
                                // full filename preserves dotted names) so a
                                // later `!record` uses the new session name too.
                                let _ = private.recorder.rename(&filename);
                                // Move the crash-recovery backup alongside the
                                // renamed session so it keeps mirroring it.
                                private
                                    .backup
                                    .rename(backup_path(&format!("{}.bkp", filename)));
                                save_stats.filename = recording_file
                                    .unwrap_or_else(|| private.typewriter.get_filename());
                                success!(private.logger, "Session renamed to \"{}\"", filename);
                            }
                            Err(err) => {
                                error!(private.logger, "Cannot rename session: {}", err)
                            }
                        }
                    }
                    GraphicsCommand::SetFilter { pattern, exclude } => {
                        // `exclude` selects the command: `!mute` (hide matches)
                        // vs `!filter` (show only matches). An empty pattern is
                        // the reset for each: `!filter` shows everything again,
                        // `!mute` mutes everything.
                        let changed = if pattern.is_empty() {
                            if exclude {
                                private.message_filter = MessageFilter::mute_all();
                                warning!(private.logger, "All received messages are muted");
                            } else {
                                private.message_filter = MessageFilter::default();
                                success!(
                                    private.logger,
                                    "Filter cleared; showing all received messages"
                                );
                            }
                            true
                        } else {
                            match MessageFilter::new(&pattern, exclude) {
                                Ok(filter) => {
                                    private.message_filter = filter;
                                    if exclude {
                                        success!(
                                            private.logger,
                                            "Muting received messages matching \"{}\"",
                                            pattern
                                        );
                                    } else {
                                        success!(
                                            private.logger,
                                            "Showing only received messages matching \"{}\"",
                                            pattern
                                        );
                                    }
                                    true
                                }
                                Err(err) => {
                                    error!(private.logger, "Invalid pattern: {}", err);
                                    false
                                }
                            }
                        };

                        // The filter is a view over the full history: re-derive
                        // the displayed buffer so lines hidden by a previous
                        // filter come back and lines the new filter rejects go
                        // away. Line indices change, so drop any stale selection
                        // and rebuild the search matches.
                        if changed {
                            Self::rebuild_displayed_buffer(&mut private);

                            let (search_buffer, is_case_sensitive, is_regex) = {
                                let input_sr = private
                                    .inputs_shared
                                    .read()
                                    .expect("Cannot get input lock for read");
                                (
                                    input_sr.search_buffer.clone(),
                                    input_sr.is_case_sensitive,
                                    input_sr.is_regex,
                                )
                            };
                            Self::update_search_state(
                                &mut private,
                                search_buffer,
                                is_case_sensitive,
                                is_regex,
                            );
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
                        private.full_buffer.clear();
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
                        let (search_buffer, is_case_sensitive, is_regex) = {
                            let input_sr = private
                                .inputs_shared
                                .read()
                                .expect("Cannot get input lock for read");
                            (
                                input_sr.search_buffer.clone(),
                                input_sr.is_case_sensitive,
                                input_sr.is_regex,
                            )
                        };

                        Self::update_search_state(
                            &mut private,
                            search_buffer,
                            is_case_sensitive,
                            is_regex,
                        );
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
                        let is_regex = shared.is_regex;

                        private
                            .screen
                            .change_mode_to_search(query, is_case_sensitive, is_regex);
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

                let serialized = new_messages
                    .iter()
                    .map(|gm| gm.serialize())
                    .collect::<Vec<_>>();

                // Mirror the history into the crash-recovery backup; the write
                // is handed to a background thread so the draw loop never stalls
                // on disk I/O.
                private.backup.append(serialized.clone());

                if private.recorder.is_recording()
                    && let Err(err) = private.recorder.add_bulk_content(serialized.clone())
                {
                    error!(private.logger, "{}", err);
                }
                private.typewriter += serialized;

                // The filter only affects what is displayed; the full history
                // (like the persistence above) keeps every line, so clearing or
                // changing the filter can bring hidden lines back. Only RX lines
                // that fail the current filter are kept out of the scrollback
                // view.
                let decoder = private.screen.decoder();
                let displayed = new_messages
                    .iter()
                    .filter(|line| private.message_filter.allows(line, decoder))
                    .cloned()
                    .collect::<Vec<_>>();
                private.full_buffer += new_messages;
                private.buffer += displayed;
                private.screen.update_after_new_lines(&private.buffer);
                save_stats.file_size = private.typewriter.get_size();
                new_messages = vec![];

                let (search_buffer, is_case_sensitive, is_regex) = {
                    let input_sr = private
                        .inputs_shared
                        .read()
                        .expect("Cannot get input lock for read");
                    (
                        input_sr.search_buffer.clone(),
                        input_sr.is_case_sensitive,
                        input_sr.is_regex,
                    )
                };

                Self::update_search_state(&mut private, search_buffer, is_case_sensitive, is_regex);
            }

            if need_redraw {
                need_redraw = false;
                // Reset ratatui's diff buffer so every cell is repainted, healing
                // a screen that was cleared outside the app (e.g. Cmd+K in Zed).
                if force_full_redraw {
                    terminal.clear().expect("Cannot clear terminal");
                }
                terminal
                    .draw(|f| {
                        let size = f.size();
                        let screen_size = Rect {
                            height: size.height.saturating_sub(Self::COMMAND_BAR_HEIGHT),
                            ..size
                        };
                        private.screen.set_size(screen_size, private.buffer.len());

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
                            &private.interface_shared,
                            f,
                            chunks[1],
                            private.latency,
                            private.screen.search_indexes(),
                            &private.message_filter.label(),
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
            DisableMouseCapture,
            DisableBracketedPaste
        )
        .expect("Cannot disable alternate screen, mouse capture and bracketed paste");
        terminal.show_cursor().expect("Cannot show mouse cursor");
    }

    /// Rebuilds the displayed `buffer` from the full history under the current
    /// filter. Called when the filter changes so hidden lines can reappear and
    /// newly-rejected lines drop out. Because every line is re-indexed, the
    /// scroll position is re-anchored to the bottom and any active selection is
    /// dropped (its old line/column no longer point at the same content).
    fn rebuild_displayed_buffer(private: &mut GraphicsConnections) {
        let decoder = private.screen.decoder();
        let displayed = private
            .full_buffer
            .iter()
            .filter(|line| private.message_filter.allows(line, decoder))
            .cloned()
            .collect::<Vec<_>>();

        private.buffer.clear();
        private.buffer += displayed;
        private.screen.clear();
        private.screen.update_after_new_lines(&private.buffer);
    }

    fn update_search_state(
        private: &mut GraphicsConnections,
        pattern: String,
        is_case_sensitive: bool,
        is_regex: bool,
    ) {
        let decoder = private.screen.decoder();
        let mode = private.screen.mode_mut();

        let is_empty = pattern.is_empty();
        mode.set_query(pattern, is_case_sensitive, is_regex);

        if is_empty {
            return;
        }

        for message in private.buffer.iter() {
            let line = message.line;
            let message = message.decode(decoder).message;
            let message = ANSI::remove_encoding(message);

            // Same matcher and same `message` as `search_line`, so the columns
            // recorded here match the highlighted spans exactly (regex or not).
            for (column, _len) in mode.search_matches(&message) {
                mode.add_entry(BufferPosition { line, column });
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
        interface_shared: Shared<InterfaceShared>,
        config: GraphicsConfig,
    ) -> Self {
        let backup = Backup::new(
            backup_path(&format!("{}.bkp", config.storage_base_filename)),
            logger.clone(),
        );

        Self {
            logger,
            logger_receiver,
            tx,
            rx,
            inputs_shared,
            interface_shared,
            full_buffer: Buffer::new(config.capacity),
            buffer: Buffer::new(config.capacity),
            message_filter: MessageFilter::default(),
            screen: Screen::default(),
            typewriter: TypeWriter::new(config.storage_base_filename.clone()),
            recorder: Recorder::new(config.storage_base_filename).expect("Cannot create Recorder"),
            backup,
            system_log_level: LogLevel::Debug,
            latency: config.latency,
            clipboard: Clipboard::new().ok(),
        }
    }
}
