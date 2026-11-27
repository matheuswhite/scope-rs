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

local REG_STAT = {
    [0] = "not registered, not searching",
    [1] = "registered, home",
    [2] = "searching",
    [3] = "registration denied",
    [4] = "unknown",
    [5] = "registered, roaming",
    [6] = "registered for SMS only, home",
    [7] = "registered for SMS only, roaming",
    [8] = "attached for emergency bearer services only",
    [9] = "registered for CSFB not preferred, home",
    [10] = "registered for CSFB not preferred, roaming",
    [11] = "attached for access to RLOS",
}

local ACT = {
    [0] = "GSM",
    [1] = "GSM Compact",
    [2] = "UTRAN",
    [3] = "GSM/EGPRS",
    [4] = "UTRAN/HSDPA",
    [5] = "UTRAN/HSUPA",
    [6] = "UTRAN/HSPA",
    [7] = "E-UTRAN",
    [8] = "EC-GSM-IoT",
    [9] = "E-UTRAN (NB-S1)",
    [10] = "E-UTRA connected to a 5GCN",
    [11] = "NR connected to a 5GCN",
    [12] = "NG-RAN",
    [13] = "E-UTRA-NR dual connectivity",
}

local COPS_MODE = {
    [0] = "automatic",
    [1] = "manual",
    [2] = "deregistered",
    [4] = "manual, then automatic",
}

local CFUN_FUN = {
    [0] = "minimum functionality",
    [1] = "full functionality",
    [2] = "transmit RF circuits disabled",
    [3] = "receive RF circuits disabled",
    [4] = "transmit and receive RF circuits disabled",
}

-- 3GPP TS 24.008 / 24.301 session management causes.
local ESM_CAUSE = {
    [8] = "operator determined barring",
    [26] = "insufficient resources",
    [27] = "missing or unknown APN",
    [28] = "unknown PDP address or PDP type",
    [29] = "user authentication failed",
    [30] = "activation rejected by the gateway",
    [31] = "activation rejected, unspecified",
    [32] = "service option not supported",
    [33] = "requested service option not subscribed",
    [34] = "service option temporarily out of order",
    [36] = "regular deactivation",
    [38] = "network failure",
    [39] = "reactivation requested",
    [50] = "PDN type IPv4 only allowed",
    [51] = "PDN type IPv6 only allowed",
    [52] = "single address bearers only allowed",
    [53] = "ESM information not received",
    [54] = "PDN connection does not exist",
    [55] = "multiple PDN connections for a given APN not allowed",
    [56] = "collision with network initiated request",
    [57] = "PDN type IPv4v6 only allowed",
    [65] = "maximum number of PDP contexts reached",
    [66] = "requested APN not supported in current RAT and PLMN",
    [111] = "protocol error, unspecified",
    [112] = "APN restriction value incompatible with active context",
}

local KINDS = { int = true, str = true, hex = true, any = true }
local RESERVED = { id = true, params = true, raw = true }

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

