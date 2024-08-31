local log = require("scope").log
local serial = require("scope").serial
local shell = require("shell")

local M = {}

function M.on_load()
  M.shell = shell.new()

  if not M.shell:exist("idf.py") then
    log.err("There isn't a command called idf.py. Export it before enter in Scope")
    return false
  end

  return true
end

--- Build the firmware
function M.build()
  M.shell:run("idf.py build")
end

--- Flash the firmware
--- @param port string? The board port
function M.flash(port)
  local cmd = "west flash"
  if port then
    cmd = cmd .. " -p " .. port
  end

  local info = serial.info()

  serial.disconnect()
  M.shell:run(cmd)
  serial.connect(info.port, info.baudrate)
end

return M
