local log = require("scope").log
local serial = require("scope").serial
local shell = require("shell")

local M = {}

function M.on_load()
  M.shell = shell.new()

  if not M.shell:exist("west") then
    log.err("west not found. Export it before enter in Scope")
    return false
  end

  return true
end

--- Build the firmware
function M.build()
  M.shell:run("west build")
end

--- Flash the firmware
function M.flash()
  local info = serial.info()

  serial.disconnect()
  M.shell:run("west flash")
  serial.connect(info.port, info.baudrate)
end

return M
