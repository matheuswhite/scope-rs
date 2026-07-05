# Select, Copy and Clear: receive some data, drag-select a line (shown in
# reverse video), copy it with Ctrl+C (the box blinks), then clear with Ctrl+L.
feed "The quick brown fox\r\n"
sleep 0.5
feed "jumps over the lazy dog\r\n"
sleep 0.5
feed "0123456789 ABCDEF GHIJ\r\n"
sleep 0.7
mouse_drag 4 14 40
sleep 0.6
press C-c 1
sleep 1
press C-l 1
sleep 1
press Escape
