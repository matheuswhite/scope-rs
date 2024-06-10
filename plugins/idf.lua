local plugin = require("scope").plugin
local log = require("scope").log
local serial = require("scope").serial
local shell = require("shell")

local M = plugin.new()

M.on_load = function ()
  M.shell = shell.new()

  if not M.shell:exist("idf.py") then
    log.err("There isn't a command called idf.py. Export it before enter in scope")
    return
  end
end

M.cmd.build = function ()
  M.shell:run("idf.py build")
end

--- comment
--- @param port string?
M.cmd.flash = function (port)
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

