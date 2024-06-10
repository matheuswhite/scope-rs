local M = {
  plugin = {},
  bytes = {},
  str = {},
  log = {},
  serial = {},
  ble = {},
  sys = {},
}

M.plugin.new = function ()
  return {
    evt = {
      serial = {},
      ble = {},
    },
    cmd = {},
  }
end

M.bytes.to_str = function (bytes)
  local str = ""

  for _, byte in pairs(bytes) do
    str = str .. string.char(byte)
  end

  return str
end


M.str.to_bytes = function (str)
  local bytes = {}

  for _, c in utf8.codes(str) do
    table.insert(bytes, c)
  end

  return bytes
end

M.log.dbg = function (msg)
  coroutine.yield({":log.dbg", msg})
end

M.log.inf = function (msg)
  coroutine.yield({":log.inf", msg})
end

M.log.wrn = function (msg)
  coroutine.yield({":log.wrn", msg})
end

M.log.err = function (msg)
  coroutine.yield({":log.err", msg})
end

M.serial.info = function ()
    local _, port, baud_rate = coroutine.yield({":serial.info"})
    return port, baud_rate
  end

M.serial.send = function (msg)
  coroutine.yield({":serial.send", msg})
end

M.serial.recv = function (timeout_ms)
  local _, err, msg = coroutine.yield({":serial.recv", timeout_ms})
  return err, msg
end

M.sys.os = function ()
  if os.getenv("OS") == "Windows_NT" then
    return "windows"
  else
    return "unix"
  end
end

M.sys.sleep = function (time_ms)
  coroutine.yield({":sys.sleep", time_ms})
end

return M

