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
  local _, stdout, stderr = coroutine.yield({":Shell:run", cmd, opts})
  return stdout, stderr
end

return Shell

