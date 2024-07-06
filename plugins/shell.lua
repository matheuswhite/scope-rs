local Shell = {
  pid = nil,
}

function Shell.new()
  local _, pid = coroutine.yield({":Shell.new"})
  local self = setmetatable({}, Shell)
  self.pid = pid

  return self
end

function Shell:run(cmd, opts)
  local _, stdout, stderr = coroutine.yield({":Shell:run", self, cmd, opts})
  return stdout, stderr
end

function Shell:exist(program)
  local _, res = coroutine.yield({"Shell:run", self, program})
  return res
end

return Shell
