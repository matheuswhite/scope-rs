use crate::graphics::special_char::{SpecialCharItem, ToSpecialChar};
use crate::infra::tags::TagList;
use crate::interfaces::rtt_if::{RttCommand, RttSetup};
use crate::interfaces::{InterfaceCommand, InterfaceType};
use crate::{
    debug, error,
    graphics::{graphics_task::GraphicsCommand, screen::ScreenPosition},
    info,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::Producer,
        task::Task,
    },
    interfaces::serial_if::{SerialCommand, SerialSetup},
    plugin::engine::PluginEngineCommand,
    success, warning,
};
use chrono::Local;
use core::panic;
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton};
use lipsum::lipsum;
use rand::{Rng, seq::SliceRandom};
use serialport::FlowControl;
use std::num::ParseIntError;
use std::ops::Range;
use std::sync::{
    Arc, RwLock,
    mpsc::{Receiver, Sender},
};

pub type InputsTask = Task<InputsShared, ()>;

#[derive(Default)]
pub struct InputsShared {
    pub search_cursor: usize,
    pub search_buffer: String,
    pub command_line: String,
    pub cursor: usize,
    pub history_len: usize,
    pub current_hint: Option<String>,
    pub mode: InputMode,
    pub is_case_sensitive: bool,
    pub tag_list: TagList,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    Search,
}

pub struct InputsConnections {
    logger: Logger,
    tx: Producer<Arc<TimedBytes>>,
    graphics_cmd_sender: Sender<GraphicsCommand>,
    interface_cmd_sender: Sender<InterfaceCommand>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    hints: Vec<&'static str>,
    history_index: Option<usize>,
    history: Vec<String>,
    backup_command_line: String,
    rx_channel: Producer<Arc<TimedBytes>>,
    has_tag_failed: bool,
    if_type: InterfaceType,
}

enum LoopStatus {
    Continue,
    Break,
}

impl InputsTask {
    pub fn spawn_inputs_task(
        inputs_connections: InputsConnections,
        inputs_cmd_sender: Sender<()>,
        inputs_cmd_receiver: Receiver<()>,
        tag_list: TagList,
    ) -> Self {
        let shared = InputsShared {
            tag_list,
            ..Default::default()
        };

        Self::new(
            shared,
            inputs_connections,
            Self::task,
            inputs_cmd_sender,
            inputs_cmd_receiver,
        )
    }

