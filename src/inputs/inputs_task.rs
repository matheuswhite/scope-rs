use crate::{
    error,
    graphics::graphics_task::GraphicsCommand,
    infra::{
        logger::{LogLevel, Logger},
        messages::TimedBytes,
        mpmc::Producer,
        task::Task,
    },
    plugin::plugin_engine::PluginEngineCommand,
    serial::serial_if::{SerialCommand, SerialSetup},
};
use chrono::Local;
use core::panic;
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use rand::seq::SliceRandom;
use std::{
    path::PathBuf,
    sync::{
        mpsc::{Receiver, Sender},
        Arc, RwLock,
    },
};

pub type InputsTask = Task<InputsShared, ()>;

#[derive(Default)]
pub struct InputsShared {
    pub command_line: String,
    pub cursor: usize,
    pub history_len: usize,
    pub current_hint: Option<String>,
    pub autocomplete_list: Vec<Arc<String>>,
    pub pattern: String,
}

pub struct InputsConnections {
    logger: Logger,
    tx: Producer<Arc<TimedBytes>>,
    graphics_cmd_sender: Sender<GraphicsCommand>,
    serial_if_cmd_sender: Sender<SerialCommand>,
    plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
    hints: Vec<&'static str>,
    history_index: Option<usize>,
    history: Vec<String>,
    backup_command_line: String,
    tag_file: PathBuf,
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
    ) -> Self {
        Self::new(
            InputsShared::default(),
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
        match key.code {
            KeyCode::Esc => {
                return LoopStatus::Break;
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
            KeyCode::Char(c) => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

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
                Self::update_tag_list();
                private.history_index = None;
            }
            KeyCode::PageUp if key.modifiers == KeyModifiers::CONTROL => {
                let _ = private
                    .graphics_cmd_sender
                    .send(GraphicsCommand::JumpToStart);
            }
            KeyCode::PageDown if key.modifiers == KeyModifiers::CONTROL => {
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
                    Self::update_tag_list();
                }

                if sw.command_line.chars().count() > 0
                    && sw.command_line.chars().all(|x| x.is_whitespace())
                {
                    sw.command_line.clear();
                    sw.cursor = 0;
                    Self::set_hint(&mut sw.current_hint, &private.hints);
                }
            }
            KeyCode::Delete => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                sw.command_line = sw
                    .command_line
                    .chars()
                    .enumerate()
                    .filter_map(|(i, c)| if i != sw.cursor { Some(c) } else { None })
                    .collect();
                Self::update_tag_list();

                if sw.command_line.chars().count() > 0
                    && sw.command_line.chars().all(|x| x.is_whitespace())
                {
                    sw.command_line.clear();
                    sw.cursor = 0;
                    Self::set_hint(&mut sw.current_hint, &private.hints);
                }
            }
            KeyCode::Right => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                if sw.cursor < sw.command_line.chars().count() {
                    sw.cursor += 1;
                }
            }
            KeyCode::Left => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                if sw.cursor > 0 {
                    sw.cursor -= 1;
                }
            }
            KeyCode::Up => {
                if private.history.is_empty() {
                    return LoopStatus::Continue;
                }

                let mut sw = shared.write().expect("Cannot get input lock for write");

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
                Self::update_tag_list();
            }
            KeyCode::Down => {
                if private.history.is_empty() {
                    return LoopStatus::Continue;
                }

                let mut sw = shared.write().expect("Cannot get input lock for write");

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
                Self::update_tag_list();
            }
            KeyCode::Enter => {
                let mut sw = shared.write().expect("Cannot get input lock for write");

                if sw.command_line.is_empty() {
                    return LoopStatus::Continue;
                }

                let command_line = sw.command_line.drain(..).collect::<String>();
                Self::set_hint(&mut sw.current_hint, &private.hints);

                let empty_string = "".to_string();
                let last_command = private.history.last().unwrap_or(&empty_string);
                if last_command != &command_line {
                    private.history.push(command_line.clone());
                }

                Self::clear_tag_list();
                private.history_index = None;
                sw.cursor = 0;
                drop(sw);

                if command_line.starts_with("!") {
                    let command_line_split = command_line
                        .strip_prefix('!')
                        .unwrap()
                        .split_whitespace()
                        .map(|arg| arg.to_string())
                        .collect();

                    Self::handle_user_command(command_line_split, &private);
                } else {
                    let command_line = Self::replace_hex_sequence(command_line);
                    let command_line = Self::replace_tag_sequence(command_line, &private.tag_file);

                    private.tx.produce(Arc::new(TimedBytes {
                        timestamp: Local::now(),
                        message: (command_line + "\r\n").as_bytes().to_vec(),
                    }));
                }
            }
            _ => {}
        }

        LoopStatus::Continue
    }

    fn handle_user_command(command_line_split: Vec<String>, private: &InputsConnections) {
        let Some(cmd_name) = command_line_split.get(0) else {
            private.tx.produce(Arc::new(TimedBytes {
                timestamp: Local::now(),
                message: vec!['!' as u8],
            }));
            return;
        };

        match cmd_name.as_str() {
            "connect" => {
                fn mount_setup(option: &str, setup: Option<SerialSetup>) -> SerialSetup {
                    if option.chars().all(|x| x.is_digit(10)) {
                        SerialSetup {
                            baudrate: Some(u32::from_str_radix(option, 10).unwrap()),
                            ..setup.unwrap_or(SerialSetup::default())
                        }
                    } else {
                        SerialSetup {
                            port: Some(option.to_string()),
                            ..setup.unwrap_or(SerialSetup::default())
                        }
                    }
                }

                match command_line_split.len() {
                    x if x < 2 => {
                        let _ = private.serial_if_cmd_sender.send(SerialCommand::Connect);
                    }
                    2 => {
                        let setup = SerialCommand::Setup(mount_setup(&command_line_split[1], None));
                        let _ = private.serial_if_cmd_sender.send(setup);
                    }
                    _ => {
                        let setup = mount_setup(&command_line_split[1], None);
                        let setup = mount_setup(&command_line_split[2], Some(setup));

                        let _ = private
                            .serial_if_cmd_sender
                            .send(SerialCommand::Setup(setup));
                    }
                }
            }
            "disconnect" => {
                let _ = private.serial_if_cmd_sender.send(SerialCommand::Disconnect);
            }
            _ => error!(private.logger, "Invalid command \"{}\"", cmd_name),
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
                        let _ = private.serial_if_cmd_sender.send(SerialCommand::Exit);
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
                    _ => {}
                },
                event::Event::Paste(_) => {}
                event::Event::Resize(_, _) => {}
                _ => {}
            }
        }
    }

    fn set_hint(current_hint: &mut Option<String>, hints: &[&'static str]) {
        *current_hint = Some(hints.choose(&mut rand::thread_rng()).unwrap().to_string());
    }

    fn replace_hex_sequence(command_line: String) -> String {
        let mut output = String::new();
        let mut in_hex_seq = false;
        let valid = "0123456789abcdefABCDEF,_-.";
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

                output.push(c);
            } else {
                if !valid.contains(c) {
                    in_hex_seq = false;
                    output.push(c);
                    continue;
                }

                match c {
                    '0'..='9' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - '0' as u8;
                    }
                    'a'..='f' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - 'a' as u8;
                    }
                    'A'..='F' => {
                        *hex_val.get_or_insert(0) <<= hex_shift;
                        *hex_val.get_or_insert(0) |= c as u8 - 'A' as u8;
                    }
                    _ => {
                        if let Some(hex) = hex_val.take() {
                            output.push(hex as char);
                        }
                        hex_shift = 0;
                    }
                }

                if hex_shift == 0 {
                    hex_shift = 4;
                } else {
                    if let Some(hex) = hex_val.take() {
                        output.push(hex as char);
                    }
                    hex_shift = 0;
                }
            }
        }

        output
    }

    fn replace_tag_sequence(command_line: String, _tag_file: &PathBuf) -> String {
        // TODO
        command_line
    }

    fn update_tag_list() {
        // TODO
    }

    fn clear_tag_list() {
        // TODO
    }
}

impl InputsConnections {
    pub fn new(
        logger: Logger,
        tx: Producer<Arc<TimedBytes>>,
        graphics_cmd_sender: Sender<GraphicsCommand>,
        serial_if_cmd_sender: Sender<SerialCommand>,
        plugin_engine_cmd_sender: Sender<PluginEngineCommand>,
        tag_file: PathBuf,
    ) -> Self {
        Self {
            logger,
            tx,
            graphics_cmd_sender,
            serial_if_cmd_sender,
            plugin_engine_cmd_sender,
            history_index: None,
            hints: vec![
                "Type @ to place a tag",
                "Type $ to start a hex sequence",
                "Type here and hit <Enter> to send the text",
            ],
            history: vec![],
            backup_command_line: String::new(),
            tag_file,
        }
    }
}
