local log = require("scope").log
local serial = require("scope").serial

local M = {}

--- Test CREG comamnd
--- @param apn string? The APN to use on CREG command
function M.test_creg(apn)
  apn = apn or "virtueyes.com.br"
  
  serial.send("AT+CREG=1," .. apn .. ",0\r\n")
  local err, rsp = serial.recv({timeout_ms = 200})

  if err then
    log.err("[ERR] Test CREG Timeout")
    return
  end

  if rsp == "OK\r\n" then
    log.inf("[ OK] Test CREG Success")
  else
    log.err("[ERR] Test GREG Fail")
  end
end

--- Run all tests with default parameters
function M.run_all()
  M.test_creg()
end

return M
