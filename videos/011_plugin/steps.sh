# Plugins: load a Lua plugin, invoke one of its commands (`!echo hello` →
# logs "Hello, World!"), and let its `on_serial_recv` hook prefix received data.
send_line "!plugin load plugins/echo.lua"
send_line "!echo hello"
feed "Ping\r\n"
sleep 0.9
feed "Pong\r\n"
sleep 0.9
send_line "Hello"
sleep 1
press Escape
