use crate::interface::{DataIn, Interface};
use crate::view::View;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use tui::backend::Backend;
use tui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use tui::style::{Color, Style};
use tui::text::Span;
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

struct ErrorPopUp<B: Backend> {
    message: String,
    spwan_time: Instant,
    _marker: PhantomData<B>,
}

impl<B: Backend> ErrorPopUp<B> {
    const TIMEOUT: Duration = Duration::from_millis(5000);

    pub fn new(message: String) -> Self {
        Self {
            message,
            _marker: PhantomData,
            spwan_time: Instant::now(),
        }
    }
}

impl<B: Backend> ErrorPopUp<B> {
    pub fn draw(&self, f: &mut Frame<B>, command_bar_y: u16) {
        let area_size = (self.message.chars().count() as u16 + 4, 3);
        let area = Rect::new(
            (f.size().width - area_size.0) / 2,
            command_bar_y - area_size.1 + 1,
            area_size.0,
            area_size.1,
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::LightRed));
        let paragraph = Paragraph::new(Span::from(self.message.clone()))
            .block(block)
            .alignment(Alignment::Center);
        f.render_widget(Clear, area);
        f.render_widget(paragraph, area);
    }

    pub fn is_timeout(&self) -> bool {
        self.spwan_time.elapsed() >= ErrorPopUp::<B>::TIMEOUT
    }
}
