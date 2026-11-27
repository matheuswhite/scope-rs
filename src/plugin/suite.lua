coroutine.create(function(tests)
  local scope = require("scope")
  local status, res = pcall(scope.test._run_suite, M, tests)
  if not status then
    scope.log.error(tostring(res))
  end
end)
