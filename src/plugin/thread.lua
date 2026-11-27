coroutine.create(function(t)
  local err = require("scope").log.error
  local status, res = pcall(M.{}, t)
  if not status then
    res = tostring(res)
    err(res:match('%[string ".+"%]:(%d+: .+)') or res)
  else
    return res
  end
end)
