local scope = require("scope")

local check_zephyr_base = function (sh)
  local stdout, _ = sh:run("echo $ZEPHYR_BASE")
  return stdout and stdout ~= "" and stdout ~= "\n"
end

local west = {}

west.load = function ()
  west.shell = scope.sys.shell()

  local res = check_zephyr_base(west.shell)
  if not res then
    scope.log.err("$ZEPHYR_BASE is empty")
  end

  west.shell:run("source $ZEPHYR_BASE/../.venv/bin/activate")

  return res
end

west.action.build = function ()
  west.shell:run("west build")
end

west.action.flash = function ()
  local info = scope.serial.info()

  scope.serial.disconnect()
  west.shell:run("west flash")
  scope.serial.connect(info.port, info.baudrate)
end

return west

