local log = require("scope").log
local fmt = require("scope").fmt
local serial = require("scope").serial
local sys = require("scope").sys

local M = {
  is_loaded = false
}

function M.on_load()
  log.info("Running on " .. sys.os_name())
  sys.sleep_ms(3000)
  log.success("Plugin is ready now!")
  M.is_loaded = true
end

function M.on_unload()
  log.warning("Cleaning up resources...")
  sys.sleep_ms(2000)
  log.success("Resources clean")
  M.is_loaded = false
end

function M.on_serial_send(msg)
  if not M.is_loaded then
    log.error("Plugin not loaded yet")
    return
  end

  log.info("Sending " .. fmt.to_str(msg) .. " ...")
  serial.send(fmt.to_bytes("AT\r\n"))
  local _, data = serial.recv({timeout_ms=2000})
  log.info("Receive sent message: " .. fmt.to_str(data))
  local _, data = serial.recv({timeout_ms=2000})
  log.info("Receive AT: " .. fmt.to_str(data))
end

function M.on_serial_recv(msg)
  log.info("Receive pkt: " .. fmt.to_str(msg))
end

function M.on_serial_connect(port, baudrate)
  log.success("Connected to " .. port .. "@" .. fmt.to_str(baudrate))
end

function M.on_serial_disconnect(port, baudrate)
  log.warning("Disconnected from " .. port .. "@" .. fmt.to_str(baudrate))
end

function M.hello(name, age)
  name = name or "???"
  age = age or "???"
  log.info("Hello, " .. name .. ". Do you have " .. age .. " years?")
end

function M.logs()
  log.debug("debug log level")
  log.info("info log level")
  log.success("success log level")
  log.warning("warning log level")
  log.error("error log level")
end

return M
