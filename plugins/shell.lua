local M = {}

function M.run(cmd)
    local stdout, stderr = coroutine.yield({ ":shell.run", cmd })
    return stdout, stderr
end

function M.exist(program)
    local res = coroutine.yield({ ":shell.exist", program })
    return res
end

return M
