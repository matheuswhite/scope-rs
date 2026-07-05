# README demo recordings

The GIFs shown in the top-level [README](../README.md) are generated here, fully
headlessly — no real serial hardware and no focused-window keystroke injection.

## How it works

Each demo is a directory `NNN_name/` containing a `steps.sh` script. Recording a
demo (`record.sh NNN_name`) does the following:

1. `socat` creates a virtual serial port pair (`COM1` ⇄ `COM1_out`).
2. `scope` runs inside a detached `tmux` session, under `asciinema rec`, and
   connects to `COM1`.
3. `steps.sh` drives the session: keystrokes are injected with `tmux send-keys`
   and received data is written to `COM1_out` (helpers live in `lib.sh`).
4. When the demo quits scope, `asciinema` writes `video.cast` and `agg` renders
   it to `video.gif`.

Because input is injected through `tmux` instead of an OS-level keyboard library,
the whole thing runs unattended and reproducibly.

## Requirements

- [`socat`](http://www.dest-unreach.org/socat/)
- [`tmux`](https://github.com/tmux/tmux)
- [`asciinema`](https://asciinema.org/)
- [`agg`](https://github.com/asciinema/agg)
- a release build of scope: `cargo build --release`

## Usage

```shell
make               # (re)record every demo
make 006_ansi      # record a single demo
make clean         # remove generated casts and gifs
./record.sh 006_ansi   # equivalent to `make 006_ansi`
```

## Adding a demo

Create `NNN_name/steps.sh` and drive the session with the helpers from `lib.sh`
(`send_line`, `press`, `repeat_key`, `feed`, `ansi_feed`, `invisibles_feed`,
`kill_socat`, `spawn_socat`). End with `press Escape` to quit scope so the
recording stops. Reference the resulting `NNN_name/video.gif` from the README.
