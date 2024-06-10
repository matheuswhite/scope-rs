local plugin = require("scope").plugin
local log = require("scope").log
local serial = require("serial").serial
local shell = require("shell")

local check_zephyr_base = function (sh)
  local stdout, _ = sh:run("echo $ZEPHYR_BASE")
  return stdout and stdout ~= "" and stdout ~= "\n"
end

local M = plugin.new()

M.on_load = function ()
  M.shell = shell.new()

  local res = check_zephyr_base(M.shell)
  if not res then
    log.err("$ZEPHYR_BASE is empty")
  end

  M.shell:run("source $ZEPHYR_BASE/../.venv/bin/activate")

  return res
end

M.cmd.build = function ()
  M.shell:run("west build")
end

M.cmd.flash = function ()
  local info = serial.info()

  serial.disconnect()
  M.shell:run("west flash")
  serial.connect(info.port, info.baudrate)
end

return M

