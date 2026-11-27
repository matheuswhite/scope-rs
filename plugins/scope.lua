local M = {
    fmt = {},
    log = {},
    serial = {},
    ble = {},
    sys = {},
    re = {},
    rtt = {},
    codec = {
        at = require("codec.at")
    }
}

local CONFIG_DEFAULTS = {
    recv_as_string = false,
    default_timeout = 1000,
    default_expected = "OK",
    line_ending = "\r\n",
    ignore_msg = {},
}

function M._init_config(plugin)
    if plugin.config == nil then
        plugin.config = {}
    end

    if type(plugin.config) ~= "table" then
        error("M.config must be a table, not " .. type(plugin.config), 0)
    end

    for key, value in pairs(CONFIG_DEFAULTS) do
        if plugin.config[key] == nil then
            plugin.config[key] = type(value) == "table" and {} or value
        end
    end
end

local function config(key)
    local plugin = rawget(_G, "M")

    if type(plugin) == "table" and type(plugin.config) == "table" and plugin.config[key] ~= nil then
        return plugin.config[key]
    end

    return CONFIG_DEFAULTS[key]
end

local function recv_data(data)
    if config("recv_as_string") then
        return M.fmt.to_str(data)
    end

    return data
end

local function bytes_arg(fn_name, msg, level)
    if type(msg) ~= "string" and type(msg) ~= "table" then
        error(fn_name .. ": the message must be a string or a list of bytes, not " .. type(msg), level + 1)
    end

    return msg
end

function M.fmt.to_str(val)
    if type(val) == "table" then
        local bytearr = {}
        for _, v in ipairs(val) do
            local utf8byte = v < 0 and (0xff + v + 1) or v
            table.insert(bytearr, string.char(utf8byte))
        end
        return table.concat(bytearr)
    elseif type(val) == "string" then
        return val
    elseif type(val) == "nil" then
        return "nil"
    else
        return tostring(val)
    end
end

function M.fmt.to_bytes(val)
    if type(val) == "string" then
        return { string.byte(val, 1, -1) }
    elseif type(val) == "table" then
        return val
    else
        return {}
    end
end

function M.fmt.split(str, sep, max)
    if type(str) == "table" then
        str = M.fmt.to_str(str)
    end
    assert(type(str) == "string", "fmt.split: the input must be a string or a list of bytes")
    assert(type(sep) == "string" and sep ~= "", "fmt.split: the separator must be a non-empty string")
    assert(max == nil or (math.type(max) == "integer" and max >= 1), "fmt.split: max must be a positive integer")

    local parts = {}
    local start = 1

    while max == nil or #parts < max - 1 do
        local first, last = string.find(str, sep, start, true)
        if first == nil then
            break
        end

        table.insert(parts, string.sub(str, start, first - 1))
        start = last + 1
    end
    table.insert(parts, string.sub(str, start))

    return table.unpack(parts)
end

function M.log.debug(msg)
    coroutine.yield({ ":log.debug", msg })
end

function M.log.info(msg)
    coroutine.yield({ ":log.info", msg })
end

function M.log.success(msg)
    coroutine.yield({ ":log.success", msg })
end

function M.log.warning(msg)
    coroutine.yield({ ":log.warning", msg })
end

function M.log.error(msg)
    coroutine.yield({ ":log.error", msg })
end

function M.serial.info()
    local res = coroutine.yield({ ":serial.info" })
    return res.port, res.baud_rate
end

function M.serial.send(msg)
    coroutine.yield({ ":serial.send", bytes_arg("serial.send", msg, 2) })
end

function M.serial.recv(opts)
    local res = coroutine.yield({ ":serial.recv", opts })
    return res.err, recv_data(res.data)
end

