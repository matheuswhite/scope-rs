use crate::error_pop_up::ErrorPopUp;
use crate::interface::{DataIn, Interface};
use crate::view::View;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::cmp::{max, min};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::{Span, Spans};
use tui::widgets::{Block, Borders, Clear, Paragraph};
use tui::Frame;

pub struct CommandBar<B: Backend> {
    interface: Box<dyn Interface>,
    view: usize,
    views: Vec<Box<dyn View<Backend = B>>>,
    command_line: String,
    command_filepath: Option<PathBuf>,
    history: Vec<String>,
    error_pop_up: Option<ErrorPopUp<B>>,
    command_list: CommandList,
    key_receiver: Receiver<KeyEvent>,
}

impl<B: Backend + Send> CommandBar<B> {
    const HEIGHT: u16 = 3;

    pub fn draw(&self, f: &mut Frame<B>) {
        let view = self.views[self.view].as_ref();

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

        view.draw(f, chunks[0]);

        let cursor_pos = (
            chunks[1].x + self.command_line.chars().count() as u16 + 1,
            chunks[1].y + 1,
        );
        let block = Block::default()
            .title(format!(
                "[{:03}] {}",
                self.history.len(),
                self.interface.description()
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if self.interface.is_connected() {
                self.interface.color()
            } else {
                Color::LightRed
            }));
        let paragraph = Paragraph::new(Span::from(self.command_line.clone())).block(block);
        f.render_widget(paragraph, chunks[1]);
        f.set_cursor(cursor_pos.0, cursor_pos.1);

        self.command_list
            .draw(f, chunks[1].y, self.interface.color());

        if let Some(pop_up) = self.error_pop_up.as_ref() {
            pop_up.draw(f, chunks[1].y);
        }
    }

    fn clear_views(&mut self) {
        for view in self.views.iter_mut() {
            view.clear();
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

        // TODO Load YAML file at start
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

    pub fn update(&mut self) -> Result<(), ()> {
        if let Some(error_pop_up) = self.error_pop_up.as_ref() {
            if error_pop_up.is_timeout() {
                self.error_pop_up.take();
            }
        }

        if let Ok(data_out) = self.interface.try_recv() {
            for view in self.views.iter_mut() {
                view.add_data_out(data_out.clone());
            }
        }

        let Ok(key) = self.key_receiver.try_recv() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Char('l') if key.modifiers == KeyModifiers::CONTROL => self.clear_views(),
            KeyCode::Char('q') if key.modifiers == KeyModifiers::CONTROL => {
                self.error_pop_up.take();
            }
            KeyCode::Char(c) => {
                self.command_line.push(c);
                self.update_command_list();
            }
            KeyCode::Backspace => {
                self.command_line.pop();
                self.update_command_list();
            }
            KeyCode::Esc => return Err(()),
            KeyCode::Enter if !self.command_line.is_empty() => {
                let command_line = self.command_line.clone();
                self.command_line.clear();
                self.command_list.clear();

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
                            self.set_error_pop_up(format!("Command </{key}> not found"));
                            return Ok(());
                        }

                        let data_to_send = yaml_content.get(key).unwrap();
                        self.interface
                            .send(DataIn::Command(key.to_string(), data_to_send.to_string()));
                    }
                    '!' => {
                        match command_line
                            .strip_prefix('!')
                            .unwrap()
                            .to_lowercase()
                            .as_ref()
                        {
                            "clear" | "clean" => self.clear_views(),
                            "cmds" | "commands" => {
                                // TODO Open pop up with commands
                            }
                            _ => {
                                self.set_error_pop_up(format!(
                                    "Command <!{command_line}> not found"
                                ));
                            }
                        }
                    }
                    _ => {
                        self.interface.send(DataIn::Data(command_line));
                    }
                }

                self.error_pop_up.take();
            }
            KeyCode::Tab if key.modifiers == KeyModifiers::SHIFT => {
                // TODO Change view mode
            }
            KeyCode::Tab => {
                if self.view == self.views.len() - 1 {
                    self.view = 0;
                } else {
                    self.view += 1;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn load_commands(&mut self, filepath: &PathBuf) -> BTreeMap<String, String> {
        let Ok(yaml) = std::fs::read(filepath) else {
            self.set_error_pop_up(format!("Cannot find {filepath:?} filepath"));
            return BTreeMap::new();
        };

        let Ok(yaml_str) = std::str::from_utf8(yaml.as_slice()) else {
            self.set_error_pop_up(format!("The file {filepath:?} has non UTF-8 characters"));
            return BTreeMap::new();
        };

        let Ok(commands) = serde_yaml::from_str(yaml_str) else {
            self.set_error_pop_up(format!("The YAML from {filepath:?} has an incorret format"));
            return BTreeMap::new();
        };

        commands
    }
}

impl<B: Backend> CommandBar<B> {
    pub fn new(interface: Box<dyn Interface>, views: Vec<Box<dyn View<Backend = B>>>) -> Self {
        assert!(!views.is_empty(), "Views cannot be empty");

        let (key_sender, key_receiver) = channel();

        thread::spawn(move || CommandBar::<B>::task(key_sender));

        Self {
            interface,
            view: 0,
            views,
            command_line: String::new(),
            history: vec![],
            key_receiver,
            error_pop_up: None,
            command_filepath: None,
            command_list: CommandList::new(),
        }
    }

    pub fn with_command_file(mut self, filepath: &str) -> Self {
        self.command_filepath = Some(PathBuf::from(filepath));
        self
    }

    fn task(sender: Sender<KeyEvent>) {
        loop {
            if let Event::Key(key) = crossterm::event::read().unwrap() {
                sender.send(key).unwrap();
            }
        }
    }
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
}

impl CommandList {
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
                        Style::default().fg(Color::White),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        let paragraph = Paragraph::new(text).block(block);
        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }
}
