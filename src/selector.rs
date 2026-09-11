//! Icon mode — the interactive picker shown before the TUI (or the headless
//! bridge) starts when `scope serial` / `scope rtt` is launched without its
//! positional arguments, e.g. from a Windows Start-Menu shortcut (issue #230).
//! It lists the serial ports (or asks for an RTT target) so someone who
//! double-clicked an icon can choose what to connect to instead of memorising a
//! command line.
//!
//! It runs on the main thread *before* any task is spawned, owns its own
//! crossterm session (raw mode + alternate screen) and always restores the
//! terminal on the way out through `Tui`'s `Drop`. `main` only enters here when
//! stdin/stdout are a real terminal — piped or headless-scripted runs skip the
//! picker and keep the historical "start disconnected" behaviour.

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
};
use std::io::{self, Stdout};

use crate::interfaces::rtt_if::{ControlBlock, RttSetup};
use crate::list::usb_ports;
use std::path::PathBuf;

/// What the user chose in the picker.
pub enum Outcome<T> {
    /// Connect using these settings.
    Selected(T),
    /// Skip connecting — start the app disconnected (the historical behaviour of
    /// running the subcommand with no positional arguments).
    Skip,
    /// Abort: quit without starting the app.
    Quit,
}

/// Baud rates offered in the list, plus a trailing "Custom…" entry at index
/// `COMMON_BAUDS.len()`.
const COMMON_BAUDS: [u32; 8] = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];
/// 115200 — by far the most common default, pre-selected when none was given.
const DEFAULT_BAUD_INDEX: usize = 4;

/// Probe clocks offered in the RTT list, plus a trailing "Custom…" entry at
/// index `COMMON_SPEEDS.len()`.
const COMMON_SPEEDS: [u32; 8] = [100, 500, 1000, 2000, 4000, 8000, 12000, 20000];
/// 4000 kHz — what `scope rtt` used before the speed was configurable.
const DEFAULT_SPEED_INDEX: usize = 4;

/// How to find the RTT control block, in the order the list shows them. The
/// indices are `ControlBlockChoice`'s discriminants.
const CONTROL_BLOCK_CHOICES: [&str; 3] = [
    "Scan the target's RAM regions  (default, no setup)",
    "Read _SEGGER_RTT from an ELF file",
    "Attach at a fixed address",
];

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested without a terminal)
// ---------------------------------------------------------------------------

/// Move `index` by `delta` within `0..len`, wrapping around the ends. A
/// zero-length list keeps index 0.
fn move_selection(index: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let len = len as isize;
    (((index as isize + delta) % len + len) % len) as usize
}

/// Parse a decimal baud the user typed. Empty, non-numeric or zero is rejected
/// (a baud of 0 never connects).
fn parse_baud(buf: &str) -> Option<u32> {
    buf.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

/// Parse a decimal RTT channel. Empty defaults to channel 0; anything
/// non-numeric is rejected.
fn parse_channel(buf: &str) -> Option<usize> {
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        return Some(0);
    }
    trimmed.parse::<usize>().ok()
}

/// Where the baud list starts: the CLI-provided rate if it matches a common
/// one, otherwise the 115200 default.
fn initial_baud_index(cli_baud: Option<u32>) -> usize {
    cli_baud
        .and_then(|b| COMMON_BAUDS.iter().position(|c| *c == b))
        .unwrap_or(DEFAULT_BAUD_INDEX)
}

/// Parse a probe clock in kHz. Empty, non-numeric or zero is rejected.
fn parse_speed(buf: &str) -> Option<u32> {
    buf.trim().parse::<u32>().ok().filter(|n| *n > 0)
}

/// Where the speed list starts: the CLI-provided clock if it matches a common
/// one, otherwise the 4000 kHz default.
fn initial_speed_index(cli_speed: Option<u32>) -> usize {
    cli_speed
        .and_then(|s| COMMON_SPEEDS.iter().position(|c| *c == s))
        .unwrap_or(DEFAULT_SPEED_INDEX)
}

/// Parse a memory address the user typed, as `0x20200410` or plain decimal.
/// `_` is allowed as a digit separator, so an address can be pasted in the
/// shape a linker map prints it.
///
/// Shared with the `--addr` flag's `value_parser`, so a typo is rejected the
/// same way whether it was typed at the prompt or on the command line.
pub fn parse_address(buf: &str) -> Result<u64, String> {
    let trimmed = buf.trim();
    let (digits, radix) = match trimmed.strip_prefix("0x").or(trimmed.strip_prefix("0X")) {
        Some(hex) => (hex, 16),
        None => (trimmed, 10),
    };
    let digits = digits.replace('_', "");

    if digits.is_empty() {
        return Err("an address is required (e.g. 0x20200410)".to_string());
    }

    u64::from_str_radix(&digits, radix)
        .map_err(|_| format!("`{buf}` is not an address (e.g. 0x20200410)"))
}

