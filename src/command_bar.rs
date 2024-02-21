use crate::command_bar::InputEvent::{HorizontalScroll, Key, VerticalScroll};
use crate::error_pop_up::ErrorPopUp;
use crate::messages::{SerialRxData, UserTxData};
use crate::plugin_manager::PluginManager;
use crate::serial::SerialIF;
use crate::text::TextView;
use chrono::Local;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};
use rand::seq::SliceRandom;
use std::cmp::{max, min};
use std::collections::btree_map::BTreeMap;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::thread::sleep;
use std::time::Duration;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use tui::Frame;

pub struct CommandBar<B: Backend> {
    interface: Arc<Mutex<SerialIF>>,
    text_view: Arc<Mutex<TextView<B>>>,
    command_line: String,
    command_line_idx: usize,
    command_filepath: Option<PathBuf>,
    history: Vec<String>,
    history_index: Option<usize>,
    backup_command_line: String,
    error_pop_up: Option<ErrorPopUp<B>>,
    command_list: CommandList,
    key_receiver: Receiver<InputEvent>,
    current_hint: Option<&'static str>,
    hints: Vec<&'static str>,
    plugin_manager: PluginManager,
}

impl<B: Backend + Send + Sync + 'static> CommandBar<B> {
    const HEIGHT: u16 = 3;

    pub fn new(interface: SerialIF, view_capacity: usize) -> Self {
        let (key_sender, key_receiver) = channel();

        thread::spawn(move || CommandBar::<B>::task(key_sender));

        let hints = vec![
            "Type / to send a command",
            "Type $ to start a hex sequence",
            "Type here and hit <Enter> to send",
        ];

        let interface = Arc::new(Mutex::new(interface));
        let text_view = Arc::new(Mutex::new(TextView::new(view_capacity)));

        let plugin_manager = PluginManager::new(interface.clone(), text_view.clone());

        Self {
            interface,
            text_view,
            command_line: String::new(),
            command_line_idx: 0,
            history: vec![],
            history_index: None,
            backup_command_line: String::new(),
            key_receiver,
            error_pop_up: None,
            command_filepath: None,
            command_list: CommandList::new(),
            hints: hints.clone(),
            current_hint: Some(hints.choose(&mut rand::thread_rng()).unwrap()),
            plugin_manager,
        }
    }

    pub fn with_command_file(mut self, filepath: &str) -> Self {
        self.command_filepath = Some(PathBuf::from(filepath));
        self
    }

    fn task(sender: Sender<InputEvent>) {
        loop {
            match crossterm::event::read().unwrap() {
                Event::Mouse(mouse_evt) if mouse_evt.modifiers == KeyModifiers::CONTROL => {
                    match mouse_evt.kind {
                        MouseEventKind::ScrollUp => sender.send(HorizontalScroll(-1)).unwrap(),
                        MouseEventKind::ScrollDown => sender.send(HorizontalScroll(1)).unwrap(),
                        _ => {}
                    }
                }
                Event::Key(key) => sender.send(Key(key)).unwrap(),
                Event::Mouse(mouse_evt) => match mouse_evt.kind {
                    MouseEventKind::ScrollUp => sender.send(VerticalScroll(-1)).unwrap(),
                    MouseEventKind::ScrollDown => sender.send(VerticalScroll(1)).unwrap(),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    pub fn draw(&self, f: &mut Frame<B>) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(f.size().height - CommandBar::<B>::HEIGHT),
                    Constraint::Length(CommandBar::<B>::HEIGHT),
                ]
                .as_ref(),
            )
            .split(f.size());

        {
            let text_view = self.text_view.lock().unwrap();
            text_view.draw(f, chunks[0]);
        }

        let (description, is_connected) = {
            let interface = self.interface.lock().unwrap();
            (interface.description(), interface.is_connected())
        };

        let cursor_pos = (
            chunks[1].x + self.command_line_idx as u16 + 2,
            chunks[1].y + 1,
        );
        let block = Block::default()
            .title(format!("[{:03}] {}", self.history.len(), description))
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(if is_connected {
                Color::LightGreen
            } else {
                Color::LightRed
            }));
        let paragraph = Paragraph::new(Span::from({
            " ".to_string()
                + if let Some(hint) = self.current_hint {
                    hint
                } else {
                    &self.command_line
                }
        }))
        .style(Style::default().fg(if self.current_hint.is_some() {
            Color::DarkGray
        } else {
            Color::Reset
        }))
        .block(block);
        f.render_widget(paragraph, chunks[1]);
        f.set_cursor(cursor_pos.0, cursor_pos.1);

        self.command_list.draw(f, chunks[1].y, Color::LightGreen);

        if let Some(pop_up) = self.error_pop_up.as_ref() {
            pop_up.draw(f, chunks[1].y);
        }
    }

    fn set_error_pop_up(&mut self, message: String) {
        self.error_pop_up = Some(ErrorPopUp::new(message));
    }

    fn update_command_list(&mut self) {
        if !self.command_line.starts_with('/') {
            self.command_list.clear();
            return;
        }

        let Some(filepath) = self.command_filepath.clone() else {
            self.set_error_pop_up("No YAML command file loaded!".to_string());
            return;
        };

        let yaml_content = self.load_commands(&filepath);
        if yaml_content.is_empty() {
            return;
        }

        let cmd_line = self.command_line.strip_prefix('/').unwrap();
        let cmds = yaml_content
            .keys()
            .filter_map(|x| {
                if x.starts_with(cmd_line) {
                    Some(x.clone())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        self.command_list
            .update_params(cmds, self.command_line.clone());
    }

    fn hex_string_to_bytes(hex_string: &str) -> Result<Vec<u8>, ()> {
        if hex_string.len() % 2 != 0 {
            return Err(());
        }

        if !hex_string
            .chars()
            .all(|x| "0123456789abcdefABCDEF".contains(x))
        {
            return Err(());
        }

        let res = (0..hex_string.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_string[i..(i + 2)], 16).unwrap())
            .collect();

        Ok(res)
    }

    fn show_hint(&mut self) {
        self.current_hint = Some(self.hints.choose(&mut rand::thread_rng()).unwrap());
    }

    fn clear_hint(&mut self) {
        self.current_hint = None;
    }

    fn handle_key_input(&mut self, key: KeyEvent, _term_size: Rect) -> Result<(), ()> {
        match key.code {
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                let mut text_view = self.text_view.lock().unwrap();
                text_view.clear()
            }
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => {
                self.error_pop_up.take();
            }
            KeyCode::Char(c) => {
                self.clear_hint();

                if self.command_line_idx >= self.command_line.chars().count() {
                    self.command_line.push(c);
                } else {
                    self.command_line = self.command_line.chars().enumerate().fold(
                        "".to_string(),
                        |mut acc, (i, x)| {
                            if i == self.command_line_idx {
                                acc.push(c);
                            }

                            acc.push(x);
                            acc
                        },
                    );
                }

                self.command_line_idx += 1;
                self.update_command_list();
                self.history_index = None;
            }
            KeyCode::Backspace => {
                if self.command_line.chars().count() == 1 {
                    self.show_hint();
                }
                self.command_line.pop();
                self.update_command_list();
                if self.command_line_idx > 0 {
                    self.command_line_idx -= 1;
                }
            }
            KeyCode::Right => {
                if self.command_line_idx == self.command_line.chars().count() {
                    return Ok(());
                }

                self.command_line_idx += 1;
            }
            KeyCode::Left => {
                if self.command_line_idx == 0 {
                    return Ok(());
                }

                self.command_line_idx -= 1;
            }
            KeyCode::Up => {
                if self.history.is_empty() {
                    return Ok(());
                }

                match &mut self.history_index {
                    None => {
                        self.history_index = Some(self.history.len() - 1);
                        self.backup_command_line = self.command_line.clone();
                    }
                    Some(0) => {}
                    Some(idx) => {
                        *idx -= 1;
                    }
                }

                self.clear_hint();
                self.command_line = self.history[self.history_index.unwrap()].clone();
                self.command_line_idx = self.command_line.chars().count();
                self.update_command_list();
            }
            KeyCode::Down => {
                if self.history.is_empty() {
                    return Ok(());
                }

                match &mut self.history_index {
                    None => {}
                    Some(idx) if *idx == (self.history.len() - 1) => {
                        self.history_index = None;
                        self.command_line = self.backup_command_line.clone();
                        if self.command_line.is_empty() {
                            self.show_hint();
                        }
                    }
                    Some(idx) => {
                        *idx += 1;
                        self.command_line = self.history[*idx].clone();
                    }
                }

                self.command_line_idx = self.command_line.chars().count();
                self.update_command_list();
            }
            KeyCode::Esc => {
                let interface = self.interface.lock().unwrap();
                interface.send(UserTxData::Exit);
                sleep(Duration::from_millis(100));
                return Err(());
            }
            KeyCode::Enter if !self.command_line.is_empty() => {
                let command_line = self.command_line.clone();
                self.show_hint();
                self.history.push(self.command_line.clone());
                self.command_line.clear();
                self.command_list.clear();
                self.history_index = None;
                self.command_line_idx = 0;

                match command_line.chars().next().unwrap() {
                    '/' => {
                        let Some(filepath) = self.command_filepath.clone() else {
                            self.set_error_pop_up("No YAML command file loaded!".to_string());
                            return Ok(());
                        };

                        let yaml_content = self.load_commands(&filepath);
                        if yaml_content.is_empty() {
                            return Ok(());
                        }

                        let key = command_line.strip_prefix('/').unwrap();

                        if !yaml_content.contains_key(key) {
                            self.set_error_pop_up(format!("Command </{}> not found", key));
                            return Ok(());
                        }

                        let data_to_send = yaml_content.get(key).unwrap();
                        let data_to_send = data_to_send.replace("\\r", "\r").replace("\\n", "\n");
                        let interface = self.interface.lock().unwrap();
                        interface.send(UserTxData::Command {
                            command_name: key.to_string(),
                            content: data_to_send,
                        });
                    }
                    '!' => {
                        let command_line_split = command_line
                            .strip_prefix('!')
                            .unwrap()
                            .split_whitespace()
                            .map(|arg| arg.to_string())
                            .collect::<Vec<_>>();
                        let name = command_line_split[0].to_lowercase();

                        match name.as_str() {
                            "plugin" => {
                                match self
                                    .plugin_manager
                                    .handle_plugin_command(command_line_split[1..].to_vec())
                                {
                                    Ok((cmd, plugin_name)) => {
                                        let mut msg_lut = HashMap::new();
                                        msg_lut.insert("load".to_string(), "Plugin loaded!");
                                        msg_lut.insert("reload".to_string(), "Plugin reloaded!");

                                        let mut text_view = self.text_view.lock().unwrap();
                                        text_view.add_data_out(SerialRxData::Plugin {
                                            timestamp: Local::now(),
                                            plugin_name,
                                            content: msg_lut[&cmd].to_string(),
                                            is_successful: true,
                                        })
                                    }
                                    Err(err_msg) => {
                                        self.set_error_pop_up(err_msg);
                                        return Ok(());
                                    }
                                }
                            }
                            _ => {
                                if let Err(err_msg) = self.plugin_manager.call_plugin_user_command(
                                    &name,
                                    command_line_split[1..].to_vec(),
                                ) {
                                    self.set_error_pop_up(err_msg);
                                    return Ok(());
                                }
                            }
                        }
                    }
                    '$' => {
                        let command_line = command_line
                            .strip_prefix('$')
                            .unwrap()
                            .replace([',', ' '], "")
                            .to_uppercase();

                        let Ok(bytes) = CommandBar::<B>::hex_string_to_bytes(&command_line) else {
                            self.set_error_pop_up(format!("Invalid hex string: {}", command_line));
                            return Ok(());
                        };

                        let interface = self.interface.lock().unwrap();
                        interface.send(UserTxData::HexString { content: bytes });
                    }
                    _ => {
                        let interface = self.interface.lock().unwrap();
                        interface.send(UserTxData::Data {
                            content: command_line,
                        });
                    }
                }

                self.error_pop_up.take();
            }
            _ => {}
        }

        Ok(())
    }

    pub fn update(&mut self, term_size: Rect) -> Result<(), ()> {
        {
            let mut text_view = self.text_view.lock().unwrap();
            text_view.set_frame_height(term_size.height);
            text_view.update_scroll();
        }

        if let Some(error_pop_up) = self.error_pop_up.as_ref() {
            if error_pop_up.is_timeout() {
                self.error_pop_up.take();
            }
        }

        {
            let interface = self.interface.lock().unwrap();
            if let Ok(data_out) = interface.try_recv() {
                let mut text_view = self.text_view.lock().unwrap();
                text_view.add_data_out(data_out.clone());

                self.plugin_manager.call_plugins_serial_rx(data_out);
            }
        }

        let Ok(input_evt) = self.key_receiver.try_recv() else {
            return Ok(());
        };

        match input_evt {
            Key(key) => return self.handle_key_input(key, term_size),
            VerticalScroll(direction) => {
                let mut text_view = self.text_view.lock().unwrap();
                if direction < 0 {
                    text_view.up_scroll();
                } else {
                    text_view.down_scroll();
                }
            }
            HorizontalScroll(direction) => {
                let mut text_view = self.text_view.lock().unwrap();
                if direction < 0 {
                    text_view.left_scroll();
                } else {
                    text_view.right_scroll();
                }
            }
        }

        Ok(())
    }

    fn load_commands(&mut self, filepath: &PathBuf) -> BTreeMap<String, String> {
        let Ok(yaml) = std::fs::read(filepath) else {
            self.set_error_pop_up(format!("Cannot find {:?} filepath", filepath));
            return BTreeMap::new();
        };

        let Ok(yaml_str) = std::str::from_utf8(yaml.as_slice()) else {
            self.set_error_pop_up(format!("The file {:?} has non UTF-8 characters", filepath));
            return BTreeMap::new();
        };

        let Ok(commands) = serde_yaml::from_str(yaml_str) else {
            self.set_error_pop_up(format!(
                "The YAML from {:?} has an incorret format",
                filepath
            ));
            return BTreeMap::new();
        };

        commands
    }
}

