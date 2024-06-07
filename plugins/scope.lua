local Shell = require("shell.Shell")

local scope = {}

scope.bytes.to_str = function (bytes)
  local str = ""

  for _, byte in pairs(bytes) do
    str = str .. string.char(byte)
  end

  return str
end


scope.string.to_bytes = function (str)
  local bytes = {}

  for _, c in utf8.codes(str) do
    table.insert(bytes, c)
  end

  return bytes
end

scope.log = {
  dbg = function (msg)
    coroutine.yield({":log.dbg", msg})
  end,
  inf = function (msg)
    coroutine.yield({":log.inf", msg})
  end,
  wrn = function (msg)
    coroutine.yield({":log.wrn", msg})
  end,
  err = function (msg)
    coroutine.yield({":log.err", msg})
  end,
}

scope.serial = {
  info = function ()
    local _, port, baud_rate = coroutine.yield({":serial.info"})
    return port, baud_rate
  end,
  send = function (msg)
    coroutine.yield({":serial.send", msg})
  end,
  recv = function (timeout_ms)
    local _, err, msg = coroutine.yield({":serial.recv", timeout_ms})
    return err, msg
  end,
}

scope.sys = {
  os = function ()
    if os.getenv("OS") == "Windows_NT" then
      return "windows"
    else
      return "unix"
    end
  end,
  sleep = function (time_ms)
    coroutine.yield({":sys.sleep", time_ms})
  end,
  shell = function ()
    local _, id = coroutine.yield({":sys.shell"})
    return Shell:new(id)
  end,
}

return scope

