local plugin = require("scope").plugin
local log = require("scope").plugin
local serial = require("scope").plugin
local sys = require("scope").sys

local M = plugin.new()
M.tries = 1
M.success = "OK\r"

M.serial.on_recv = {
  {
    "AT\r",
    function (_)
      serial.send("OK\r\n")
    end,
  },
  {
    "AT+COPS?\r",
    function (_)
      sys.sleep(1000)
      serial.send("+COPS: 0\r\n")
      serial.send("OK\r\n")
    end,
  },
  {
    ".*",
    function (msg)
      serial.send("+CMERR: Invalid " .. msg .. "\r\n")
      serial.send("ERROR\r\n")
    end
  },
}

local function ord_ends(n)
  if n == 1 then
    return "st"
  elseif n == 2 then
    return "nd"
  elseif n == 3 then
    return "rd"
  else
    return "th"
  end
end

M.serial.on_send = function (msg)
  local err, rsp = serial.recv({timeout = 200})
  if not err and rsp == M.success then
    return
  end

  local tries = M.tries
  for i=1,tries do
    sys.sleep_ms(50)
    log.wrn(tostring(i) .. ord_ends(i) .. " try fail")
    log.wrn("Trying to send \"" .. msg .. "\" again...")
    serial.send(msg)

    err, rsp = serial.recv({timeout = 200})
    if not err and rsp == "OK\r" then
      break
    end
  end
end

return M