enum InputEvent {
    Key(KeyEvent),
    VerticalScroll(i8),
    HorizontalScroll(i8),
}

struct CommandList {
    commands: Vec<String>,
    pattern: String,
}

impl CommandList {
    pub fn new() -> Self {
        Self {
            commands: vec![],
            pattern: String::new(),
        }
    }

    pub fn clear(&mut self) {
        self.commands.clear();
        self.pattern.clear();
    }

    pub fn update_params(&mut self, commands: Vec<String>, pattern: String) {
        self.commands = commands;
        self.pattern = pattern;
    }

    pub fn draw<B: Backend>(&self, f: &mut Frame<B>, command_bar_y: u16, color: Color) {
        if self.commands.is_empty() {
            return;
        }

        let max_commands = min(f.size().height as usize / 2, self.commands.len());
        let mut commands = self.commands[..max_commands].to_vec();
        if commands.len() < self.commands.len() {
            commands.push("...".to_string());
        }

        let longest_command_len = commands
            .iter()
            .fold(0u16, |len, x| max(len, x.chars().count() as u16));
        let area_size = (longest_command_len + 5, commands.len() as u16 + 2);
        let area = Rect::new(
            f.size().x + 2,
            command_bar_y - area_size.1,
            area_size.0,
            area_size.1,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .style(Style::default().fg(color));
        let text = commands
            .iter()
            .map(|x| {
                let is_last =
                    (x == commands.last().unwrap()) && (commands.len() < self.commands.len());

                Spans::from(vec![
                    Span::styled(
                        format!(" {}", if !is_last { &self.pattern } else { "" }),
                        Style::default().fg(color),
                    ),
                    Span::styled(
                        x[self.pattern.len() - 1..].to_string(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }
}
