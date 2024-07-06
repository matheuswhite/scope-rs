local log = require("scope").log

local M = {
  data = {
    level = 'info'
  }
}

function M.on_serial_recv(msg)
  if M.data.level == "debug" then
    log.debug(msg)
  elseif M.data.level == "info" then
    log.info(msg)
  elseif M.data.level == "success" then
    log.success(msg)
  elseif M.data.level == "warning" then
    log.warning(msg)
  elseif M.data.level == "error" then
    log.error(msg)
  end
end

--- Set up the level of echo message
--- @param lvl string The level of echo message
function M.level(lvl)
  if not (lvl == "debug" or lvl == "info" or lvl == "success" or lvl == "warning" or lvl == "error") then
    return
  end
    
  M.data.level = level
end

return M