/// The step after the channel. The optional steps are skipped when the command
/// line already answered them, so `scope rtt --speed 8000` asks one thing less.
/// `None` means there is nothing left to ask.
fn stage_after_channel(ask_speed: bool, ask_control_block: bool) -> Option<RttStage> {
    if ask_speed {
        Some(RttStage::Speed)
    } else {
        stage_after_speed(ask_control_block)
    }
}

/// The step after the probe speed — see [`stage_after_channel`].
fn stage_after_speed(ask_control_block: bool) -> Option<RttStage> {
    if ask_control_block {
        Some(RttStage::ControlBlock)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Terminal lifecycle
// ---------------------------------------------------------------------------

/// Owns the crossterm session for the picker and restores the terminal on drop,
/// so an early return or a panic can never leave the shell in raw mode.
struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| format!("Cannot enter raw mode: {e}"))?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)
            .map_err(|e| format!("Cannot enter alternate screen: {e}"))?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout))
            .map_err(|e| format!("Cannot create terminal backend: {e}"))?;
        Ok(Self { terminal })
    }

    fn draw(&mut self, render: impl FnOnce(&mut Frame)) -> Result<(), String> {
        self.terminal
            .draw(render)
            .map(|_| ())
            .map_err(|e| format!("Cannot draw picker: {e}"))
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

/// Read the next key press, skipping non-key and key-release events.
fn read_key() -> Result<crossterm::event::KeyEvent, String> {
    loop {
        match event::read().map_err(|e| format!("Cannot read input: {e}"))? {
            Event::Key(key) if key.kind == KeyEventKind::Press => return Ok(key),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Serial picker
// ---------------------------------------------------------------------------

enum SerialStage {
    Port,
    Baud,
    CustomBaud,
}

/// Prompt for a serial port and baud rate. Any argument already supplied on the
/// command line is pre-filled and its step skipped.
pub fn select_serial(
    port: Option<String>,
    baud: Option<u32>,
) -> Result<Outcome<(String, u32)>, String> {
    // Nothing to ask if both are already known.
    if let (Some(port), Some(baud)) = (&port, baud) {
        return Ok(Outcome::Selected((port.clone(), baud)));
    }

    let mut tui = Tui::enter()?;
    serial_loop(&mut tui, port, baud)
}

fn serial_loop(
    tui: &mut Tui,
    cli_port: Option<String>,
    cli_baud: Option<u32>,
) -> Result<Outcome<(String, u32)>, String> {
    let mut ports = usb_ports();
    let mut port_idx = 0usize;
    let mut chosen_port = cli_port.clone();
    let mut baud_idx = initial_baud_index(cli_baud);
    let mut custom_buf = String::new();
    // Start on the baud step when the port came from the CLI (`scope serial COM3`).
    let mut stage = if chosen_port.is_some() {
        SerialStage::Baud
    } else {
        SerialStage::Port
    };

    loop {
        tui.draw(|f| match stage {
            SerialStage::Port => render_port_list(f, &ports, port_idx),
            SerialStage::Baud => {
                render_baud_list(f, chosen_port.as_deref().unwrap_or(""), baud_idx)
            }
            SerialStage::CustomBaud => {
                render_custom_baud(f, chosen_port.as_deref().unwrap_or(""), &custom_buf)
            }
        })?;

        let key = read_key()?;
        if is_ctrl_c(&key) {
            return Ok(Outcome::Quit);
        }

        match stage {
            SerialStage::Port => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    port_idx = move_selection(port_idx, ports.len(), -1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    port_idx = move_selection(port_idx, ports.len(), 1)
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    ports = usb_ports();
                    port_idx = move_selection(port_idx, ports.len(), 0);
                }
                KeyCode::Char('s') | KeyCode::Char('S') => return Ok(Outcome::Skip),
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    return Ok(Outcome::Quit);
                }
                KeyCode::Enter => {
                    if let Some((name, _)) = ports.get(port_idx) {
                        chosen_port = Some(name.clone());
                        stage = SerialStage::Baud;
                    }
                }
                _ => {}
            },
            SerialStage::Baud => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    baud_idx = move_selection(baud_idx, COMMON_BAUDS.len() + 1, -1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    baud_idx = move_selection(baud_idx, COMMON_BAUDS.len() + 1, 1)
                }
                KeyCode::Esc => {
                    // Back to the port list, unless the port was fixed on the CLI.
                    if cli_port.is_some() {
                        return Ok(Outcome::Quit);
                    }
                    stage = SerialStage::Port;
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(Outcome::Quit),
                KeyCode::Enter => {
                    if baud_idx == COMMON_BAUDS.len() {
                        custom_buf.clear();
                        stage = SerialStage::CustomBaud;
                    } else if let Some(port) = chosen_port.clone() {
                        return Ok(Outcome::Selected((port, COMMON_BAUDS[baud_idx])));
                    }
                }
                _ => {}
            },
            SerialStage::CustomBaud => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() && custom_buf.len() < 7 => {
                    custom_buf.push(c)
                }
                KeyCode::Backspace => {
                    custom_buf.pop();
                }
                KeyCode::Esc => stage = SerialStage::Baud,
                KeyCode::Enter => {
                    if let (Some(port), Some(baud)) = (chosen_port.clone(), parse_baud(&custom_buf))
                    {
                        return Ok(Outcome::Selected((port, baud)));
                    }
                }
                _ => {}
            },
        }
    }
}

// ---------------------------------------------------------------------------
// RTT picker
// ---------------------------------------------------------------------------

enum RttStage {
    Target,
    Channel,
    Speed,
    CustomSpeed,
    ControlBlock,
    ElfPath,
    Address,
}

/// Prompt for the RTT settings: the target (chip name) and channel, then the
/// probe speed and where the control block is (issue #248). The target is free
/// text — probe-rs expects a chip name, not something we can enumerate like
/// serial ports — so that one is a text field rather than a list. An empty
/// target starts the app disconnected.
///
/// Anything already given on the command line is kept and its step skipped, so
/// the extra settings cost nothing to whoever does not want them: `Enter`
/// through the two lists takes the previous defaults (4000 kHz, scan).
pub fn select_rtt(setup: RttSetup) -> Result<Outcome<RttSetup>, String> {
    // Target already supplied: nothing to prompt (every other field keeps
    // whatever the command line said, or its default).
    if setup.target.is_some() {
        return Ok(Outcome::Selected(setup));
    }

    let mut tui = Tui::enter()?;
    rtt_loop(&mut tui, setup)
}

fn rtt_loop(tui: &mut Tui, cli: RttSetup) -> Result<Outcome<RttSetup>, String> {
    let ask_speed = cli.probe_speed.is_none();
    let ask_control_block = cli.control_block.is_none();

    let mut target = String::new();
    let mut channel_buf = cli.channel.map(|c| c.to_string()).unwrap_or_default();
    let mut speed_idx = initial_speed_index(cli.probe_speed);
    let mut custom_speed_buf = String::new();
    let mut speed = cli.probe_speed;
    let mut choice_idx = 0usize;
    let mut elf_buf = String::new();
    let mut addr_buf = String::new();
    let mut stage = RttStage::Target;

    /// Assemble the outcome from whatever the loop has gathered.
    macro_rules! selected {
        ($control_block:expr) => {
            Ok(Outcome::Selected(RttSetup {
                target: Some(target.trim().to_string()),
                channel: parse_channel(&channel_buf),
                control_block: $control_block,
                probe_speed: speed,
            }))
        };
    }

    loop {
        tui.draw(|f| match stage {
            RttStage::Target => render_text_prompt(
                f,
                " scope — RTT target ",
                "chip name",
                &target,
                "Enter connect · empty Enter starts disconnected · Esc quit",
            ),
            RttStage::Channel => render_text_prompt(
                f,
                " scope — RTT channel ",
                "channel",
                &channel_buf,
                "Enter next (default 0) · Esc back",
            ),
            RttStage::Speed => render_speed_list(f, &target, speed_idx),
            RttStage::CustomSpeed => render_text_prompt(
                f,
                &format!(" scope — custom probe speed for {target} "),
                "kHz",
                &custom_speed_buf,
                "type digits · Enter confirm · Esc back",
            ),
            RttStage::ControlBlock => render_control_block_list(f, &target, choice_idx),
            RttStage::ElfPath => render_text_prompt(
                f,
                " scope — RTT control block from an ELF ",
                "elf path",
                &elf_buf,
                "Enter connect · Esc back",
            ),
            RttStage::Address => render_text_prompt(
                f,
                " scope — RTT control block address ",
                "address",
                &addr_buf,
                "e.g. 0x20200410 · Enter connect · Esc back",
            ),
        })?;

        let key = read_key()?;
        if is_ctrl_c(&key) {
            return Ok(Outcome::Quit);
        }

        match stage {
            RttStage::Target => match key.code {
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    target.push(c)
                }
                KeyCode::Backspace => {
                    target.pop();
                }
                KeyCode::Esc => return Ok(Outcome::Quit),
                KeyCode::Enter => {
                    if target.trim().is_empty() {
                        return Ok(Outcome::Skip);
                    }
                    stage = RttStage::Channel;
                }
                _ => {}
            },
            RttStage::Channel => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() && channel_buf.len() < 4 => {
                    channel_buf.push(c)
                }
                KeyCode::Backspace => {
                    channel_buf.pop();
                }
                KeyCode::Esc => stage = RttStage::Target,
                KeyCode::Enter => {
                    if parse_channel(&channel_buf).is_some() {
                        match stage_after_channel(ask_speed, ask_control_block) {
                            Some(next) => stage = next,
                            None => return selected!(cli.control_block.clone()),
                        }
                    }
                }
                _ => {}
            },
            RttStage::Speed => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    speed_idx = move_selection(speed_idx, COMMON_SPEEDS.len() + 1, -1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    speed_idx = move_selection(speed_idx, COMMON_SPEEDS.len() + 1, 1)
                }
                KeyCode::Esc => stage = RttStage::Channel,
                KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(Outcome::Quit),
                KeyCode::Enter => {
                    if speed_idx == COMMON_SPEEDS.len() {
                        custom_speed_buf.clear();
                        stage = RttStage::CustomSpeed;
                    } else {
                        speed = Some(COMMON_SPEEDS[speed_idx]);
                        match stage_after_speed(ask_control_block) {
                            Some(next) => stage = next,
                            None => return selected!(cli.control_block.clone()),
                        }
                    }
                }
                _ => {}
            },
            RttStage::CustomSpeed => match key.code {
                KeyCode::Char(c) if c.is_ascii_digit() && custom_speed_buf.len() < 6 => {
                    custom_speed_buf.push(c)
                }
                KeyCode::Backspace => {
                    custom_speed_buf.pop();
                }
                KeyCode::Esc => stage = RttStage::Speed,
                KeyCode::Enter => {
                    if let Some(custom) = parse_speed(&custom_speed_buf) {
                        speed = Some(custom);
                        match stage_after_speed(ask_control_block) {
                            Some(next) => stage = next,
                            None => return selected!(cli.control_block.clone()),
                        }
                    }
                }
                _ => {}
            },
            RttStage::ControlBlock => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    choice_idx = move_selection(choice_idx, CONTROL_BLOCK_CHOICES.len(), -1)
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    choice_idx = move_selection(choice_idx, CONTROL_BLOCK_CHOICES.len(), 1)
                }
                KeyCode::Esc => {
                    stage = if ask_speed {
                        RttStage::Speed
                    } else {
                        RttStage::Channel
                    }
                }
                KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(Outcome::Quit),
                KeyCode::Enter => match choice_idx {
                    1 => {
                        elf_buf.clear();
                        stage = RttStage::ElfPath;
                    }
                    2 => {
                        addr_buf.clear();
                        stage = RttStage::Address;
                    }
                    _ => return selected!(Some(ControlBlock::Scan)),
                },
                _ => {}
            },
            RttStage::ElfPath => match key.code {
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    elf_buf.push(c)
                }
                KeyCode::Backspace => {
                    elf_buf.pop();
                }
                KeyCode::Esc => stage = RttStage::ControlBlock,
                KeyCode::Enter => {
                    let path = elf_buf.trim();
                    if !path.is_empty() {
                        return selected!(Some(ControlBlock::Elf(PathBuf::from(path))));
                    }
                }
                _ => {}
            },
            RttStage::Address => match key.code {
                KeyCode::Char(c) if c.is_ascii_alphanumeric() && addr_buf.len() < 18 => {
                    addr_buf.push(c)
                }
                KeyCode::Backspace => {
                    addr_buf.pop();
                }
                KeyCode::Esc => stage = RttStage::ControlBlock,
                KeyCode::Enter => {
                    if let Ok(address) = parse_address(&addr_buf) {
                        return selected!(Some(ControlBlock::Exact(address)));
                    }
                }
                _ => {}
            },
        }
    }
}

