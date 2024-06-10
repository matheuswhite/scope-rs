local plugin = require("scope").plugin
local log = require("scope").log
local serial = require("scope").serial

local M = plugin.new()

M.serial.on_recv = function (msg)
  log.dbg('Rx evt: ' .. msg)
end

M.serial.on_send = function (msg)
  log.wrn('Tx evt: ' .. msg)
end

M.cmd.test1 = function (apn)
  serial.send("AT+CREG=1," .. apn .. ",0\r\n")
  local err, rsp = serial.recv({timeout = 200})

  if err ~= 0 then
    log.err("[ERR] Test1 Timeout")
    return
  end

  if rsp == "OK\r\n" then
    log.inf("[ OK] Test1 Success")
  else
    log.err("[ERR] Test1 Fail")
  end
end

return M

