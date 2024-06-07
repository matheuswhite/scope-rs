local scope = require("scope")

local M = scope.plugin.new({"event.serial", "cmd"})

M.event.serial.rx = function (msg)

end

M.event.serial.tx = function (msg)

end

M.event.serial.connect = function (port, baudrate)

end

M.event.serial.disconnect = function ()

end

M.cmd.hello = function (arg1, arg2)

end

M.cmd.world = function ()

end

return M

