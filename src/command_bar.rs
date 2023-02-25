use crate::interface::{DataIn, Interface};
use crate::view::View;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use tui::backend::Backend;
use tui::layout::{Constraint, Direction, Layout};
use tui::style::{Color, Style};
use tui::text::Span;
use tui::widgets::{Block, Borders, Paragraph};
use tui::Frame;

pub struct CommandBar<B: Backend> {
    interface: usize,
    interfaces: Vec<Box<dyn Interface>>,
    view: usize,
    views: Vec<Box<dyn View<Backend = B>>>,
    command_line: String,
    command_filepath: Option<PathBuf>,
    history: Vec<String>,
    key_receiver: Receiver<KeyEvent>,
}

impl<B: Backend> CommandBar<B> {
    const HEIGHT: u16 = 3;

    pub fn draw(&self, f: &mut Frame<B>) {
        let view = self.views[self.view].as_ref();
        let interface = self.interfaces[self.interface].as_ref();

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
                interface.description()
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if interface.is_connected() {
                interface.color()
            } else {
                Color::LightRed
            }));
        let paragraph = Paragraph::new(Span::from(self.command_line.clone())).block(block);
        f.render_widget(paragraph, chunks[1]);
        f.set_cursor(cursor_pos.0, cursor_pos.1);
    }

    pub fn update(&mut self) -> Result<(), ()> {
        let view = self.views[self.view].as_mut();
        let interface = self.interfaces[self.interface].as_ref();

        if let Ok(data_out) = interface.try_recv() {
            view.add_data_out(data_out);
        }

        let Ok(key) = self.key_receiver.try_recv() else {
            return Ok(());
        };

        match key.code {
            KeyCode::Char(c) => self.command_line.push(c),
            KeyCode::Backspace => {
                self.command_line.pop();
            }
            KeyCode::Esc => return Err(()),
            KeyCode::Enter if !self.command_line.is_empty() => {
                let command_line = self.command_line.clone();
                self.command_line.clear();

                match command_line.chars().next().unwrap() {
                    '/' => {
                        let Some(filepath) = &self.command_filepath else {
                            // TODO Show error at command bar
                            return Ok(());
                        };

                        let yaml_content = CommandBar::<B>::load_commands(filepath);
                        let key = command_line.strip_prefix('/').unwrap();

                        if !yaml_content.contains_key(key) {
                            // TODO Show error at command bar
                            return Ok(());
                        }

                        let data_to_send = yaml_content.get(key).unwrap();
                        interface.send(DataIn::Command(key.to_string(), data_to_send.to_string()));
                    }
                    '!' => {
                        match command_line
                            .strip_prefix('!')
                            .unwrap()
                            .to_lowercase()
                            .as_ref()
                        {
                            "clear" | "clean" => view.clear(),
                            "cmds" | "commands" => {
                                // TODO Open pop up with commands
                            }
                            _ => {
                                // TODO Show error at command bar
                            }
                        }
                    }
                    _ => {
                        interface.send(DataIn::Data(command_line));
                    }
                }
            }
            KeyCode::Tab if key.modifiers == KeyModifiers::SHIFT => {
                // TODO Change interface
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
}

impl<B: Backend> CommandBar<B> {
    pub fn new(
        interfaces: Vec<Box<dyn Interface>>,
        views: Vec<Box<dyn View<Backend = B>>>,
    ) -> Self {
        assert!(!interfaces.is_empty(), "Interfaces cannot be empty");
        assert!(!views.is_empty(), "Views cannot be empty");

        let (key_sender, key_receiver) = channel();

        thread::spawn(move || CommandBar::<B>::task(key_sender));

        Self {
            interface: 0,
            interfaces,
            view: 0,
            views,
            command_line: String::new(),
            history: vec![],
            key_receiver,
            command_filepath: None,
        }
    }

    pub fn with_command_file(mut self, filepath: &str) -> Self {
        self.command_filepath = Some(PathBuf::from(filepath));
        self
    }

    fn load_commands(filepath: &PathBuf) -> BTreeMap<String, String> {
        let Ok(yaml) = std::fs::read(filepath) else {
            // TODO Show error at command bar
            return BTreeMap::new();
        };

        let Ok(yaml_str) = std::str::from_utf8(yaml.as_slice()) else {
            // TODO Show error at command bar
            return BTreeMap::new();
        };

        let Ok(commands) = serde_yaml::from_str(yaml_str) else {
            // TODO show error at command bar
            return BTreeMap::new();
        };

        commands
    }

    fn task(sender: Sender<KeyEvent>) {
        loop {
            if let Event::Key(key) = crossterm::event::read().unwrap() {
                sender.send(key).unwrap();
            }
        }
    }
}
