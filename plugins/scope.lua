local M = {
    fmt = {},
    log = {},
    serial = {},
    ble = {},
    sys = {},
    re = {},
    rtt = {},
    test = {},
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
    local timeout = opts.timeout_ms
    if type(timeout) ~= "number" or timeout < 0 or timeout ~= math.floor(timeout) or timeout >= 2 ^ 53 then
        error(fn_name .. ": the timeout must be a whole number of milliseconds", 3)
    end
    if opts.error_pattern ~= nil and type(opts.error_pattern) ~= "string" then
        error(fn_name .. ": config.error_pattern must be a string pattern", 3)
    end
    local ignore_is_list = type(opts.ignore) == "table"
    for _, pattern in ipairs(ignore_is_list and opts.ignore or {}) do
        ignore_is_list = ignore_is_list and type(pattern) == "string"
    end
    if not ignore_is_list then
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

local Signal = {}

Signal.__tostring = function(signal)
    if signal.kind == "skip" then
        return "skipped" .. (signal.msg and (": " .. signal.msg) or "")
    end

    if signal.line then
        return signal.line .. ": " .. signal.msg
    end

    return signal.msg
end

-- Level 4 is the test's line only when an assert function calls fail directly.
local function fail(msg)
    local _, where = pcall(error, "", 4)
    error(setmetatable({ kind = "fail", msg = msg, line = where:match(":(%d+): $") }, Signal), 0)
end

local function repr(value, depth)
    depth = depth or 0

    if type(value) == "string" then
        return '"' .. value:gsub("\r", "\\r"):gsub("\n", "\\n") .. '"'
    end

    if type(value) ~= "table" or depth >= 3 then
        return tostring(value)
    end

    local items = {}
    for _, item in ipairs(value) do
        table.insert(items, repr(item, depth + 1))
    end

    local keys = {}
    for key in pairs(value) do
        if math.type(key) ~= "integer" or key < 1 or key > #value then
            table.insert(keys, key)
        end
    end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    for _, key in ipairs(keys) do
        table.insert(items, tostring(key) .. " = " .. repr(value[key], depth + 1))
    end

    return "{" .. table.concat(items, ", ") .. "}"
end

local function equal(a, b)
    if a == b then
        return true
    end

    if type(a) ~= "table" or type(b) ~= "table" then
        return false
    end

    for key, value in pairs(a) do
        if not equal(value, b[key]) then
            return false
        end
    end

    for key in pairs(b) do
        if a[key] == nil then
            return false
        end
    end

    return true
end

local function with(msg, detail)
    if msg ~= nil then
        return tostring(msg) .. ": " .. detail
    end

    return detail
end

function M.test.assert(cond, msg)
    if not cond then
        fail(msg ~= nil and tostring(msg) or "assertion failed")
    end
end

function M.test.assert_eq(actual, expected, msg)
    if not equal(actual, expected) then
        fail(with(msg, "expected " .. repr(expected) .. ", got " .. repr(actual)))
    end
end

function M.test.assert_opt(value, options, msg)
    assert(type(options) == "table", "test.assert_opt: the options must be a list")

    for _, option in ipairs(options) do
        if equal(value, option) then
            return
        end
    end

    fail(with(msg, repr(value) .. " is not one of " .. repr(options)))
end

function M.test.assert_re(value, pattern, msg)
    assert(type(pattern) == "string", "test.assert_re: the pattern must be a string")

    local text = value
    if type(value) == "table" then
        text = M.fmt.to_str(value)
    elseif type(value) == "number" then
        text = tostring(value)
    end

    if type(text) ~= "string" or not M.re.match(text, "^(?:" .. pattern .. ")$") then
        fail(with(msg, repr(value) .. " does not match " .. pattern))
    end
end

function M.test.skip(reason)
    error(setmetatable({ kind = "skip", msg = reason }, Signal), 0)
end

local function result_of(ok, err)
    if ok then
        return "pass"
    end

    if getmetatable(err) == Signal then
        return err.kind
    end

    return "fail"
end

local function describe(err)
    if getmetatable(err) == Signal then
        return tostring(err)
    end

    return (tostring(err):gsub('^%[string ".-"%]:', ""))
end

local function report(name, result, err)
    if result == "fail" then
        M.log.error(name .. ": " .. describe(err))
    elseif result == "skip" and err.msg ~= nil then
        M.log.warning(name .. ": " .. describe(err))
    end
end

local function default_teardown_test(index, total, name, result)
    local text = "[" .. index .. "/" .. total .. "] " .. name .. ": " .. result

    if result == "pass" then
        M.log.success(text)
    elseif result == "skip" then
        M.log.warning(text)
    else
        M.log.error(text)
    end
end

local function default_teardown_suite(results)
    local text = string.format("%d passed, %d failed, %d skipped", results.ok, results.fail, results.skip)

    if results.fail > 0 then
        M.log.error(text)
    else
        M.log.success(text)
    end
end

local function run_test(plugin, fn_name, name, fixture, setup_test)
    local ok, err = pcall(setup_test, name, fixture)
    local label = name .. " (setup_test)"

    if ok then
        ok, err = pcall(plugin[fn_name], fixture)
        label = name
    end

    local result = result_of(ok, err)
    report(label, result, err)

    return result
end

function M.test._run_suite(plugin, tests)
    local noop = function() end
    local setup_suite = plugin.setup_suite or noop
    local teardown_suite = plugin.teardown_suite or default_teardown_suite
    local setup_test = plugin.setup_test or noop
    local teardown_test = plugin.teardown_test or default_teardown_test
    local results = { ok = 0, fail = 0, skip = 0, total = #tests, tests = {} }

    local ok, fixture = pcall(setup_suite)
    local suite_result
    if not ok then
        suite_result = result_of(ok, fixture)
        report("setup_suite", suite_result, fixture)
        fixture = nil
    end

    for index, fn_name in ipairs(tests) do
        local name = (fn_name:gsub("^test_", ""))
        local result = suite_result

        if result == nil then
            result = run_test(plugin, fn_name, name, fixture, setup_test)

            local td_ok, td_err = pcall(teardown_test, index, #tests, name, result, fixture)
            if not td_ok then
                M.log.error(name .. " (teardown_test): " .. describe(td_err))
            end
        end

        local key = result == "pass" and "ok" or result
        results[key] = results[key] + 1
        table.insert(results.tests, { name = name, result = result })
    end

    local td_ok, td_err = pcall(teardown_suite, results, fixture)
    if not td_ok then
        M.log.error("teardown_suite: " .. describe(td_err))
    end
end

return M
