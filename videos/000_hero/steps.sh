# Hero demo: a compact tour of scope for the top of the README. A short
# firmware-session narrative that touches the highlights without dragging on:
# colored RX with timestamps, non-printable bytes, sending plain commands and
# @tags, search, auto-reconnect and saving. Kept tight so it reads well as a
# looping GIF.

# 1. The device boots: colored status lines stream in (ANSI colors, each with a
#    gray timestamp on the left).
feed '\x1b[32mboot:\x1b[0m firmware v1.4.2 starting\r\n'
sleep 0.5
feed '\x1b[36mnet:\x1b[0m link up, dhcp lease acquired\r\n'
sleep 0.5
feed '\x1b[33msensor:\x1b[0m temperature = 21.4C\r\n'
sleep 0.5
# A frame with non-printable bytes: high bytes show magenta in hex, NUL as \0.
feed 'raw frame: \xd5\xed\xf0 \x00 ok\r\n'
sleep 0.5
feed '\x1b[33msensor:\x1b[0m humidity = 48%\r\n'
sleep 0.7

# 2. Send a plain command; the device answers.
send_line "AT"
feed '\x1b[32mOK\x1b[0m\r\n'
sleep 0.6

# 3. Send a value from the tag file: type @re, autocomplete with Tab, send.
type_str "@re"
sleep 0.4
press Tab 0.6
sleep 0.3
tmux send-keys -t "$SESSION" Enter
sleep "$WAIT_END"
feed '\x1b[33msensor:\x1b[0m reset done\r\n'
sleep 0.5
feed '\x1b[33msensor:\x1b[0m pressure = 1013 hPa\r\n'
sleep 0.7

# 4. Search the log with Ctrl+F, walk the matches, then leave search mode.
press C-f 0.6
type_str "sensor"
sleep 0.8
press Down 0.9
press Down 0.9
press Up 0.9
press Escape 0.7

# 5. Auto-reconnect: drop the port (bar turns red) and bring it back (bar green),
#    then the device resumes sending.
kill_socat
sleep 3
spawn_socat
sleep 3
feed '\x1b[36mnet:\x1b[0m reconnected\r\n'
sleep 0.8

# 6. Save the whole session to a .txt file (the history box blinks).
press C-s 1.2
sleep 1

press Escape
