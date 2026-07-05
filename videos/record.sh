#!/usr/bin/env bash
#
# Record one README demo into <demo>/video.cast and <demo>/video.gif.
#
#   ./record.sh 001_send_data
#
# The pipeline is fully headless: a socat PTY pair stands in for the serial
# port, scope runs inside a detached tmux session under `asciinema rec`, the
# demo's steps.sh drives it with `tmux send-keys`, and `agg` turns the finished
# cast into a GIF. Requires: socat, tmux, asciinema, agg.
set -euo pipefail

DEMO="${1:?usage: record.sh <demo_dir>}"
DEMO="${DEMO%/}"

HERE="$(cd "$(dirname "$0")" && pwd)"
STEPS="$HERE/$DEMO/steps.sh"
[ -f "$STEPS" ] || { echo "no such demo: $HERE/$DEMO/steps.sh" >&2; exit 1; }

# Match the look of the previous recordings: 116x28 xterm-256color.
COLS=${COLS:-116}
ROWS=${ROWS:-28}
SCOPE_BIN="${SCOPE_BIN:-$HERE/../target/release/scope}"
[ -x "$SCOPE_BIN" ] || { echo "scope binary not found at $SCOPE_BIN (run: cargo build --release)" >&2; exit 1; }

for tool in socat tmux asciinema agg; do
    command -v "$tool" >/dev/null || { echo "missing required tool: $tool" >&2; exit 1; }
done

CAST="$HERE/$DEMO/video.cast"
GIF="$HERE/$DEMO/video.gif"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/scope_rec.XXXXXX")"
SESSION="scope_rec_$$"

cleanup() {
    tmux kill-session -t "$SESSION" 2>/dev/null || true
    [ -f "$WORK/socat.pid" ] && kill "$(cat "$WORK/socat.pid")" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

export SESSION PORT_OUT="$WORK/COM1_out" WORK
# shellcheck source=lib.sh
source "$HERE/lib.sh"

spawn_socat

# Make the demo plugins loadable via the same relative path the README shows
# (`!plugin load plugins/echo.lua`), since scope runs with cwd = $WORK.
[ -d "$HERE/plugins" ] && cp -R "$HERE/plugins" "$WORK/plugins"

# Per-demo assets (e.g. a tags.yml or a file to `!send_file`) land in the
# working dir so a demo can reference them by their bare name.
[ -d "$HERE/$DEMO/assets" ] && cp -R "$HERE/$DEMO/assets/." "$WORK/"

# Optional per-demo options: a demo may set SCOPE_ARGS (extra global flags
# passed before the `serial` subcommand, e.g. `-l 100000` to slow the poll loop
# so a file transfer's progress is visible).
SCOPE_ARGS=""
# shellcheck source=/dev/null
[ -f "$HERE/$DEMO/opts.env" ] && source "$HERE/$DEMO/opts.env"

# scope runs with cwd = $WORK and port arg "COM1", so the title bar reads "COM1"
# (the link lives in $WORK). asciinema stops when scope exits, writing the cast.
rm -f "$CAST"
tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
    "cd '$WORK' && TERM=xterm-256color asciinema rec --overwrite -c '$SCOPE_BIN $SCOPE_ARGS serial COM1 0' '$CAST'"

# Wait for scope to come up and connect before driving it.
sleep 2

# shellcheck source=/dev/null
source "$STEPS"

# Make sure scope has quit so asciinema flushes the cast even if a demo forgot.
tmux send-keys -t "$SESSION" Escape 2>/dev/null || true
for _ in $(seq 1 50); do
    tmux has-session -t "$SESSION" 2>/dev/null || break
    sleep 0.2
done

# A session still alive means scope never exited: asciinema hasn't flushed a
# complete cast, so fail loudly rather than render a truncated GIF.
if tmux has-session -t "$SESSION" 2>/dev/null; then
    echo "scope did not exit after Escape — demo likely broken, cast is incomplete" >&2
    exit 1
fi

[ -f "$CAST" ] || { echo "recording failed: no cast produced" >&2; exit 1; }
# Cap long idle stretches so the loop stays snappy without dropping the
# deliberate pauses in the demos.
agg --idle-time-limit 2 "$CAST" "$GIF"
echo "wrote $GIF"
