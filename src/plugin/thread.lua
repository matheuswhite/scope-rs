coroutine.create(function(t)
  local err = require("scope").log.error
  local status, res = pcall(M.{}, t)
  if not status then
    err(res:match('%[string ".+"%]:(%d+: .+)'))
  else
    return res
  end
end)
