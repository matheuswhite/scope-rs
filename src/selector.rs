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

use crate::list::usb_ports;

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
}

/// Prompt for an RTT target (chip name) and channel. The target is free text —
/// probe-rs expects a chip name, not something we can enumerate like serial
/// ports — so this is a text field rather than a list. An empty target starts
/// the app disconnected.
pub fn select_rtt(
    target: Option<String>,
    channel: Option<usize>,
) -> Result<Outcome<(String, usize)>, String> {
    // Target already supplied: nothing to prompt (the channel defaults to 0).
    if let Some(target) = target {
        return Ok(Outcome::Selected((target, channel.unwrap_or(0))));
    }

    let mut tui = Tui::enter()?;
    rtt_loop(&mut tui, channel)
}

fn rtt_loop(tui: &mut Tui, cli_channel: Option<usize>) -> Result<Outcome<(String, usize)>, String> {
    let mut target = String::new();
    let mut channel_buf = cli_channel.map(|c| c.to_string()).unwrap_or_default();
    let mut stage = RttStage::Target;

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
                "Enter connect (default 0) · Esc back",
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
                    if let Some(channel) = parse_channel(&channel_buf) {
                        return Ok(Outcome::Selected((target.trim().to_string(), channel)));
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
    fn initial_baud_index_prefers_matching_cli_value() {
        assert_eq!(initial_baud_index(None), DEFAULT_BAUD_INDEX);
        assert_eq!(initial_baud_index(Some(115200)), 4);
        assert_eq!(initial_baud_index(Some(9600)), 0);
        // A baud not in the common list falls back to the default.
        assert_eq!(initial_baud_index(Some(12345)), DEFAULT_BAUD_INDEX);
    }
}
