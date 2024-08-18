use crate::blink_color::BlinkColor;
use crate::command_bar::InputEvent::{HorizontalScroll, Key, VerticalScroll};
use crate::error_pop_up::ErrorPopUp;
use crate::messages::{SerialRxData, UserTxData};
use crate::plugin_manager::PluginManager;
use crate::serial::SerialIF;
use crate::text::TextView;
use chrono::Local;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use futures::StreamExt;
use rand::seq::SliceRandom;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;
use std::cmp::{max, min};
use std::collections::btree_map::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::sleep;

pub struct CommandBar {
    interface: Arc<Mutex<SerialIF>>,
    text_view: Arc<Mutex<TextView>>,
    command_line: String,
    command_line_idx: usize,
    command_filepath: Option<PathBuf>,
    history: Vec<String>,
    history_index: Option<usize>,
    backup_command_line: String,
    error_pop_up: Option<ErrorPopUp>,
    command_list: CommandList,
    key_receiver: UnboundedReceiver<InputEvent>,
    current_hint: Option<&'static str>,
    hints: Vec<&'static str>,
    plugin_manager: PluginManager,
    blink_color: BlinkColor,
}

impl CommandBar {
    const HEIGHT: u16 = 3;

    pub fn new(interface: SerialIF, view_capacity: usize, save_filename: String) -> Self {
        let (key_sender, key_receiver) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            CommandBar::task(key_sender).await;
        });

        let hints = vec![
            "Type / to send a command",
            "Type $ to start a hex sequence",
            "Type here and hit <Enter> to send",
        ];

        let interface = Arc::new(Mutex::new(interface));
        let text_view = Arc::new(Mutex::new(TextView::new(view_capacity, save_filename)));

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
            blink_color: BlinkColor::new(Color::Black, Duration::from_millis(200), 2),
        }
    }

    pub fn with_command_file(mut self, filepath: &str) -> Self {
        self.command_filepath = Some(PathBuf::from(filepath));
        self
    }

    async fn task(sender: UnboundedSender<InputEvent>) {
        let mut reader = EventStream::new();

        loop {
            let event = reader.next().await;

            match event {
                Some(Ok(event)) => match event {
                    Event::Mouse(mouse_evt) if mouse_evt.modifiers == KeyModifiers::CONTROL => {
                        match mouse_evt.kind {
                            MouseEventKind::ScrollUp => sender.send(HorizontalScroll(-1)).unwrap(),
                            MouseEventKind::ScrollDown => sender.send(HorizontalScroll(1)).unwrap(),
                            _ => {}
                        }
                    }
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        sender.send(Key(key)).unwrap()
                    }
                    Event::Mouse(mouse_evt) => match mouse_evt.kind {
                        MouseEventKind::ScrollUp => sender.send(VerticalScroll(-1)).unwrap(),
                        MouseEventKind::ScrollDown => sender.send(VerticalScroll(1)).unwrap(),
                        _ => {}
                    },
                    _ => {}
                },
                Some(Err(e)) => panic!("Error at command bar task: {:?}", e),
                None => break,
            }
        }
    }

    pub async fn get_text_view(&self) -> MutexGuard<TextView> {
        self.text_view.lock().await
    }

    pub async fn get_interface(&self) -> MutexGuard<SerialIF> {
        self.interface.lock().await
    }

    pub fn draw(&self, f: &mut Frame<'_>, text_view: &TextView, interface: &SerialIF) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(f.size().height - CommandBar::HEIGHT),
                    Constraint::Length(CommandBar::HEIGHT),
                ]
                .as_ref(),
            )
            .split(f.size());

        text_view.draw(f, chunks[0], self.blink_color.get_color());

        let (description, is_connected) = (interface.description(), interface.is_connected());

        let cursor_pos = (
            chunks[1].x + self.command_line_idx as u16 + 2,
            chunks[1].y + 1,
        );
        let bar_color = if self.plugin_manager.has_process_running() {
            Color::DarkGray
        } else if is_connected {
            Color::LightGreen
        } else {
            Color::LightRed
        };
        let block = Block::default()
            .title(format!("[{:03}] {}", self.history.len(), description))
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(bar_color));
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

    async fn handle_key_input(&mut self, key: KeyEvent, _term_size: Rect) -> Result<(), ()> {
        match key.code {
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => {
                let mut text_view = self.text_view.lock().await;
                text_view.clear()
            }
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => {
                self.error_pop_up.take();
            }
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.plugin_manager.stop_process().await;
            }
            KeyCode::Char('s') if key.modifiers == KeyModifiers::CONTROL => {
                let is_recording = {
                    let mut text_view = self.text_view.lock().await;
                    text_view.get_mut_recorder().is_recording()
                };

                if is_recording {
                    self.set_error_pop_up("Cannot save file while recording.".to_string());
                    return Ok(());
                }

                self.blink_color.start();
                let mut text_view = self.text_view.lock().await;
                let typewriter = text_view.get_mut_typewriter();
                let filename = typewriter.get_filename();
                let save_result = typewriter.flush().await;
                text_view
                    .add_data_out(SerialRxData::Plugin {
                        plugin_name: "SAVE".to_string(),
                        timestamp: Local::now(),
                        content: if let Err(err) = &save_result {
                            format!("Cannot save on \"{}\": {}", filename, err)
                        } else {
                            format!("Content saved on \"{}\"", filename)
                        },
                        is_successful: save_result.is_ok(),
                    })
                    .await;
            }
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                let mut text_view = self.text_view.lock().await;
                let recorder = text_view.get_mut_recorder();
                let filename = recorder.get_filename();
                let record_msg = if !recorder.is_recording() {
                    let record_result = recorder.start_record().await;
                    SerialRxData::Plugin {
                        plugin_name: "REC".to_string(),
                        timestamp: Local::now(),
                        content: if let Err(err) = &record_result {
                            format!(
                                "Cannot start content recording on \"{}\": {}",
                                filename, err
                            )
                        } else {
                            format!("Recording content on \"{}\"...", filename)
                        },
                        is_successful: record_result.is_ok(),
                    }
                } else {
                    recorder.stop_record();
                    SerialRxData::Plugin {
                        plugin_name: "REC".to_string(),
                        timestamp: Local::now(),
                        content: format!("Content recorded on \"{}\"", filename),
                        is_successful: true,
                    }
                };
                text_view.add_data_out(record_msg).await;
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
            KeyCode::PageUp if key.modifiers == KeyModifiers::CONTROL => {
                self.text_view.lock().await.scroll_to_start();
            }
            KeyCode::PageUp => self.text_view.lock().await.page_up(),
            KeyCode::PageDown if key.modifiers == KeyModifiers::CONTROL => {
                self.text_view.lock().await.scroll_to_end();
            }
            KeyCode::PageDown => self.text_view.lock().await.page_down(),
            KeyCode::Backspace => {
                if self.command_line.chars().count() == 1 {
                    self.show_hint();
                }

                if self.command_line_idx > 0 {
                    self.command_line_idx -= 1;
                    self.command_line = self
                        .command_line
                        .chars()
                        .enumerate()
                        .filter_map(|(i, c)| {
                            if i != self.command_line_idx {
                                Some(c)
                            } else {
                                None
                            }
                        })
                        .collect();
                    self.update_command_list();
                }

                if self.command_line.chars().count() > 0
                    && self.command_line.chars().all(|x| x.is_whitespace())
                {
                    self.command_line.clear();
                    self.command_line_idx = 0;
                    self.show_hint();
                }
            }
            KeyCode::Delete => {
                if self.command_line.chars().count() == 0 {
                    self.show_hint();
                }

                self.command_line = self
                    .command_line
                    .chars()
                    .enumerate()
                    .filter_map(|(i, c)| {
                        if i != self.command_line_idx {
                            Some(c)
                        } else {
                            None
                        }
                    })
                    .collect();
                self.update_command_list();

                if self.command_line.chars().count() > 0
                    && self.command_line.chars().all(|x| x.is_whitespace())
                {
                    self.command_line.clear();
                    self.command_line_idx = 0;
                    self.show_hint();
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
                        self.backup_command_line.clone_from(&self.command_line);
                    }
                    Some(0) => {}
                    Some(idx) => {
                        *idx -= 1;
                    }
                }

                self.clear_hint();
                self.command_line
                    .clone_from(&self.history[self.history_index.unwrap()]);
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
                        self.command_line.clone_from(&self.backup_command_line);
                        if self.command_line.is_empty() {
                            self.show_hint();
                        }
                    }
                    Some(idx) => {
                        *idx += 1;
                        self.command_line.clone_from(&self.history[*idx]);
                    }
                }

                self.command_line_idx = self.command_line.chars().count();
                self.update_command_list();
            }
            KeyCode::End => {
                self.command_line_idx = self.command_line.chars().count();
            }
            KeyCode::Home => {
                self.command_line_idx = 0;
            }
            KeyCode::Esc => {
                let interface = self.interface.lock().await;
                interface.exit().await;
                sleep(Duration::from_millis(100)).await;
                return Err(());
            }
            KeyCode::Enter if !self.command_line.is_empty() => {
                if self.plugin_manager.has_process_running() {
                    self.set_error_pop_up(
                        "Cannot send data or command while a command is running".to_string(),
                    );
                    return Ok(());
                }

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
                        let interface = self.interface.lock().await;
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
                        if command_line_split.is_empty() {
                            let interface = self.interface.lock().await;
                            interface.send(UserTxData::Data {
                                content: command_line,
                            });
                            return Ok(());
                        }

                        let name = command_line_split[0].to_lowercase();

                        match name.as_str() {
                            "plugin" => {
                                match self
                                    .plugin_manager
                                    .handle_plugin_command(command_line_split[1..].to_vec())
                                {
                                    Ok(plugin_name) => {
                                        let mut text_view = self.text_view.lock().await;
                                        text_view
                                            .add_data_out(SerialRxData::Plugin {
                                                timestamp: Local::now(),
                                                plugin_name: plugin_name.clone(),
                                                content: format!(
                                                    "Plugin \"{}\" loaded!",
                                                    plugin_name
                                                ),
                                                is_successful: true,
                                            })
                                            .await;
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

                        let Ok(bytes) = CommandBar::hex_string_to_bytes(&command_line) else {
                            self.set_error_pop_up(format!("Invalid hex string: {}", command_line));
                            return Ok(());
                        };

                        let interface = self.interface.lock().await;
                        interface.send(UserTxData::HexString { content: bytes });
                    }
                    _ => {
                        let interface = self.interface.lock().await;
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

    pub async fn update(&mut self, term_size: Rect) -> Result<(), ()> {
        {
            let mut text_view = self.text_view.lock().await;
            text_view.set_frame_height(term_size.height);
            text_view.update_scroll();
        }

        if let Some(error_pop_up) = self.error_pop_up.as_ref() {
            if error_pop_up.is_timeout() {
                self.error_pop_up.take();
            }
        }

        self.blink_color.update();

        {
            let mut interface = self.interface.lock().await;
            if let Ok(data_out) = interface.try_recv() {
                let mut text_view = self.text_view.lock().await;
                text_view.add_data_out(data_out.clone()).await;

                self.plugin_manager.call_plugins_serial_rx(data_out);
            }
        }

        let Ok(input_evt) = self.key_receiver.try_recv() else {
            return Ok(());
        };

        match input_evt {
            Key(key) => return self.handle_key_input(key, term_size).await,
            VerticalScroll(direction) => {
                let mut text_view = self.text_view.lock().await;
                if direction < 0 {
                    text_view.up_scroll();
                } else {
                    text_view.down_scroll();
                }
            }
            HorizontalScroll(direction) => {
                let mut text_view = self.text_view.lock().await;
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

    pub fn draw(&self, f: &mut Frame, command_bar_y: u16, color: Color) {
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

                Line::from(vec![
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
