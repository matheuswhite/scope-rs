# Setup Serial Port: change port and/or baud rate at runtime, then release it.
send_line "!serial connect COM4 9600"
send_line "!serial connect 9600"
send_line "!serial connect COM4"
send_line "!serial disconnect"
sleep 1
press Escape
