local log = require("scope").log
local serial = require("scope").serial
local plugin = require("scope").plugin

local M = plugin.new()

M.serial.on_recv = {
  {
    "AT\r",
    function (_)
      log.inf("Sending msg \"OK\" via serial tx...")
      serial.send("\r\nOK\r\n")
      log.inf("Message sent!")
    end
  },
  {
    "AT+COPS?\r",
    function (_)
      serial.send("+COPS: 0\r\nOK\r\n")
    end
  },
  {
    ".*",
    function (_)
      serial.send("ERROR\r\n")
    end
  }
}

M.cmd.hello = function ()
  log.inf("Hello, World!\r\n")
end

return M