fn is_ctrl_c(key: &crossterm::event::KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c'))
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// A centered rectangle of at most `width`×`height`, clamped to `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// Draw the centered picker box (amber border + `title`) with `hint` on its
/// bottom line, and return the inner rect the caller renders content into.
fn box_with_hint(f: &mut Frame, title: &str, hint: &str, height: u16) -> Rect {
    let area = centered_rect(66, height.clamp(7, 22), f.area());
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .title(Span::styled(
            title,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    f.render_widget(
        Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray)))
            .alignment(Alignment::Center),
        rows[1],
    );
    rows[0]
}

fn highlight_style() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD)
}

fn render_port_list(f: &mut Frame, ports: &[(String, String)], idx: usize) {
    let content = box_with_hint(
        f,
        " scope — select serial port ",
        "↑/↓ move · Enter select · r refresh · s skip · q quit",
        ports.len() as u16 + 4,
    );

    if ports.is_empty() {
        f.render_widget(
            Paragraph::new("No serial ports found. Plug a device and press 'r'.")
                .style(Style::default().fg(Color::Red))
                .alignment(Alignment::Center),
            content,
        );
        return;
    }

    let items: Vec<ListItem> = ports
        .iter()
        .map(|(name, desc)| ListItem::new(format!("{name}  —  {desc}")))
        .collect();
    let list = List::new(items)
        .highlight_style(highlight_style())
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    state.select(Some(idx));
    f.render_stateful_widget(list, content, &mut state);
}

