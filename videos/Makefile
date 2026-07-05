# Regenerate the README demo GIFs.
#
# Each demo lives in NNN_name/steps.sh and is recorded headlessly by record.sh
# (socat virtual serial port + tmux + asciinema + agg) — no focused window or
# real hardware needed. Build the release binary first (cargo build --release).
#
#   make            # (re)record every demo
#   make 006_ansi   # record a single demo
#   make clean      # remove generated casts and gifs
#
# Requires: socat, tmux, asciinema, agg.

SHELL := /bin/bash
DEMOS := $(sort $(patsubst %/steps.sh,%,$(wildcard */steps.sh)))

.PHONY: all clean $(DEMOS)

all: $(DEMOS)

$(DEMOS):
	./record.sh $@

clean:
	rm -f */video.cast */video.gif
