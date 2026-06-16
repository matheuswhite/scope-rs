---
name: test-tui
description: Run and verify the scope serial-monitor TUI end-to-end without real hardware — drive keystrokes, send/receive data over a virtual serial port, and inspect the visual layout/colors. Use when asked to manually test, verify a fix, or check the TUI's behavior or appearance.
---

# Test the scope TUI end-to-end

> **Prefer the automated tests first.** Most of this procedure is encoded as Rust
> integration tests in `tests/tui_e2e.rs` — run `cargo test --test tui_e2e`. They
> use `openpty` for the virtual serial port, `portable-pty` to run the app in a real
> terminal, and a `vt100` parser as the screen-capture equivalent. Use the manual
> `socat`/`tmux` flow below for exploratory/visual debugging or when you need to *see*
> the live TUI (e.g. colors, layout, interactive feel).

`scope` is a serial-monitor TUI: it reads keystrokes in raw terminal mode and sends/receives bytes over a serial port. To test it without hardware, pair three tools:

- **`socat`** — creates a virtual serial port (a PTY pair) so there is "a serial port" to connect to and a far end to read/write.
- **`tmux`** — runs the TUI in a real (detached) terminal and injects keystrokes via `send-keys`, which a piped stdin cannot do for a raw-mode app.
- **`tmux capture-pane`** — reads back what is on screen, as plain text (`-p`) or with ANSI color escapes (`-ep`).

## Prerequisites

```bash
which socat tmux   # both required; install with: brew install socat tmux
cargo build --bin scope
```

## 1. Create a virtual serial port

```bash
rm -f /tmp/scope_a /tmp/scope_b
socat -d -d PTY,raw,echo=0,link=/tmp/scope_a PTY,raw,echo=0,link=/tmp/scope_b >/tmp/socat.log 2>&1 &
echo $! > /tmp/socat.pid
until [ -e /tmp/scope_a ] && [ -e /tmp/scope_b ]; do :; done   # wait for the links
```

`/tmp/scope_a` is the end scope connects to; `/tmp/scope_b` is the far end you read from / write to (acting as the device).

## 2. Launch scope in a detached tmux session

```bash
# Optional: a tag file to exercise the @tag feature
cat > /tmp/scope_tags.yaml <<'EOF'
tag1: hello
tag2: world
EOF

tmux kill-session -t scopetest 2>/dev/null; sleep 0.3
tmux new-session -d -s scopetest -x 160 -y 40 \
  "./target/debug/scope -t /tmp/scope_tags.yaml serial /tmp/scope_a 115200"
sleep 1.5
tmux capture-pane -t scopetest -p   # should show the title bar, output area and input bar
```

`-x`/`-y` set the virtual terminal size; pick wide enough that lines are not truncated.

## 3. Drive interaction (send keystrokes)

Use `send-keys -l` (literal) so special characters like `$` are typed verbatim and not interpreted as tmux key names. Send `Enter` as a named key.

```bash
send_line () {                       # type a string then press Enter
  tmux send-keys -t scopetest -l "$1"
  sleep 0.3; tmux send-keys -t scopetest Enter; sleep 0.7
}

send_line '$01 $02'                  # hex sequence
send_line '@tag1@tag2'               # tags
```

Other useful keys: `BSpace` (backspace, to clear input), `Up`/`Down` (history), `Escape`.

## 4. Verify what was SENT (the TX path)

The TUI's TX log line renders the exact bytes placed in the send buffer, so it is authoritative for verifying parsing/encoding (hex `$..`, tags `@..`, escapes):

```bash
tmux capture-pane -t scopetest -p | grep '\\x'   # e.g. "13:50:16 \x01\x02\r\n"
```

To also capture the raw bytes on the wire, read the far end of the serial pair **with `socat`, not `cat`**:

```bash
socat -u /tmp/scope_b,raw,echo=0 CREATE:/tmp/scope_capture.bin >/tmp/reader.log 2>&1 &
echo $! > /tmp/reader.pid
# ... then send from the TUI ...
sleep 0.6; sync; xxd /tmp/scope_capture.bin
```

> **Gotchas**
> - `cat /tmp/scope_b > file` block-buffers and looks empty — use `socat ... CREATE:file` instead.
> - On **macOS the `serialport` crate cannot set baud rate via ioctl on a PTY**, so scope's writes may never reach the wire even though the TUI logs them. When the on-wire capture is empty for this reason, rely on the TUI's TX log render, which reflects the post-parse bytes. (A direct shell write `printf '\x41' > /tmp/scope_a` reaching the reader confirms the socat bridge itself works.)

## 5. Verify what was RECEIVED (the RX path)

Write bytes into the far end and confirm they appear in scope's output area:

```bash
printf 'ping\r\n' > /tmp/scope_b
sleep 0.5
tmux capture-pane -t scopetest -p | grep ping
```

## 6. Inspect the VISUAL

Plain text layout (title bar, output area, autocomplete popups, input bar):

```bash
tmux send-keys -t scopetest -l '@ta'; sleep 0.5   # trigger tag autocomplete
tmux capture-pane -t scopetest -p
```

Colors / highlighting — capture with escapes and make them visible. Special chars (`\x01`, `\r\n`) and timestamps are color-coded:

```bash
tmux capture-pane -t scopetest -ep | sed -n '2p' | cat -v
# e.g. ^[[38;5;8m<timestamp>^[[39m ^[[38;5;5m\x01\x02^[[...  -> gray time, magenta special chars
```

## 7. Clean up (always)

```bash
tmux kill-session -t scopetest 2>/dev/null
kill $(cat /tmp/reader.pid) 2>/dev/null
kill $(cat /tmp/socat.pid) 2>/dev/null
rm -f /tmp/scope_a /tmp/scope_b /tmp/scope_capture.bin \
      /tmp/socat.log /tmp/socat.pid /tmp/reader.log /tmp/reader.pid /tmp/scope_tags.yaml
# scope may drop a log file (20YYMMDD_*.txt) or .scope_history in the cwd — remove if unwanted
```

## Notes

- Subcommands: `scope serial [PORT] [BAUDRATE]`, also `ble`, `rtt`, `list`. Global opts: `-t/--tag-file`, `-c/--capacity`, `-l/--latency`.
- Add small `sleep`s after each action so the TUI redraws before you capture.
- The input bar does not colorize hex/tag sequences while typing (by design); parsing and highlighting happen on send and render in the output area.
