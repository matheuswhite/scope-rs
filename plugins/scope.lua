local M = {
  fmt = {},
  log = {},
  serial = {},
  ble = {},
  sys = {},
  re = {},
}

function M.fmt.to_str(val)
  if type(val) == "table" then
    local bytearr = {}
    for _, v in ipairs(val) do
      local utf8byte = v < 0 and (0xff + v + 1) or v
      table.insert(bytearr, string.char(utf8byte))
    end
    return table.concat(bytearr)
  elseif type(val) == "string" then
    return val
  elseif type(val) == "nil" then
    return "nil"
  else
    return tostring(val)
  end
end

function M.fmt.to_bytes(val)
  if type(val) == "string" then
    return { string.byte(val, 1, -1) }
  elseif type(val) == "table" then
    return val
  else
    return {}
  end
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
  local port, baud_rate = table.unpack(coroutine.yield({":serial.info"}))
  return port, baud_rate
end

function M.serial.send(msg)
  coroutine.yield({":serial.send", msg})
end

function M.serial.recv(opts)
  local err, msg = table.unpack(coroutine.yield({":serial.recv", opts}))
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

function M.re.literal(str)
  return coroutine.yield({":re.literal", str})
end

function M.re.matches(str, pattern_table)
  local fn_name = coroutine.yield({":re.matches", str, pattern_table})
  if fn_name ~= nil then
    local fn = pattern_table[fn_name]
    fn(str)
  end
end

function M.re.match(str, pattern)
  return coroutine.yield({":re.match", str, pattern})
end

return M
