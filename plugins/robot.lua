local robot_status = require("robot_status")
local scope = require("scope")
local rtt = scope.rtt
local log = scope.log
local fmt = scope.fmt

local plugin = {
    status_address = nil
}

function plugin.on_rtt_recv(msg)
    local msg_str = fmt.to_str(msg)
    local msg_start = msg_str:sub(1, 15)

    if msg_start == "Status Address:" then
        plugin.status_address = msg_str:match("Status Address: 0x(%x+)")
        log.info("Received status address: " .. plugin.status_address)
    end
end

function plugin.status()
    if plugin.status_address == nil then
        log.error("Status address not set")
        return
    end

    log.info("Reading struct from address: " .. plugin.status_address)
    local address = tonumber(plugin.status_address, 16)

    local err, data = rtt.read({ address = address, size = robot_status.size() })
    if err then
        log.error("Failed to read struct: " .. fmt.to_str(err))
        return
    end

    local status = robot_status.decode(data)
    log.info("Status: " .. tostring(status))
end

return plugin
