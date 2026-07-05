# Search: fill the history with data, then Ctrl+F to search and navigate the
# matches with Down (next) and Up (previous); Esc leaves search mode.
feed "boot: system starting\r\n"
sleep 0.3
feed "sensor: temperature = 21.4C\r\n"
sleep 0.3
feed "net: link up\r\n"
sleep 0.3
feed "sensor: humidity = 48%\r\n"
sleep 0.3
feed "net: dhcp lease acquired\r\n"
sleep 0.3
feed "sensor: pressure = 1013 hPa\r\n"
sleep 0.3
feed "app: ready\r\n"
sleep 0.3
feed "sensor: temperature = 21.7C\r\n"
sleep 0.6
# enter search and look for every "sensor" line
press C-f 0.6
type_str "sensor"
sleep 0.8
press Down 0.9
press Down 0.9
press Down 0.9
press Up 0.9
press Escape 0.6
sleep 0.6
press Escape
