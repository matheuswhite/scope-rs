# Auto Reconnect: drop the port (bar turns red, "Disconnected" is logged), then
# bring it back and watch scope reconnect on its own and resume sending.
kill_socat
sleep 4
send_line "Hello"
sleep 1
spawn_socat
sleep 4
send_line "Hello"
send_line "hello"
sleep 1
press Escape
