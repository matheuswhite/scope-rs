local log = require("scope").log
local fmt = require("scope").fmt

local M = {}

function M.on_serial_recv(msg)
  log.info("Received: " .. fmt.to_str(msg))
end

function M.hello()
  log.info("Hello, World!")
end

return M
