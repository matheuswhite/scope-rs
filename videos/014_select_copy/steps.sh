# Select, Copy and Clear: receive data, drag-select a line (shown in reverse
# video), copy it with Ctrl+C (the box blinks), paste it into the command bar to
# show what was copied, then clear the history with Ctrl+L.
feed "sensor reading: 21.4C\r\n"
sleep 0.4
feed "device id: AB12-CD34\r\n"
sleep 0.4
feed "status: all systems OK\r\n"
sleep 0.7
mouse_drag 4 14 33
sleep 0.7
press C-c 1
sleep 0.8
paste_text "device id: AB12-CD34"
sleep 1.2
press C-l 1
sleep 1
press Escape