-- 3GPP TS 27.007 <AcT>: 0 (GSM) to 13 (E-UTRA-NR dual connectivity).
local is_act = range(0, 13)

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
        { "mode",     "int", one_of({ 0, 1, 2, 4 }), text = COPS_MODE },
        { "format",   "int", one_of({ 0, 1, 2 }) },
        { "provider", "str" },
        { "act",      "int", is_act,                 text = ACT },
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
        { "fun", "int", range(0, 127), text = CFUN_FUN },
    },
    CGATT = {
        required = 1,
        { "state", "int", one_of({ 0, 1 }), text = { [0] = "detached", [1] = "attached" } },
    },
    CGACT = {
        list = true,
        key = "cid",
        required = 2,
        { "cid",   "int", non_negative },
        { "state", "int", one_of({ 0, 1 }), text = { [0] = "deactivated", [1] = "activated" } },
    },
    CGDCONT = {
        list = true,
        key = "cid",
        required = 3,
        { "cid",      "int", non_negative },
        { "pdp_type", "str", one_of(PDP_TYPES) },
        { "apn",      "str" },
        { "pdp_addr", "str" },
        { "d_comp",   "int", non_negative },
        { "h_comp",   "int", non_negative },
    },
    CGPADDR = {
        list = true,
        key = "cid",
        required = 1,
        { "cid",        "int", non_negative },
        { "pdp_addr",   "str" },
        { "pdp_addr_2", "str" },
    },
    CIMI = {
        unprefixed = true,
        required = 1,
        { "imsi", "str" },
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
    return {
        fields = function(params)
            local fields = {
                required = 1,
                { "stat", "int", range(0, 11), text = REG_STAT },
                { area,   "hex" },
                { "ci",   "hex" },
                { "act",  "int", is_act,       text = ACT },
            }

            -- The read response starts with an unquoted <n>; the URC starts
            -- with <stat>, followed by the quoted <lac>/<tac>.
            if math.type(params[2]) == "integer" then
                table.insert(fields, 1, { "n", "int", range(0, 5) })
                fields.required = 2
            end

            return fields
        end,
    }
end

COMMANDS.CREG = registration("lac")
COMMANDS.CGREG = registration("lac")
COMMANDS.CEREG = registration("tac")

local DEFINED = {}

local function fmt()
    return require("scope").fmt
end

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

    if kind == "int" then
        local digits = unquote(raw)
        return digits:match("^%-?%d+$") and math.tointeger(tonumber(digits)) or nil
    end

    local int = raw:match("^%-?%d+$") and math.tointeger(tonumber(raw)) or nil
    if int ~= nil then
        return int
    end

    return unquote(raw)
end

--- The prefix id of a response line and its parameters, or nil for a line
--- without a "+NAME:" prefix.
local function split_line(text)
    local prefix, args = fmt().split(text, ":", 2)
    local id = args and prefix:match("^[%+%^%$#%%%*]([%w_%-%. ]+)$")
    if id == nil then
        return nil
    end
    return id, args
end

local function fields_of(spec, params)
    if spec.fields ~= nil then
        return spec.fields(params)
    end
    return spec
end

local function decode_entry(id, spec, args)
    local entry = { id = id, params = {}, raw = {} }
    local raws = split_params(args)
    for index, raw in ipairs(raws) do
        raw = trim(raw)
        entry.raw[index] = raw
        if raw ~= "" then
            entry.params[index] = parse(raw, "any")
        end
    end

    if spec == nil then
        return entry
    end

    local fields = fields_of(spec, entry.params)
    for index, field in ipairs(fields) do
        local name, kind, check = field[1], field[2], field[3]
        local raw = entry.raw[index]

        if raw == nil or raw == "" then
            if index <= (fields.required or 0) then
                return nil, "+" .. id .. ": missing " .. name
            end
        else
            local value = parse(raw, kind)
            if value == nil or (check ~= nil and not check(value)) then
                return nil, "+" .. id .. ": invalid " .. name .. " " .. raw
            end
            entry[name] = value
            entry.raw[name] = raw
        end
    end

    return entry
end

--- The command name and the spec a sent command is decoded with.
local function command_spec(cmd)
    local name, rest = cmd:match("^[Aa][Tt][%+%^%$#%%%*]?([%w_]+)(.*)$")
    if name == nil then
        return nil
    end

    local first = rest:match("^=([^?][^,]*)")
    if first ~= nil and DEFINED[name .. "=" .. trim(first)] ~= nil then
        return name, DEFINED[name .. "=" .. trim(first)]
    end

    return name, DEFINED[name] or COMMANDS[name]
end

local function line_text(line)
    return trim(fmt().to_str(line))
end

local function decode_line(line, cmd)
    local text = line_text(line)

    if RESULT_CODES[text] then
        return { id = text, params = {}, raw = {} }
    end

    local id, args = split_line(text)
    if cmd ~= nil then
        local name, spec = command_spec(cmd)
        if id == nil and spec ~= nil and spec.unprefixed then
            return decode_entry(name, spec, text)
        end
        if id ~= nil and id == name then
            return decode_entry(id, spec, args)
        end
    end

    if id == nil then
        return nil, "not an AT response: " .. text
    end

    return decode_entry(id, DEFINED[id] or COMMANDS[id], args)
end

local function decode_response(lines, cmd)
    local name, spec
    if cmd ~= nil then
        name, spec = command_spec(cmd)
    else
        for _, line in ipairs(lines) do
            name = split_line(line_text(line))
            if name ~= nil then
                break
            end
        end
        if name == nil then
            return nil, "no AT response in the lines"
        end
        spec = DEFINED[name] or COMMANDS[name]
    end

    local entries = {}
    for _, line in ipairs(lines) do
        local text = line_text(line)
        local id, args = split_line(text)
        local entry, err
        if spec ~= nil and spec.unprefixed then
            if id == nil and text ~= "" and not RESULT_CODES[text] then
                entry, err = decode_entry(name, spec, text)
            end
        elseif id == name then
            entry, err = decode_entry(id, spec, args)
        end

        if err ~= nil then
            return nil, err
        end
        table.insert(entries, entry)
    end

    if spec == nil or not spec.list then
        if entries[1] == nil then
            return nil, "no +" .. name .. " line in the response"
        end
        return entries[1]
    end

    if spec.key == nil then
        return entries
    end

    local keyed = {}
    for _, entry in ipairs(entries) do
        if entry[spec.key] ~= nil then
            keyed[entry[spec.key]] = entry
        end
    end
    return keyed
end

--- Decodes an AT response. `lines` is either one line (a string or a list of
--- bytes), decoded into one table, or the list of lines `send_and_wait`
--- returned, decoded into the response of `cmd`: one table, or for a command
--- answering one line per entry, a list indexed by the command's key (e.g.
--- `cid`). `cmd`, the command sent, is needed for lines without a "+NAME:"
--- prefix. Returns nil and the reason when the response is not valid.
function at.decode(lines, cmd)
    if cmd ~= nil and (type(cmd) ~= "string" or command_spec(cmd) == nil) then
        error("at.decode: cmd must be the AT command sent (e.g. \"AT+CIMI\"), not " .. tostring(cmd), 2)
    end

    if type(lines) == "string" or (type(lines) == "table" and type(lines[1]) == "number") then
        return decode_line(lines, cmd)
    end

    if type(lines) ~= "table" then
        error("at.decode: the response must be a line or a list of lines", 2)
    end

    return decode_response(lines, cmd)