    fn handle_key_input(
        private: &mut InputsConnections,
        shared: Arc<RwLock<InputsShared>>,
        key: KeyEvent,
    ) -> LoopStatus {
        #[cfg(windows)]
        const ACTION_MODIFIER: KeyModifiers = KeyModifiers::CONTROL;
        #[cfg(not(windows))]
        const ACTION_MODIFIER: KeyModifiers = KeyModifiers::ALT;

        #[cfg(target_os = "macos")]
        const CTRL_MODIFIER: KeyModifiers = KeyModifiers::ALT;
        #[cfg(not(target_os = "macos"))]
        const CTRL_MODIFIER: KeyModifiers = KeyModifiers::CONTROL;

        match key.code {
            KeyCode::Esc => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => return LoopStatus::Break,
                    InputMode::Search => {
                        sw.mode = InputMode::Normal;
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ChangeToNormalMode);
                        return LoopStatus::Continue;
                    }
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') if key.modifiers == KeyModifiers::CONTROL => {
                let _ = private
                    .graphics_cmd_sender
                    .send(GraphicsCommand::CopyToClipboard);
            }
            KeyCode::Char('l') | KeyCode::Char('L') if key.modifiers == KeyModifiers::CONTROL => {
                let _ = private.graphics_cmd_sender.send(GraphicsCommand::Clear);
            }
            KeyCode::Char('s') | KeyCode::Char('S') if key.modifiers == KeyModifiers::CONTROL => {
                let _ = private.graphics_cmd_sender.send(GraphicsCommand::SaveData);
            }
            KeyCode::Char('r') | KeyCode::Char('R') if key.modifiers == KeyModifiers::CONTROL => {
                let _ = private
                    .graphics_cmd_sender
                    .send(GraphicsCommand::RecordData);
            }
            KeyCode::Char('f') | KeyCode::Char('F') if key.modifiers == KeyModifiers::CONTROL => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        sw.mode = InputMode::Search;

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ChangeToSearchMode);

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::SearchChange);
                    }
                    InputMode::Search => {
                        sw.mode = InputMode::Normal;

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ChangeToNormalMode);
                    }
                }
            }

            KeyCode::Right if key.modifiers == CTRL_MODIFIER => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                let (pos, buffer) = match sw.mode {
                    InputMode::Normal => (sw.cursor, &sw.command_line),
                    InputMode::Search => (sw.search_cursor, &sw.search_buffer),
                };

                let new_pos = buffer
                    .chars()
                    .skip(pos)
                    .zip(buffer.chars().skip(pos + 1))
                    .enumerate()
                    .find(|(_, (prev, current))| prev.is_whitespace() && !current.is_whitespace())
                    .map(|(i, _)| pos + i + 1)
                    .unwrap_or(buffer.chars().count());

                match sw.mode {
                    InputMode::Normal => sw.cursor = new_pos,
                    InputMode::Search => sw.search_cursor = new_pos,
                };

                Self::update_tag_list(&mut sw, private);
            }
            KeyCode::Left if key.modifiers == CTRL_MODIFIER => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                let (pos, buffer) = match sw.mode {
                    InputMode::Normal => (sw.cursor, &sw.command_line),
                    InputMode::Search => (sw.search_cursor, &sw.search_buffer),
                };

                let rev_pos = buffer.chars().count() - pos;

                let new_pos = buffer
                    .chars()
                    .rev()
                    .skip(rev_pos + 1)
                    .zip(buffer.chars().rev().skip(rev_pos))
                    .enumerate()
                    .find(|(_, (prev, current))| prev.is_whitespace() && !current.is_whitespace())
                    .map(|(i, _)| pos - i - 1)
                    .unwrap_or(0);

                match sw.mode {
                    InputMode::Normal => sw.cursor = new_pos,
                    InputMode::Search => sw.search_cursor = new_pos,
                };

                Self::update_tag_list(&mut sw, private);
            }
            KeyCode::Char('w') | KeyCode::Char('W') if key.modifiers == KeyModifiers::CONTROL => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                if matches!(sw.mode, InputMode::Search) {
                    sw.is_case_sensitive = !sw.is_case_sensitive;

                    let _ = private
                        .graphics_cmd_sender
                        .send(GraphicsCommand::SearchChange);
                }
            }
            KeyCode::Char(c) => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        sw.current_hint = None;

                        if sw.cursor >= sw.command_line.chars().count() {
                            sw.command_line.push(c);
                        } else {
                            sw.command_line = sw.command_line.chars().enumerate().fold(
                                "".to_string(),
                                |mut acc, (i, x)| {
                                    if i == sw.cursor {
                                        acc.push(c);
                                    }

                                    acc.push(x);
                                    acc
                                },
                            );
                        }

                        sw.cursor += 1;
                        Self::update_tag_list(&mut sw, private);
                        private.history_index = None;
                    }
                    InputMode::Search => {
                        if sw.search_cursor >= sw.search_buffer.chars().count() {
                            sw.search_buffer.push(c);
                        } else {
                            sw.search_buffer = sw.search_buffer.chars().enumerate().fold(
                                "".to_string(),
                                |mut acc, (i, x)| {
                                    if i == sw.search_cursor {
                                        acc.push(c);
                                    }

                                    acc.push(x);
                                    acc
                                },
                            );
                        }

                        sw.search_cursor += 1;
                        Self::update_tag_list(&mut sw, private);

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::SearchChange);
                    }
                }
            }
            KeyCode::PageUp if key.modifiers == ACTION_MODIFIER => {
                let _ = private
                    .graphics_cmd_sender
                    .send(GraphicsCommand::JumpToStart);
            }
            KeyCode::PageDown if key.modifiers == ACTION_MODIFIER => {
                let _ = private.graphics_cmd_sender.send(GraphicsCommand::JumpToEnd);
            }
            KeyCode::PageUp => {
                let _ = private.graphics_cmd_sender.send(GraphicsCommand::PageUp);
            }
            KeyCode::PageDown => {
                let _ = private.graphics_cmd_sender.send(GraphicsCommand::PageDown);
            }
            KeyCode::Backspace => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if sw.command_line.chars().count() == 1 {
                            Self::set_hint(&mut sw.current_hint, &private.hints);
                        }

                        if sw.cursor > 0 {
                            sw.cursor -= 1;
                            sw.command_line = sw
                                .command_line
                                .chars()
                                .enumerate()
                                .filter_map(|(i, c)| if i != sw.cursor { Some(c) } else { None })
                                .collect();
                            Self::update_tag_list(&mut sw, private);
                        }

                        if sw.command_line.chars().count() > 0
                            && sw.command_line.chars().all(|x| x.is_whitespace())
                        {
                            sw.command_line.clear();
                            sw.cursor = 0;
                            Self::set_hint(&mut sw.current_hint, &private.hints);
                        }
                    }
                    InputMode::Search => {
                        if sw.search_cursor > 0 {
                            sw.search_cursor -= 1;
                            sw.search_buffer = sw
                                .search_buffer
                                .chars()
                                .enumerate()
                                .filter_map(
                                    |(i, c)| if i != sw.search_cursor { Some(c) } else { None },
                                )
                                .collect();
                            Self::update_tag_list(&mut sw, private);

                            let _ = private
                                .graphics_cmd_sender
                                .send(GraphicsCommand::SearchChange);
                        }

                        if sw.search_buffer.chars().count() > 0
                            && sw.search_buffer.chars().all(|x| x.is_whitespace())
                        {
                            sw.search_buffer.clear();
                            sw.search_cursor = 0;

                            let _ = private
                                .graphics_cmd_sender
                                .send(GraphicsCommand::SearchChange);
                        }
                    }
                }
            }
            KeyCode::Delete => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        sw.command_line = sw
                            .command_line
                            .chars()
                            .enumerate()
                            .filter_map(|(i, c)| if i != sw.cursor { Some(c) } else { None })
                            .collect();
                        Self::update_tag_list(&mut sw, private);

                        if sw.command_line.chars().count() > 0
                            && sw.command_line.chars().all(|x| x.is_whitespace())
                        {
                            sw.command_line.clear();
                            sw.cursor = 0;
                            Self::set_hint(&mut sw.current_hint, &private.hints);
                        }
                    }
                    InputMode::Search => {
                        sw.search_buffer = sw
                            .search_buffer
                            .chars()
                            .enumerate()
                            .filter_map(|(i, c)| if i != sw.search_cursor { Some(c) } else { None })
                            .collect();
                        Self::update_tag_list(&mut sw, private);

                        if sw.search_buffer.chars().count() > 0
                            && sw.search_buffer.chars().all(|x| x.is_whitespace())
                        {
                            sw.search_buffer.clear();
                            sw.search_cursor = 0;
                        }

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::SearchChange);
                    }
                }
            }
            KeyCode::Right => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if sw.cursor < sw.command_line.chars().count() {
                            sw.cursor += 1;
                            Self::update_tag_list(&mut sw, private);
                        }
                    }
                    InputMode::Search => {
                        if sw.search_cursor < sw.search_buffer.chars().count() {
                            sw.search_cursor += 1;
                            Self::update_tag_list(&mut sw, private);
                        }
                    }
                }
            }
            KeyCode::Left => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if sw.cursor > 0 {
                            sw.cursor -= 1;
                            Self::update_tag_list(&mut sw, private);
                        }
                    }
                    InputMode::Search => {
                        if sw.search_cursor > 0 {
                            sw.search_cursor -= 1;
                            Self::update_tag_list(&mut sw, private);
                        }
                    }
                }
            }
            KeyCode::End => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        sw.cursor = sw.command_line.chars().count();
                    }
                    InputMode::Search => {
                        sw.search_cursor = sw.search_buffer.chars().count();
                    }
                }
                Self::update_tag_list(&mut sw, private);
            }
            KeyCode::Home => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        sw.cursor = 0;
                    }
                    InputMode::Search => {
                        sw.search_cursor = 0;
                    }
                }
                Self::update_tag_list(&mut sw, private);
            }
            KeyCode::Up => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if private.history.is_empty() {
                            return LoopStatus::Continue;
                        }

                        match &mut private.history_index {
                            None => {
                                private.history_index = Some(private.history.len() - 1);
                                private.backup_command_line.clone_from(&sw.command_line);
                            }
                            Some(0) => {}
                            Some(idx) => *idx -= 1,
                        }

                        sw.current_hint = None;
                        sw.command_line
                            .clone_from(&private.history[private.history_index.unwrap()]);
                        sw.cursor = sw.command_line.chars().count();
                        Self::update_tag_list(&mut sw, private);
                    }
                    InputMode::Search => {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::PrevSearch);
                    }
                }
            }
            KeyCode::Down => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if private.history.is_empty() {
                            return LoopStatus::Continue;
                        }

                        match &mut private.history_index {
                            None => {}
                            Some(idx) if *idx == (private.history.len() - 1) => {
                                private.history_index = None;
                                sw.command_line.clone_from(&private.backup_command_line);
                                if sw.command_line.is_empty() {
                                    Self::set_hint(&mut sw.current_hint, &private.hints);
                                }
                            }
                            Some(idx) => {
                                *idx += 1;
                                sw.command_line.clone_from(&private.history[*idx]);
                            }
                        }

                        sw.cursor = sw.command_line.chars().count();
                        Self::update_tag_list(&mut sw, private);
                    }
                    InputMode::Search => {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::NextSearch);
                    }
                }
            }
            KeyCode::Tab => {
                Self::handle_tab_input(private, shared.clone());
            }
            KeyCode::Enter if key.modifiers == ACTION_MODIFIER => {
                let sr = shared.read().expect("Cannot get input lock for read");

                if matches!(sr.mode, InputMode::Search) {
                    let _ = private
                        .graphics_cmd_sender
                        .send(GraphicsCommand::PrevSearch);
                }
            }
            KeyCode::Enter => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                match sw.mode {
                    InputMode::Normal => {
                        if sw.command_line.is_empty() {
                            if let KeyModifiers::ALT = key.modifiers {
                                private.tx.produce(Arc::new(TimedBytes {
                                    timestamp: Local::now(),
                                    message: b"\r\n".to_vec(),
                                }));
                            }

                            return LoopStatus::Continue;
                        }

                        let command_line = sw.command_line.drain(..).collect::<String>();
                        Self::set_hint(&mut sw.current_hint, &private.hints);

                        let empty_string = "".to_string();
                        let last_command = private.history.last().unwrap_or(&empty_string);
                        if last_command != &command_line {
                            private.history.push(command_line.clone());
                        }

                        sw.tag_list.clear();
                        private.history_index = None;
                        sw.cursor = 0;

                        if command_line.starts_with("!") {
                            let command_line_split = command_line
                                .strip_prefix('!')
                                .unwrap()
                                .split_whitespace()
                                .map(|arg| arg.to_string())
                                .collect();

                            Self::handle_user_command(command_line_split, private);
                        } else {
                            let command_line =
                                Self::replace_tag_sequence(command_line, &sw.tag_list);
                            let mut command_line = Self::replace_hex_sequence(command_line);

                            let end_bytes = if let KeyModifiers::ALT = key.modifiers {
                                b"".as_slice()
                            } else {
                                b"\r\n".as_slice()
                            };

                            command_line.extend_from_slice(end_bytes);

                            private.tx.produce(Arc::new(TimedBytes {
                                timestamp: Local::now(),
                                message: command_line,
                            }));
                        }
                    }
                    InputMode::Search => {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::NextSearch);
                    }
                }
            }
            _ => {}
        }

        LoopStatus::Continue
    }

    fn replace_range_chars(text: &str, range: Range<usize>, replacement: &str) -> String {
        let mut new_text = String::new();

        new_text.push_str(&text.chars().take(range.start).collect::<String>());
        new_text.push_str(replacement);
        new_text.push_str(&text.chars().skip(range.end).collect::<String>());

        new_text
    }

    fn handle_tab_input(private: &mut InputsConnections, shared: Arc<RwLock<InputsShared>>) {
        let mut sw = shared.write().expect("Cannot get input lock for write");

        let Some(first_entry) = sw.tag_list.get_first_autocomplete_list() else {
            return;
        };

        let first_entry = format!("@{}", first_entry);
        let first_entry_len = first_entry.chars().count();

        let InputMode::Normal = sw.mode else {
            return;
        };

        let pattern = sw.tag_list.pattern();
        let pattern_len = pattern.chars().count();
        let cursor = sw.cursor;
        let start = cursor.saturating_sub(pattern_len);

        sw.command_line = Self::replace_range_chars(&sw.command_line, start..cursor, &first_entry);
        sw.cursor += first_entry_len - pattern_len;

        let command_line_len = sw.command_line.chars().count();
        let cursor_after = sw.cursor;
        let white_space = sw
            .command_line
            .chars()
            .skip(cursor_after)
            .position(|c| c.is_whitespace())
            .map(|pos| pos + cursor_after)
            .unwrap_or(command_line_len);
        sw.command_line =
            Self::replace_range_chars(&sw.command_line, cursor_after..white_space, "");

        Self::update_tag_list(&mut sw, private);
    }

    fn handle_serial_connect_command(command_line_split: Vec<String>, private: &InputsConnections) {
        fn mount_setup(option: &str, setup: Option<SerialSetup>) -> SerialSetup {
            if option.chars().all(|x| x.is_ascii_digit()) {
                SerialSetup {
                    baudrate: Some(option.parse::<u32>().unwrap()),
                    ..setup.unwrap_or_default()
                }
            } else {
                SerialSetup {
                    port: Some(option.to_string()),
                    ..setup.unwrap_or_default()
                }
            }
        }

        match command_line_split.len() {
            x if x < 2 => {
                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Serial(SerialCommand::Connect));
            }
            2 => {
                let setup = SerialCommand::Setup(mount_setup(&command_line_split[1], None));
                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Serial(setup));
            }
            _ => {
                let setup = mount_setup(&command_line_split[1], None);
                let setup = mount_setup(&command_line_split[2], Some(setup));

                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Serial(SerialCommand::Setup(setup)));
            }
        }
    }

    fn handle_rtt_connect_command(command_line_split: Vec<String>, private: &InputsConnections) {
        let mount_setup = |option: &str, setup: Option<RttSetup>| {
            if option.chars().all(|x| x.is_ascii_digit()) {
                RttSetup {
                    channel: Some(option.parse::<usize>().unwrap_or_default()),
                    ..setup.unwrap_or_default()
                }
            } else {
                RttSetup {
                    target: Some(option.to_string()),
                    ..setup.unwrap_or_default()
                }
            }
        };

        match command_line_split.len() {
            x if x < 2 => {
                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Rtt(RttCommand::Connect));
            }
            2 => {
                let setup = RttCommand::Setup(mount_setup(&command_line_split[1], None));
                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Rtt(setup));
            }
            _ => {
                let setup = mount_setup(&command_line_split[1], None);
                let setup = mount_setup(&command_line_split[2], Some(setup));

                let _ = private
                    .interface_cmd_sender
                    .send(InterfaceCommand::Rtt(RttCommand::Setup(setup)));
            }
        }
    }

    fn handle_connect_command(command_line_split: Vec<String>, private: &InputsConnections) {
        match private.if_type {
            InterfaceType::Rtt => Self::handle_rtt_connect_command(command_line_split, private),
            InterfaceType::Serial => {
                Self::handle_serial_connect_command(command_line_split, private)
            }
        }
    }

    fn parse_address_argument(arg: &str) -> Result<u64, ParseIntError> {
        let cleaned = arg.chars().filter(|c| *c != '_').collect::<String>();

        if cleaned.starts_with("0x") || cleaned.starts_with("0X") {
            u64::from_str_radix(&cleaned[2..], 16)
        } else {
            cleaned.parse::<u64>()
        }
    }

    fn handle_rtt_read_command(
        mut command_line_split: Vec<String>,
        private: &InputsConnections,
        logger: &Logger,
    ) {
        command_line_split.remove(0);

        let (address, size) = match command_line_split.len() {
            0 => {
                error!(logger, "Please, specify the address to read from");
                return;
            }
            1 => {
                let Ok(address) = Self::parse_address_argument(&command_line_split[0]) else {
                    error!(
                        logger,
                        "Invalid address argument. Please, specify a valid hexadecimal or decimal number: {}",
                        command_line_split[0]
                    );
                    return;
                };

                (address, 4usize)
            }
            2 => {
                let Ok(address) = Self::parse_address_argument(&command_line_split[0]) else {
                    error!(
                        logger,
                        "Invalid address argument. Please, specify a valid hexadecimal or decimal number: {}",
                        command_line_split[0]
                    );
                    return;
                };
                let Ok(size) = command_line_split[1].parse::<usize>() else {
                    error!(
                        logger,
                        "Invalid size argument. Please, specify a valid decimal number: {}",
                        command_line_split[1]
                    );
                    return;
                };
                if size == 0 {
                    error!(logger, "Size argument must be greater than 0: {}", size);
                    return;
                }

                (address, size)
            }
            _ => {
                error!(logger, "Too many arguments for RTT read command");
                return;
            }
        };

        let _ = private
            .interface_cmd_sender
            .send(InterfaceCommand::Rtt(RttCommand::Read { address, size }));
    }

    fn handle_flow_command(command_line_split: Vec<String>, private: &InputsConnections) {
        if command_line_split.len() < 2 {
            error!(
                private.logger,
                "Insufficient arguments for \"!flow\" command"
            );
            return;
        }

        let flow_control = match command_line_split[1].as_str() {
            "none" => FlowControl::None,
            "sw" => FlowControl::Software,
            "hw" => FlowControl::Hardware,
            _ => {
                error!(
                    private.logger,
                    "Invalid flow control. Please, chose one of these options: none, sw, hw"
                );
                return;
            }
        };

        let res =
            private
                .interface_cmd_sender
                .send(InterfaceCommand::Serial(SerialCommand::Setup(
                    SerialSetup {
                        flow_control: Some(flow_control),
                        ..SerialSetup::default()
                    },
                )));

        match res {
            Ok(_) => success!(
                private.logger,
                "Flow control setted to \"{}\"",
                command_line_split[1]
            ),
            Err(err) => error!(private.logger, "Cannot set flow control: {}", err),
        }
    }

    fn handle_user_command(command_line_split: Vec<String>, private: &InputsConnections) {
        let Some(cmd_name) = command_line_split.first() else {
            private.tx.produce(Arc::new(TimedBytes {
                timestamp: Local::now(),
                message: vec![b'!'],
            }));
            return;
        };

        match cmd_name.as_str() {
            "serial" => {
                if command_line_split.len() < 2 {
                    error!(
                        private.logger,
                        "Please, use \"connect\" or \"disconnect\" subcommands"
                    );
                    return;
                }

                match command_line_split.get(1).unwrap().as_str() {
                    "connect" => {
                        Self::handle_serial_connect_command(
                            command_line_split[1..].to_vec(),
                            private,
                        );
                    }
                    "disconnect" => {
                        let _ = private
                            .interface_cmd_sender
                            .send(InterfaceCommand::Serial(SerialCommand::Disconnect));
                    }
                    "flow" => match private.if_type {
                        InterfaceType::Serial => {
                            Self::handle_flow_command(command_line_split[1..].to_vec(), private);
                        }
                        InterfaceType::Rtt => {
                            error!(
                                private.logger,
                                "Flow control is only available for serial interfaces"
                            );
                        }
                    },
                    _ => {
                        error!(private.logger, "Invalid subcommand for serial");
                    }
                }
            }
            "rtt" => {
                if command_line_split.len() < 2 {
                    error!(
                        private.logger,
                        "Please, use \"connect\", \"disconnect\" or \"read\" subcommands"
                    );
                    return;
                }

                match command_line_split.get(1).unwrap().as_str() {
                    "connect" => {
                        Self::handle_rtt_connect_command(command_line_split[1..].to_vec(), private);
                    }
                    "disconnect" => {
                        let _ = private
                            .interface_cmd_sender
                            .send(InterfaceCommand::Rtt(RttCommand::Disconnect));
                    }
                    "read" => {
                        Self::handle_rtt_read_command(
                            command_line_split[1..].to_vec(),
                            private,
                            &private.logger,
                        );
                    }
                    _ => {
                        error!(private.logger, "Invalid subcommand for rtt");
                    }
                }
            }
            "ipsum" => {
                Self::handle_ipsum_command(command_line_split, private);
            }
            "connect" => {
                Self::handle_connect_command(command_line_split, private);
            }
            "disconnect" => {
                let disconn_cmd = match private.if_type {
                    InterfaceType::Serial => InterfaceCommand::Serial(SerialCommand::Disconnect),
                    InterfaceType::Rtt => InterfaceCommand::Rtt(RttCommand::Disconnect),
                };

                let _ = private.interface_cmd_sender.send(disconn_cmd);
            }
            "flow" => match private.if_type {
                InterfaceType::Serial => {
                    Self::handle_flow_command(command_line_split, private);
                }
                InterfaceType::Rtt => {
                    error!(
                        private.logger,
                        "Flow control is only available for serial interfaces"
                    );
                }
            },
            "log" => {
                if command_line_split.len() < 3 {
                    error!(
                        private.logger,
                        "Insufficient arguments for \"!log\" command"
                    );
                    return;
                }

                let module = command_line_split[1].as_str();
                let log_level = match command_line_split[2].as_str() {
                    "debug" | "dbg" | "all" => LogLevel::Debug,
                    "info" | "inf" => LogLevel::Info,
                    "success" | "ok" => LogLevel::Success,
                    "warning" | "wrn" => LogLevel::Warning,
                    "error" | "err" => LogLevel::Error,
                    _ => {
                        error!(
                            private.logger,
                            "Invalid log level. Please, choose one of these options: debug, info, success, warning, error"
                        );
                        return;
                    }
                };

                if module == "system" || module == "sys" {
                    let _ = private
                        .graphics_cmd_sender
                        .send(GraphicsCommand::SetLogLevel(log_level));
                } else {
                    let _ =
                        private
                            .plugin_engine_cmd_sender
                            .send(PluginEngineCommand::SetLogLevel {
                                plugin_name: module.to_string(),
                                log_level,
                            });
                }
            }
            "plugin" => {
                if command_line_split.len() < 3 {
                    error!(
                        private.logger,
                        "Insufficient arguments for \"!plugin\" command"
                    );
                    return;
                }

                let command = command_line_split[1].as_str();

                match command {
                    "load" | "reload" => {
                        let filepath = command_line_split[2].clone();

                        let _ = private
                            .plugin_engine_cmd_sender
                            .send(PluginEngineCommand::LoadPlugin { filepath });
                    }
                    "unload" => {
                        let plugin_name = command_line_split[2].clone();

                        let _ = private
                            .plugin_engine_cmd_sender
                            .send(PluginEngineCommand::UnloadPlugin { plugin_name });
                    }
                    _ => {
                        error!(
                            private.logger,
                            "Invalid command. Please, choose one of these options: load, reload, unload"
                        );
                    }
                }
            }
            _ => {
                if command_line_split.len() < 2 {
                    error!(private.logger, "Insufficient arguments");
                    return;
                }

                let plugin_name = command_line_split[0].clone();
                let command = command_line_split[1].clone();
                let options = command_line_split
                    .get(2..)
                    .map(|v| v.to_vec())
                    .unwrap_or(vec![]);

                let _ = private
                    .plugin_engine_cmd_sender
                    .send(PluginEngineCommand::UserCommand {
                        plugin_name,
                        command,
                        options,
                    });
            }
        }
    }

    fn handle_ipsum_command(command_line_split: Vec<String>, private: &InputsConnections) {
        let n_words = rand::thread_rng().gen_range(10..=100);
        let split_size = rand::thread_rng().gen_range(20..=80);
        let ipsum = lipsum(n_words);
        let ipsum = ipsum
            .chars()
            .collect::<Vec<char>>()
            .chunks(split_size)
            .map(|chunk| chunk.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join("\r\n");

        let has_special_chars = command_line_split.iter().any(|s| s == "--sp");
        let has_ansi_colors = command_line_split.iter().any(|s| s == "--ansi");
        let mode = command_line_split
            .get(1)
            .map(|s| s.as_str())
            .unwrap_or("rx");

        let ipsum = if has_special_chars {
            ipsum
                .chars()
                .map(|c| {
                    let outcome = rand::random::<f32>();
                    if outcome <= 0.01 {
                        rand::random::<u8>() as char
                    } else {
                        c
                    }
                })
                .collect::<String>()
        } else {
            ipsum
        };

        let ipsum = if has_ansi_colors {
            let colors = [
                "\x1b[31m", // Red
                "\x1b[32m", // Green
                "\x1b[33m", // Yellow
                "\x1b[34m", // Blue
                "\x1b[35m", // Magenta
                "\x1b[36m", // Cyan
                "\x1b[37m", // White
            ];

            let mut rng = rand::thread_rng();

            ipsum
                .lines()
                .map(|line| {
                    let mut positions = line.char_indices().map(|(i, _)| i).collect::<Vec<_>>();
                    positions.push(line.len());

                    let chars = positions.len().saturating_sub(1);
                    if chars < 2 {
                        return line.to_string();
                    }

                    let start_char = rng.gen_range(0..chars - 1);
                    let end_char = rng.gen_range(start_char + 1..=chars);

                    let start = positions[start_char];
                    let end = positions[end_char];

                    let colored_line = &line[start..end];
                    let (left, line, right) = (&line[..start], colored_line, &line[end..]);
                    let color = colors.choose(&mut rng).unwrap_or(&"\x1b[37m");

                    format!("{}{}{}{}{}", left, color, line, "\x1b[0m", right)
                })
                .collect::<Vec<String>>()
                .join("\r\n")
        } else {
            ipsum
        };

        match mode {
            "err" => {
                for line in ipsum.lines() {
                    error!(private.logger, "{}", line);
                }
            }
            "warn" => {
                for line in ipsum.lines() {
                    warning!(private.logger, "{}", line);
                }
            }
            "ok" => {
                for line in ipsum.lines() {
                    success!(private.logger, "{}", line);
                }
            }
            "inf" => {
                for line in ipsum.lines() {
                    info!(private.logger, "{}", line);
                }
            }
            "dbg" => {
                for line in ipsum.lines() {
                    debug!(private.logger, "{}", line);
                }
            }
            "rx" => {
                let timestamp = Local::now();
                for line in ipsum.lines() {
                    let message = format!("{}\r\n", line);
                    private.rx_channel.produce(Arc::new(TimedBytes {
                        timestamp,
                        message: message.into_bytes(),
                    }));
                }
            }
            "tx" => {
                let timestamp = Local::now();
                for line in ipsum.lines() {
                    let message = format!("{}\r\n", line);
                    private.tx.produce(Arc::new(TimedBytes {
                        timestamp,
                        message: message.into_bytes(),
                    }));
                }
            }
            _ => {
                error!(
                    private.logger,
                    "Invalid mode for \"!ipsum\" command. Valid modes are: rx, tx, dbg, inf, ok, warn, err"
                );
            }
        }
    }

    pub fn task(
        shared: Arc<RwLock<InputsShared>>,
        mut private: InputsConnections,
        _inputs_cmd_receiver: Receiver<()>,
    ) {
        {
            let mut sw = shared.write().expect("Cannot get input lock for write");
            Self::set_hint(&mut sw.current_hint, &private.hints);
        }

        'input_loop: loop {
            let evt = match event::read() {
                Ok(evt) => evt,
                Err(err) => panic!("Error at input task: {:?}", err),
            };

            match evt {
                event::Event::FocusGained => {}
                event::Event::FocusLost => {}
                event::Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if let LoopStatus::Break =
                        Self::handle_key_input(&mut private, shared.clone(), key)
                    {
                        let _ = private
                            .plugin_engine_cmd_sender
                            .send(PluginEngineCommand::Exit);
                        let exit_cmd = match private.if_type {
                            InterfaceType::Serial => InterfaceCommand::Serial(SerialCommand::Exit),
                            InterfaceType::Rtt => InterfaceCommand::Rtt(RttCommand::Exit),
                        };
                        let _ = private.interface_cmd_sender.send(exit_cmd);
                        let _ = private.graphics_cmd_sender.send(GraphicsCommand::Exit);
                        break 'input_loop;
                    }
                }
                event::Event::Mouse(mouse_evt) => match mouse_evt.kind {
                    event::MouseEventKind::ScrollUp
                        if mouse_evt.modifiers == KeyModifiers::CONTROL =>
                    {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ScrollLeft);
                    }
                    event::MouseEventKind::ScrollDown
                        if mouse_evt.modifiers == KeyModifiers::CONTROL =>
                    {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ScrollRight);
                    }
                    event::MouseEventKind::ScrollDown => {
                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::ScrollDown);
                    }
                    event::MouseEventKind::ScrollUp => {
                        let _ = private.graphics_cmd_sender.send(GraphicsCommand::ScrollUp);
                    }
                    event::MouseEventKind::Down(MouseButton::Left) => {
                        let point = ScreenPosition {
                            x: mouse_evt.column,
                            y: mouse_evt.row,
                        };

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::Click(point));
                    }
                    event::MouseEventKind::Drag(MouseButton::Left) => {
                        let point = ScreenPosition {
                            x: mouse_evt.column,
                            y: mouse_evt.row,
                        };

                        let _ = private
                            .graphics_cmd_sender
                            .send(GraphicsCommand::Move(point));
                    }
                    _ => {}
                },
                event::Event::Paste(_) => {}
                event::Event::Resize(_, _) => {}
                _ => continue 'input_loop,
            }

            let _ = private.graphics_cmd_sender.send(GraphicsCommand::Redraw);
        }
    }

    fn set_hint(current_hint: &mut Option<String>, hints: &[&'static str]) {
        *current_hint = Some(hints.choose(&mut rand::thread_rng()).unwrap().to_string());
    }

    fn replace_hex_sequence(command_line: String) -> Vec<u8> {
        let mut output = vec![];
        let mut in_hex_seq = false;
        let valid = "0123456789abcdefABCDEF,_-. ";
        let mut hex_shift = 0;
        let mut hex_val = None;

        for c in command_line.chars() {
            if !in_hex_seq {
                if c == '$' {
                    in_hex_seq = true;
                    hex_shift = 0;
                    hex_val = Some(0);
                    continue;
                }

                output.push(c as u8);
            } else {
                if !valid.contains(c) {
                    in_hex_seq = false;
                    output.push(c as u8);
                    continue;
                }

                match c {
                    '0'..='9' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - b'0';
                    }
                    'a'..='f' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - b'a' + 0x0a;
                    }
                    'A'..='F' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - b'A' + 0x0A;
                    }
                    _ => {
                        if let Some(hex) = hex_val.take() {
                            output.push(hex);
                        }
                        hex_shift = 0;
                        continue;
                    }
                }

                if hex_shift == 0 {
                    hex_shift = 4;
                } else {
                    if let Some(hex) = hex_val.take() {
                        output.push(hex);
                    }
                    hex_shift = 0;
                }
            }
        }

        output
    }

    fn replace_tag_sequence(command_line: String, tag_list: &TagList) -> String {
        let mut res = String::with_capacity(command_line.len());

        for item in command_line.to_special_char(|string| tag_list.tag_filter(string)) {
            match item {
                SpecialCharItem::Plain(s) => {
                    res.push_str(&s);
                }
                SpecialCharItem::Special(s, _column) => {
                    let tag_value = tag_list.get_tagged_key(&s);
                    res.push_str(&tag_value);
                }
            }
        }

        res
    }

    fn update_tag_list(sw: &mut InputsShared, private: &mut InputsConnections) {
        let (buffer, cursor) = if sw.mode == InputMode::Normal {
            (&sw.command_line, sw.cursor)
        } else {
            (&sw.search_buffer, sw.search_cursor)
        };

        if let Err(err) = sw.tag_list.reload() {
            if !private.has_tag_failed {
                error!(private.logger, "Failed to reload tag list: {}", err);
                private.has_tag_failed = true;
                sw.tag_list.full_clear();
            }
            return;
        }

        private.has_tag_failed = false;

        sw.tag_list.update_pattern(buffer, cursor);
        sw.tag_list.update_autocomplete_list();
    }
}