fn render_baud_list(f: &mut Frame, port: &str, idx: usize) {
    let content = box_with_hint(
        f,
        &format!(" scope — baud rate for {port} "),
        "↑/↓ move · Enter select · Esc back · q quit",
        COMMON_BAUDS.len() as u16 + 5,
    );

    let mut items: Vec<ListItem> = COMMON_BAUDS
        .iter()
        .map(|b| ListItem::new(b.to_string()))
        .collect();
    items.push(ListItem::new("Custom…"));
    let list = List::new(items)
        .highlight_style(highlight_style())
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    state.select(Some(idx));
    f.render_stateful_widget(list, content, &mut state);
}

fn render_custom_baud(f: &mut Frame, port: &str, buf: &str) {
    render_text_prompt(
        f,
        &format!(" scope — custom baud for {port} "),
        "baud",
        buf,
        "type digits · Enter confirm · Esc back",
    );
}

fn render_speed_list(f: &mut Frame, target: &str, idx: usize) {
    let content = box_with_hint(
        f,
        &format!(" scope — probe speed for {target} "),
        "↑/↓ move · Enter select · Esc back · q quit",
        COMMON_SPEEDS.len() as u16 + 5,
    );

    let mut items: Vec<ListItem> = COMMON_SPEEDS
        .iter()
        .map(|s| ListItem::new(format!("{s} kHz")))
        .collect();
    items.push(ListItem::new("Custom…"));
    render_list(f, content, items, idx);
}

