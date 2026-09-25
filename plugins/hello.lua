local serial = require('scope').serial

local M = {}

function M.on_serial_recv_line(msg)
    serial.send("Hello," .. msg)
end

return M
