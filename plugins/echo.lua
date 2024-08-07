local log = require("scope").log
local fmt = require("scope").fmt

local M = {
  data = {
    level = 'info'
  }
}

local function print_with_level(msg, level)
  if level == "debug" then
    log.debug(msg)
  elseif level == "info" then
    log.info(msg)
  elseif level == "success" then
    log.success(msg)
  elseif level == "warning" then
    log.warning(msg)
  elseif level == "error" then
    log.error(msg)
  end
end

function M.on_serial_recv(msg)
  print_with_level(fmt.to_str(msg), M.data.level)
end

--- Set up the level of echo message
--- @param lvl string The level of echo message
function M.level(lvl)
  if not (lvl == "debug" or lvl == "info" or lvl == "success" or lvl == "warning" or lvl == "error") then
    log.error("Level invalid: " .. lvl)
    return
  end

  M.data.level = lvl

  print_with_level("Level setted as " .. M.data.level, lvl)
end

return M
