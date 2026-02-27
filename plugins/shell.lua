local M = {}

function M.run(cmd)
    local res = coroutine.yield({ ":shell.run", cmd })
    return res.stdout, res.stderr
end

function M.exist(program)
    local res = coroutine.yield({ ":shell.exist", program })
    return res.exist
end

return M