impl InputsConnections {
    pub fn new(
        logger: Logger,
        tx: Producer<Arc<TimedBytes>>,
        graphics_cmd_sender: Sender<GraphicsCommand>,
        interface_cmd_sender: Sender<InterfaceCommand>,
        plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
        rx_channel: Producer<Arc<TimedBytes>>,
        if_type: InterfaceType,
    ) -> Self {
        Self {
            logger,
            tx,
            graphics_cmd_sender,
            interface_cmd_sender,
            plugin_engine_cmd_sender,
            history_index: None,
            hints: vec![
                "Type @ to place a tag",
                "Type $ to start a hex sequence",
                "Type here and hit <Enter> to send the text",
            ],
            history: vec![],
            backup_command_line: String::new(),
            rx_channel,
            has_tag_failed: false,
            if_type,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InputsTask;

    #[test]
    fn test_rhs_one() {
        let res = InputsTask::replace_hex_sequence("$61".to_string());

        assert_eq!(&res, b"a");
    }

    #[test]
    fn test_rhs_two_no_sep() {
        let res = InputsTask::replace_hex_sequence("$6161".to_string());

        assert_eq!(&res, b"aa");
    }

    #[test]
    fn test_rhs_two_comma() {
        let res = InputsTask::replace_hex_sequence("$61,61".to_string());

        assert_eq!(&res, b"aa");
    }

    #[test]
    fn test_all_bytes() {
        let mut command_line = "$".to_string();
        let mut expected = vec![];
        for b in 0u8..=0xff {
            command_line.push_str(&format!("{:02x},", b));
            expected.push(b);
        }
        for b in 0u8..=0xff {
            command_line.push_str(&format!("{:02X},", b));
            expected.push(b);
        }

        let res = InputsTask::replace_hex_sequence(command_line.clone());
        let mut it = res.iter().enumerate();

        for (i, b) in &mut it {
            assert_eq!(*b, i as u8);
        }
        for (i, b) in it {
            assert_eq!(*b, i as u8);
        }

        assert_eq!(&res, &expected);
    }
}
