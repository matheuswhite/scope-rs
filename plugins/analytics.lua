local serial = require("scope").serial
local log = require("scope").log

local M = {
    recv = 0,
    send = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
