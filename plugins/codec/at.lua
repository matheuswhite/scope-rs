local at = {}

-- V.250 final result codes, sent without a "+NAME:" prefix.
local RESULT_CODES = {
    ["OK"] = true,
    ["ERROR"] = true,
    ["CONNECT"] = true,
    ["RING"] = true,
    ["NO CARRIER"] = true,
    ["NO DIALTONE"] = true,
    ["BUSY"] = true,
    ["NO ANSWER"] = true,
}

local CPIN_CODES = {
    "READY", "SIM PIN", "SIM PUK", "PH-SIM PIN", "PH-FSIM PIN", "PH-FSIM PUK",
    "SIM PIN2", "SIM PUK2", "PH-NET PIN", "PH-NET PUK", "PH-NETSUB PIN",
    "PH-NETSUB PUK", "PH-SP PIN", "PH-SP PUK", "PH-CORP PIN", "PH-CORP PUK",
}

local PDP_TYPES = { "X.25", "IP", "IPV6", "IPV4V6", "OSPIH", "PPP", "Non-IP", "Ethernet" }

local function one_of(options)
    return function(value)
        for _, option in ipairs(options) do
            if value == option then
                return true
            end
        end
        return false
    end
end

local function range(min, max)
    return function(value)
        return value >= min and value <= max
    end
end

local function is_act(value)
    return value == 0 or (value >= 2 and value <= 9) or (value >= 11 and value <= 13)
end

local function is_rssi(value)
    return (value >= 0 and value <= 31) or value == 99
end

local function is_ber(value)
    return (value >= 0 and value <= 7) or value == 99
end

local function non_negative(value)
    return value >= 0
end

local COMMANDS = {
    COPS = {
        required = 1,
        { "mode",     "int", one_of({ 0, 1, 2, 4 }) },
        { "format",   "int", one_of({ 0, 1, 2 }) },
        { "provider", "str" },
        { "act",      "int", is_act },
    },
    CSQ = {
        required = 2,
        { "rssi", "int", is_rssi },
        { "ber",  "int", is_ber },
    },
    CPIN = {
        required = 1,
        { "code", "str", one_of(CPIN_CODES) },
    },
    CFUN = {
        required = 1,
        { "fun", "int", range(0, 127) },
    },
    CGATT = {
        required = 1,
        { "state", "int", one_of({ 0, 1 }) },
    },
    CGACT = {
        required = 2,
        { "cid",   "int", non_negative },
        { "state", "int", one_of({ 0, 1 }) },
    },
    CGDCONT = {
        required = 3,
        { "cid",      "int", non_negative },
        { "pdp_type", "str", one_of(PDP_TYPES) },
        { "apn",      "str" },
        { "pdp_addr", "str" },
        { "d_comp",   "int", non_negative },
        { "h_comp",   "int", non_negative },
    },
    CGPADDR = {
        required = 1,
        { "cid",        "int", non_negative },
        { "pdp_addr",   "str" },
        { "pdp_addr_2", "str" },
    },
    ["CME ERROR"] = {
        required = 1,
        { "err", "any" },
    },
    ["CMS ERROR"] = {
        required = 1,
        { "err", "any" },
    },
}

local function registration(area)
    return function(params)
        local fields = {
            required = 1,
            { "stat", "int", range(0, 11) },
            { area,   "hex" },
            { "ci",   "hex" },
            { "act",  "int", is_act },
        }

        -- The read response starts with an unquoted <n>; the URC starts with
        -- <stat>, followed by the quoted <lac>/<tac>.
        if #params >= 2 and params[2]:match("^%s*%d+%s*$") then
            table.insert(fields, 1, { "n", "int", range(0, 5) })
            fields.required = 2
        end

        return fields
    end
end

COMMANDS.CREG = registration("lac")
COMMANDS.CGREG = registration("lac")
COMMANDS.CEREG = registration("tac")

local function trim(text)
    return (text:gsub("^%s+", ""):gsub("%s+$", ""))
end

local function split_params(args)
    local params = {}
    local current = {}
    local quoted = false

    for char in args:gmatch(".") do
        if char == '"' then
            quoted = not quoted
        end

        if char == "," and not quoted then
            table.insert(params, table.concat(current))
            current = {}
        else
            table.insert(current, char)
        end
    end
    table.insert(params, table.concat(current))

    return params
