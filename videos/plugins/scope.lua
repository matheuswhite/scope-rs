local M = {
    fmt = {},
    log = {},
    serial = {},
    ble = {},
    sys = {},
    re = {},
}

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
    local port, baud_rate = table.unpack(coroutine.yield({ ":serial.info" }))
    return port, baud_rate
end

function M.serial.send(msg)
    coroutine.yield({ ":serial.send", msg })
end

function M.serial.recv(opts)
    local err, msg = table.unpack(coroutine.yield({ ":serial.recv", opts }))
    return err, msg
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
    assert(not (ty == "nil" or ty == "function" or ty == "userdata" or ty == "thread" or ty == "table"), "Argument must not be " .. ty)

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
    return table.unpack(coroutine.yield({ ":re.literal", str }))
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

    local fn_name = table.unpack(coroutine.yield({ ":re.matches", str, pattern_list }))
    if fn_name ~= nil then
        local fn = pattern_table[fn_name]
        fn(str)
    end
end

function M.re.match(str, pattern)
    return table.unpack(coroutine.yield({ ":re.match", str, pattern }))
end

return M