fn render_control_block_list(f: &mut Frame, target: &str, idx: usize) {
    let content = box_with_hint(
        f,
        &format!(" scope — RTT control block on {target} "),
        "↑/↓ move · Enter select · Esc back · q quit",
        CONTROL_BLOCK_CHOICES.len() as u16 + 5,
    );

    let items = CONTROL_BLOCK_CHOICES
        .iter()
        .map(|choice| ListItem::new(*choice))
        .collect();
    render_list(f, content, items, idx);
}

/// Render `items` into `area` as the picker's highlighted list.
fn render_list(f: &mut Frame, area: Rect, items: Vec<ListItem>, idx: usize) {
    let list = List::new(items)
        .highlight_style(highlight_style())
        .highlight_symbol("▶ ");
    let mut state = ListState::default();
    state.select(Some(idx));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_text_prompt(f: &mut Frame, title: &str, label: &str, buf: &str, hint: &str) {
    let content = box_with_hint(f, title, hint, 5);
    let line = Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::Yellow)),
        Span::raw(buf.to_string()),
        Span::styled("▏", Style::default().fg(Color::Yellow)),
    ]);
    f.render_widget(Paragraph::new(line), content);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_selection_wraps_both_ways() {
        assert_eq!(move_selection(0, 3, -1), 2, "up from top wraps to bottom");
        assert_eq!(move_selection(2, 3, 1), 0, "down from bottom wraps to top");
        assert_eq!(move_selection(1, 3, 1), 2);
        assert_eq!(move_selection(1, 3, -1), 0);
    }

    #[test]
    fn move_selection_handles_empty_and_noop() {
        assert_eq!(move_selection(0, 0, 1), 0, "empty list stays at 0");
        assert_eq!(move_selection(5, 0, -1), 0);
        assert_eq!(move_selection(1, 3, 0), 1, "no delta keeps the index");
        assert_eq!(
            move_selection(9, 3, 0),
            0,
            "out-of-range index is clamped in"
        );
    }

    #[test]
    fn parse_baud_rejects_invalid() {
        assert_eq!(parse_baud("115200"), Some(115200));
        assert_eq!(parse_baud("  9600 "), Some(9600));
        assert_eq!(parse_baud(""), None);
        assert_eq!(parse_baud("0"), None);
        assert_eq!(parse_baud("abc"), None);
        assert_eq!(parse_baud("-1"), None);
    }

    #[test]
    fn parse_channel_defaults_empty_to_zero() {
        assert_eq!(parse_channel(""), Some(0));
        assert_eq!(parse_channel("   "), Some(0));
        assert_eq!(parse_channel("2"), Some(2));
        assert_eq!(parse_channel("x"), None);
    }

    #[test]
    fn parse_speed_rejects_invalid() {
        assert_eq!(parse_speed("4000"), Some(4000));
        assert_eq!(parse_speed(" 8000 "), Some(8000));
        assert_eq!(parse_speed(""), None);
        assert_eq!(parse_speed("0"), None);
        assert_eq!(parse_speed("fast"), None);
    }

    #[test]
    fn initial_speed_index_prefers_matching_cli_value() {
        assert_eq!(initial_speed_index(None), DEFAULT_SPEED_INDEX);
        assert_eq!(initial_speed_index(Some(4000)), DEFAULT_SPEED_INDEX);
        assert_eq!(initial_speed_index(Some(100)), 0);
        assert_eq!(initial_speed_index(Some(20000)), COMMON_SPEEDS.len() - 1);
        // A clock not in the common list falls back to the default.
        assert_eq!(initial_speed_index(Some(3333)), DEFAULT_SPEED_INDEX);
    }

    // Shared with the `--addr` flag, so these cases pin the CLI too (issue #248).
    #[test]
    fn parse_address_reads_hex_and_decimal() {
        assert_eq!(parse_address("0x20200410"), Ok(0x2020_0410));
        assert_eq!(parse_address("0X20200410"), Ok(0x2020_0410));
        // A linker map prints addresses without the prefix.
        assert_eq!(parse_address("20200410"), Ok(20_200_410));
        assert_eq!(parse_address(" 0x2020_0410 "), Ok(0x2020_0410));
        assert_eq!(parse_address("0"), Ok(0));
    }

    #[test]
    fn parse_address_rejects_what_is_not_an_address() {
        for bad in [
            "",
            "   ",
            "0x",
            "0xzz",
            "nope",
            "-1",
            "0x1_0000_0000_0000_0000",
        ] {
            assert!(parse_address(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn the_optional_steps_are_skipped_when_the_cli_answered_them() {
        // Nothing given: ask both, in order.
        assert!(matches!(
            stage_after_channel(true, true),
            Some(RttStage::Speed)
        ));
        assert!(matches!(
            stage_after_speed(true),
            Some(RttStage::ControlBlock)
        ));

        // `--speed` given: straight to the control block.
        assert!(matches!(
            stage_after_channel(false, true),
            Some(RttStage::ControlBlock)
        ));
        // `--addr`/`--elf` given: the speed is still asked, then it is done.
        assert!(matches!(
            stage_after_channel(true, false),
            Some(RttStage::Speed)
        ));
        assert!(stage_after_speed(false).is_none());
        // Both given: the picker only asks for target and channel.
        assert!(stage_after_channel(false, false).is_none());
    }

    #[test]
    fn initial_baud_index_prefers_matching_cli_value() {
        assert_eq!(initial_baud_index(None), DEFAULT_BAUD_INDEX);
        assert_eq!(initial_baud_index(Some(115200)), 4);
        assert_eq!(initial_baud_index(Some(9600)), 0);
        // A baud not in the common list falls back to the default.
        assert_eq!(initial_baud_index(Some(12345)), DEFAULT_BAUD_INDEX);
    }
}