end

--- Declares how to decode a command's response, as the built-in 27.007
--- commands are: `id` is the name after `AT+` (e.g. "QNWINFO"), or the name and
--- its first parameter (e.g. 'QCFG="roamservice"') when the response changes
--- with it. Replaces any earlier spec for the same `id`.
function at.define(id, spec)
    if type(id) ~= "string" or id == "" then
        error("at.define: the id must be a non-empty string", 2)
    end
    if type(spec) ~= "table" then
        error("at.define: the spec must be a table", 2)
    end
    if spec.fields ~= nil and type(spec.fields) ~= "function" then
        error("at.define: fields must be a function of the line's params", 2)
    end

    local names = {}
    for index, field in ipairs(spec) do
        local name, kind = field[1], field[2]
        if type(name) ~= "string" or RESERVED[name] then
            error("at.define: field " .. index .. " needs a name other than id, params and raw", 2)
        end
        if not KINDS[kind] then
            error("at.define: field " .. name .. " needs a kind: int, str, hex or any", 2)
        end
        names[name] = true
    end
    if spec.key ~= nil and spec.fields == nil and not names[spec.key] then
        error("at.define: the key " .. tostring(spec.key) .. " is not a field", 2)
    end

    DEFINED[id] = spec
end

--- The description of an enumerated field's value, e.g.
--- `at.text("CEREG", "stat", 1)` gives "registered, home".
function at.text(id, field, value)
    local spec = DEFINED[id] or COMMANDS[id]
    if spec == nil then
        return nil
    end

    for _, f in ipairs(fields_of(spec, {})) do
        if f[1] == field and f.text ~= nil then
            return f.text[value]
        end
    end
    return nil
end

--- The description of a session management cause (3GPP TS 24.008 / 24.301),
--- as `+CEER` reports it after a failed data call.
function at.esm_cause(cause)
    return ESM_CAUSE[cause]
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