end

local function unquote(raw)
    if #raw >= 2 and raw:sub(1, 1) == '"' and raw:sub(-1) == '"' then
        return raw:sub(2, -2)
    end
    return raw
end

local function parse(raw, kind)
    if kind == "str" then
        return unquote(raw)
    end

    if kind == "hex" then
        local digits = unquote(raw)
        return digits:match("^%x+$") and tonumber(digits, 16) or nil
    end

    local int = raw:match("^%-?%d+$") and math.tointeger(tonumber(raw))
    if kind == "int" or int ~= nil then
        return int or nil
    end

    return unquote(raw)
end

local function field(raw, kind)
    raw = trim(raw)

    if raw == "" then
        return { raw = raw }
    end

    return { raw = raw, value = parse(raw, kind) }
end

local function decode_fields(output, fields, params)
    for index, spec in ipairs(fields) do
        local name, kind, check = spec[1], spec[2], spec[3]
        local raw = params[index]

        if raw == nil or trim(raw) == "" then
            if index <= fields.required then
                return nil, "+" .. output.id .. ": missing " .. name
            end
        else
            local decoded = field(raw, kind)
            if decoded.value == nil or (check ~= nil and not check(decoded.value)) then
                return nil, "+" .. output.id .. ": invalid " .. name .. " " .. decoded.raw
            end
            output[name] = decoded
        end
    end

    return output
end

--- Decodes one response line (a string or a list of bytes) into a table with
--- the command `id` and its `params`, plus a field per parameter of a known
--- command. Returns nil and the reason when the line is not a valid response.
function at.decode(line)
    local fmt = require("scope").fmt
    local text = trim(fmt.to_str(line))

    if RESULT_CODES[text] then
        return { id = text, params = {} }
    end

    local prefix, args = fmt.split(text, ":", 2)
    local id = prefix:match("^[%+%^%$#%%%*]([%w_%-%. ]+)$")
    if args == nil or id == nil then
        return nil, "not an AT response: " .. text
    end

    local params = split_params(args)
    local output = { id = id, params = {} }
    for index, raw in ipairs(params) do
        output.params[index] = field(raw, "any")
    end

    local fields = COMMANDS[id]
    if type(fields) == "function" then
        fields = fields(params)
    end
    if fields == nil then
        return output
    end

    return decode_fields(output, fields, params)
end

local function encode_param(value)
    if value == nil then
        return ""
    elseif type(value) == "number" then
        return tostring(math.tointeger(value) or value)
    elseif type(value) == "boolean" then
        return value and "1" or "0"
    elseif type(value) == "string" then
        return '"' .. value .. '"'
    end

    error("at.encode: cannot encode a " .. type(value) .. " parameter", 3)
end

local function check_command(fn_name, cmd)
    if type(cmd) ~= "string" or cmd == "" then
        error("at." .. fn_name .. ": the command must be a non-empty string, as written after AT", 3)
    end
end

--- Builds a set or execute command: `at.encode("+CGDCONT", 1, "IP", "apn")`
--- gives `AT+CGDCONT=1,"IP","apn"`, and `at.encode("+CIMI")` gives `AT+CIMI`.
function at.encode(cmd, ...)
    check_command("encode", cmd)

    local count = select("#", ...)
    local params = { ... }
    while count > 0 and params[count] == nil do
        count = count - 1
    end

    if count == 0 then
        return "AT" .. cmd
    end

    local encoded = {}
    for index = 1, count do
        encoded[index] = encode_param(params[index])
    end

    return "AT" .. cmd .. "=" .. table.concat(encoded, ",")
end

--- Builds a read command: `at.encode_read("+COPS")` gives `AT+COPS?`.
function at.encode_read(cmd)
    check_command("encode_read", cmd)
    return "AT" .. cmd .. "?"
end

--- Builds a test command: `at.encode_test("+COPS")` gives `AT+COPS=?`.
function at.encode_test(cmd)
    check_command("encode_test", cmd)
    return "AT" .. cmd .. "=?"
end

return at