local function send_and_wait(interface, msg, expected, timeout_ms)
    local fn_name = interface .. ".send_and_wait"
    local text = M.fmt.to_str(bytes_arg(fn_name, msg, 3))

    local prefix = config("send_prefix")
    if prefix ~= nil and prefix ~= "" then
        text = prefix .. (prefix:match("%s$") and "" or " ") .. text
    end
    text = text .. config("line_ending")

    local opts = {
        message = text,
        expected = expected or config("default_expected"),
        timeout_ms = timeout_ms or config("default_timeout"),
        error_pattern = config("error_pattern"),
        ignore = config("ignore_msg"),
    }

    if type(opts.expected) ~= "string" then
        error(fn_name .. ": the expected message must be a string pattern", 3)
    end
    if type(opts.timeout_ms) ~= "number" or opts.timeout_ms < 0 then
        error(fn_name .. ": the timeout must be a number of milliseconds", 3)
    end
    if opts.error_pattern ~= nil and type(opts.error_pattern) ~= "string" then
        error(fn_name .. ": config.error_pattern must be a string pattern", 3)
    end
    if type(opts.ignore) ~= "table" then
        error(fn_name .. ": config.ignore_msg must be a list of string patterns", 3)
    end

    local res = coroutine.yield({ ":" .. fn_name, opts })
    if res.err ~= nil then
        return nil, res.err
    end

    return res.lines, res.matched
end

function M.serial.send_and_wait(msg, expected, timeout_ms)
    local lines, matched = send_and_wait("serial", msg, expected, timeout_ms)
    return lines, matched
end

function M.rtt.info()
    local res = coroutine.yield({ ":rtt.info" })
    return res.target, res.channel
end

function M.rtt.send(msg)
    coroutine.yield({ ":rtt.send", bytes_arg("rtt.send", msg, 2) })
end

function M.rtt.recv(opts)
    local res = coroutine.yield({ ":rtt.recv", opts })
    return res.err, recv_data(res.data)
end

function M.rtt.send_and_wait(msg, expected, timeout_ms)
    local lines, matched = send_and_wait("rtt", msg, expected, timeout_ms)
    return lines, matched
end

function M.rtt.read(opts)
    local res = coroutine.yield({ ":rtt.read", opts })
    return res.err, res.data
end

function M.sys.os_name()
    if os.getenv("OS") == "Windows_NT" then
        return "windows"
    else
        return "unix"
    end
end

function M.sys.sleep_ms(time)
    coroutine.yield({ ":sys.sleep", time })
end

local function ord(idx)
    local rem = idx % 10
    if rem == 1 then
        return tostring(idx) .. "st"
    elseif rem == 2 then
        return tostring(idx) .. "nd"
    elseif rem == 3 then
        return tostring(idx) .. "rd"
    else
        return tostring(idx) .. "th"
    end
end

local function parse_arg(idx, arg, ty, validate, default)
    assert(not (ty == "nil" or ty == "function" or ty == "userdata" or ty == "thread" or ty == "table"),
        "Argument must not be " .. ty)

    if not arg then
        if default then
            return default
        else
            error(ord(idx) .. " argument must not be empty")
        end
    end

    if type(arg) ~= ty then
        if ty == "number" then
            arg = tonumber(arg)
            assert(arg, ord(idx) .. " argument must be a number")
        elseif ty == "hex" then
            arg = tonumber(arg, 16)
            assert(arg, ord(idx) .. " argument must be a hex number")
        elseif ty == "boolean" then
            arg = arg ~= "0" and arg ~= "false"
        end
    end

    if validate then
        assert(validate(arg), ord(idx) .. " argument is invalid")
    end

    return arg
end

function M.sys.parse_args(args)
    res = {}
    for i, v in ipairs(args) do
        table.insert(res, parse_arg(i, v.arg, v.ty, v.validate, v.default))
    end
    return table.unpack(res)
end

function M.re.literal(str)
    local res = coroutine.yield({ ":re.literal", str })
    return res.literal
end

function M.re.matches(str, ...)
    local args = table.pack(...)
    assert(args.n % 2 == 0, "Each function need to have a name")

    local pattern_list = {}
    local pattern_table = {}
    for i = 1, args.n, 2 do
        table.insert(pattern_list, { args[i], args[i + 1] })
        pattern_table[args[i]] = args[i + 1]
    end

    local res = coroutine.yield({ ":re.matches", str, pattern_list })
    local fn_name = res.fn_name
    if fn_name ~= nil then
        local fn = pattern_table[fn_name]
        fn(str)
    end
end

function M.re.match(str, pattern)
    local res = coroutine.yield({ ":re.match", str, pattern })
    return res.is_match
end

return M
