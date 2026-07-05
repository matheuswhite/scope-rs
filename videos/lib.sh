# Shared helpers for the README demo recordings.
#
# Sourced by record.sh with these globals already set:
#   SESSION   tmux session the scope TUI runs in (drive it with `tmux send-keys`)
#   PORT_OUT  far end of the virtual serial port (write bytes here → scope RX)
#   WORK      scratch dir holding the COM1 / COM1_out socat links
#
# Input is injected with `tmux send-keys` instead of an OS-level keystroke
# library, so recording is fully headless and reproducible — no focused window
# or accessibility permission required.

# Pacing (seconds). Tuned to read comfortably in a looping GIF.
TYPE_SPEED=${TYPE_SPEED:-0.11}   # delay between typed characters
WAIT_MSG=${WAIT_MSG:-0.30}       # pause after typing, before Enter
WAIT_END=${WAIT_END:-0.65}       # pause after Enter, before the next action

# Type a string one character at a time (so the GIF shows it being typed).
# `-l` sends every character literally, so `$ ! @ - ,` etc. are not read as
# tmux key names.
type_str() {
    local s="$1" i
    for ((i = 0; i < ${#s}; i++)); do
        tmux send-keys -t "$SESSION" -l "${s:$i:1}"
        sleep "$TYPE_SPEED"
    done
}

# Type a line and press Enter (the scope "send message" gesture).
send_line() {
    type_str "$1"
    sleep "$WAIT_MSG"
    tmux send-keys -t "$SESSION" Enter
    sleep "$WAIT_END"
}

# Press a named key once (e.g. Up, Escape, C-s), with an optional pause after.
press() {
    tmux send-keys -t "$SESSION" "$1"
    sleep "${2:-$TYPE_SPEED}"
}

# Press a named key several times, then Enter (used to walk the input history).
repeat_key() {
    local key="$1" times="$2" i
    for ((i = 0; i < times; i++)); do
        tmux send-keys -t "$SESSION" "$key"
        sleep "$TYPE_SPEED"
    done
    sleep "$WAIT_MSG"
    tmux send-keys -t "$SESSION" Enter
    sleep "$WAIT_END"
}

# Feed one already-escaped payload to scope's RX (printf %b interprets \xNN).
feed() {
    printf '%b' "$1" >"$PORT_OUT"
}

# Stream `Hello, World!` lines colored with random ANSI foreground colors.
# $1 = number of lines.
ansi_feed() {
    local lines="$1" i j color msg
    local colors=('\x1b[31m' '\x1b[32m' '\x1b[33m' '\x1b[34m' '\x1b[35m' '\x1b[36m' '\x1b[37m')
    for ((i = 0; i < lines; i++)); do
        sleep 0.5
        msg=""
        for ((j = 0; j < 3; j++)); do
            color="${colors[$((RANDOM % 7))]}"
            msg+="${color}Hello, World!\x1b[0m "
        done
        feed "${msg}\r\n"
    done
}

# Stream a line mixing printable text, non-printable high bytes (shown magenta
# in hex) and a NUL (shown as its `\0` representation). $1 = number of lines.
invisibles_feed() {
    local lines="$1" i
    for ((i = 0; i < lines; i++)); do
        sleep 0.5
        # "World" with each byte shifted by +0x7E → 0xD5 0xED 0xF0 0xEA 0xE2.
        feed 'Hello, \xd5\xed\xf0\xea\xe2 \x00Again\r\n'
    done
}

# Bring the virtual serial port down / back up, recreating the SAME links so
# scope's auto-reconnect picks it up again (used by the reconnect demo).
kill_socat() {
    if [ -f "$WORK/socat.pid" ]; then
        kill "$(cat "$WORK/socat.pid")" 2>/dev/null || true
        rm -f "$WORK/socat.pid"
    fi
    # Remove the links too: otherwise scope reopens the (now dead) PTY symlink
    # and may never notice the drop, so the demo wouldn't show a disconnect.
    rm -f "$WORK/COM1" "$WORK/COM1_out"
}

spawn_socat() {
    rm -f "$WORK/COM1" "$WORK/COM1_out"
    # `exec` so the backgrounded job *is* socat — then $! is socat's real PID and
    # kill_socat actually terminates it (scope detects the drop by the PTY dying,
    # not by the link disappearing).
    (cd "$WORK" && exec socat PTY,link=COM1,raw,echo=0 PTY,link=COM1_out,raw,echo=0 >/dev/null 2>&1) &
    echo $! >"$WORK/socat.pid"
    until [ -e "$WORK/COM1" ] && [ -e "$WORK/COM1_out" ]; do sleep 0.05; done
}
