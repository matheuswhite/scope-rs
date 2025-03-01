local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re
local shell = require("shell")

local M = {
    recv = 0,
    send = 0,
    at = 0,
    at_plus = 0,
    number = 0,
    shell = nil,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.shell = shell.new()

    M.shell:run("echo Hello")

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

    re.matches(msg,
        "AT.*", function(msg)
            M.at = M.at + 1
        end,
        re.literal("AT+"), function(msg)
            M.at_plus = M.at_plus + 1
        end,
        "\\d+", function(msg)
            M.number = M.number + 1
        end
    )

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
