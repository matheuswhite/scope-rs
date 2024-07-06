local M = {
  fmt = {},
  log = {},
  serial = {},
  ble = {},
  sys = {},
}

function M.fmt.to_str(val)
end

function M.fmt.to_bytes(val)
end

function M.log.debug(msg)
  coroutine.yield({":log.debug", msg})
end

function M.log.info(msg)
  coroutine.yield({":log.info", msg})
end

function M.log.success(msg)
  coroutine.yield({":log.success", msg})
end

function M.log.warning(msg)
  coroutine.yield({":log.warning", msg})
end

function M.log.error(msg)
  coroutine.yield({":log.error", msg})
end

function M.serial.info()
  local _, port, baud_rate = coroutine.yield({":serial.info"})
  return port, baud_rate
end

function M.serial.send(msg)
  coroutine.yield({":serial.send", msg})
end

function M.serial.recv(opts)
  local _, err, msg = coroutine.yield({":serial.recv", opts})
  return err, msg
end

function M.sys.os_name()
  if os.getenv("OS") == "Windows_NT" then
    return "windows"
  else
    return "unix"
  end
end

function M.sys.sleep_ms(time)
  coroutine.yield({":sys.sleep", time})
end

return M
