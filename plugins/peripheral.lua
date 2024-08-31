local log = require("scope").plugin
local serial = require("scope").plugin
local sys = require("scope").sys
local re = require("scope").re

local M = {
  tries = 1,
  success = "OK\r"
}

function M.serial_on_recv(msg)
  re.matches(msg, {
    ["AT\r"] = function (_)
      serial.send("OK\r\n")
    end,
    [re.literal("AT+COPS?")] = function (_)
      sys.sleep(1000)
      serial.send("+COPS: 0\r\n")
      serial.send("OK\r\n")
    end,
    [".*"] = function (msg)
      serial.send("+CMERR: Invalid " .. msg .. "\r\n")
      serial.send("ERROR\r\n")
    end
  })
end

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

return M
