//! The Lua standard library bundled with scope (`require("scope")`,
//! `require("shell")`, `require("codec.at")`).

use std::path::Path;

/// Each module as `(path relative to the plugins directory, contents)`.
pub const STDLIB: &[(&str, &str)] = &[
    ("scope.lua", include_str!("../../plugins/scope.lua")),
    ("shell.lua", include_str!("../../plugins/shell.lua")),
    ("codec/at.lua", include_str!("../../plugins/codec/at.lua")),
];

/// Writes every module into `dir` whose copy there is missing or differs, so
/// the plugins always run against the library of this scope version.
pub fn provision(dir: &Path) -> Result<(), String> {
    for (name, contents) in STDLIB {
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|current| current == contents.as_bytes()) {
            continue;
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Cannot create {:?}: {}", parent, err))?;
        }
        std::fs::write(&path, contents)
            .map_err(|err| format!("Cannot write {:?}: {}", path, err))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use mlua::{Lua, LuaOptions, StdLib};
    use regex::Regex;

    /// Runs plugin code as the plugin engine would, answering each request it
    /// yields from the `responses` table, and records the requests and logs.
    const DRIVER: &str = r##"
        local function show(value, depth)
            depth = depth or 0
            if type(value) == "string" then return string.format("%q", value) end
            if type(value) ~= "table" or depth > 4 then return tostring(value) end
            local keys = {}
            for key in pairs(value) do table.insert(keys, key) end
            table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
            local items = {}
            for _, key in ipairs(keys) do
                table.insert(items, tostring(key) .. "=" .. show(value[key], depth + 1))
            end
            return "{" .. table.concat(items, ", ") .. "}"
        end

        local function same(a, b)
            if a == b then return true end
            if type(a) ~= "table" or type(b) ~= "table" then return false end
            for key, value in pairs(a) do if not same(value, b[key]) then return false end end
            for key in pairs(b) do if a[key] == nil then return false end end
            return true
        end

        function expect_eq(actual, expected)
            if not same(actual, expected) then
                error("\nexpected " .. show(expected) .. "\n     got " .. show(actual), 2)
            end
        end

        function drive(fn, responses, ...)
            responses = responses or {}
            local requests, logs = {}, {}
            local co = coroutine.create(fn)
            local resumed = table.pack(coroutine.resume(co, ...))
            while true do
                if not resumed[1] then error(resumed[2], 0) end
                if coroutine.status(co) == "dead" then
                    return { requests = requests, logs = logs, results = { table.unpack(resumed, 2, resumed.n) } }
                end
                local req = resumed[2]
                local id = req[1]
                local rsp = {}
                if id:match("^:log%.") then
                    table.insert(logs, id:sub(6) .. ": " .. tostring(req[2]))
                elseif id == ":re.match" then
                    rsp = { is_match = rust_is_match(req[2], req[3]) }
                else
                    table.insert(requests, req)
                    assert(responses[id], "unexpected request " .. id)
                    rsp = responses[id](req)
                end
                resumed = table.pack(coroutine.resume(co, rsp))
            end
        end

        function load_plugin(code)
            M = load(code)()
            require("scope")._init_config(M)
            return M
        end

        scope = require("scope")
    "##;

    fn run(chunk: &str) {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()).unwrap();
        lua.load("package.path = 'plugins/?.lua;' .. package.path")
            .exec()
            .unwrap();
        let is_match = lua
            .create_function(|_, (text, pattern): (String, String)| {
                Ok(Regex::new(&pattern).is_ok_and(|re| re.is_match(&text)))
            })
            .unwrap();
        lua.globals().set("rust_is_match", is_match).unwrap();
        lua.load(DRIVER).exec().unwrap();

        if let Err(err) = lua.load(chunk).exec() {
            panic!("{err}");
        }
    }

    #[test]
    fn split_keeps_empty_fields_and_takes_the_separator_literally() {
        run(r#"
            expect_eq({ scope.fmt.split("a,,b", ",") }, { "a", "", "b" })
            expect_eq({ scope.fmt.split("1.2.3", ".") }, { "1", "2", "3" })
            expect_eq({ scope.fmt.split("abc", ",") }, { "abc" })
            expect_eq({ scope.fmt.split("a, b", ", ") }, { "a", "b" })
        "#);
    }

    #[test]
    fn split_stops_at_max_pieces() {
        run(r#"
            expect_eq({ scope.fmt.split('+COPS: 0,"a:b"', ":", 2) }, { "+COPS", ' 0,"a:b"' })
        "#);
    }

    #[test]
    fn split_takes_a_list_of_bytes() {
        run(r#"
            expect_eq({ scope.fmt.split({ 0x61, 0x3a, 0x62 }, ":") }, { "a", "b" })
        "#);
    }

    #[test]
    fn send_passes_strings_and_bytes_through() {
        run(r#"
            local out = drive(function()
                scope.serial.send("AT\r\n")
                scope.rtt.send({ 0x41, 0x54 })
            end, {
                [":serial.send"] = function() return {} end,
                [":rtt.send"] = function() return {} end,
            })
            expect_eq(out.requests, { { ":serial.send", "AT\r\n" }, { ":rtt.send", { 0x41, 0x54 } } })
        "#);
    }

    #[test]
    fn send_rejects_other_types() {
        run(r#"
            local ok, err = pcall(drive, function() scope.serial.send(42) end)
            assert(not ok and err:find("serial.send: the message must be a string or a list of bytes, not number"), err)
        "#);
    }

    #[test]
    fn init_config_fills_the_defaults_and_keeps_the_plugin_values() {
        run(r#"
            local plugin = load_plugin([[ return { config = { default_timeout = 200 } } ]])
            expect_eq(plugin.config, {
                recv_as_string = false,
                default_timeout = 200,
                default_expected = "OK",
                line_ending = "\r\n",
                ignore_msg = {},
            })

            expect_eq(load_plugin("return {}").config.default_timeout, 1000)

            local ok, err = pcall(load_plugin, "return { config = 1 }")
            expect_eq(err, "M.config must be a table, not number")
        "#);
    }

    #[test]
    fn recv_returns_a_string_when_the_plugin_asks_for_it() {
        run(r#"
            local responses = {
                [":serial.recv"] = function() return { data = { 0x4f, 0x4b } } end,
            }
            local recv = function() return scope.serial.recv({}) end

            load_plugin("return {}")
            expect_eq(drive(recv, responses).results, { nil, { 0x4f, 0x4b } })

            load_plugin("return { config = { recv_as_string = true } }")
            expect_eq(drive(recv, responses).results, { nil, "OK" })
        "#);
    }

    #[test]
    fn send_and_wait_builds_the_request_from_the_config() {
        run(r#"
            load_plugin([[ return { config = {
                send_prefix = "modem at",
                error_pattern = "\\+CME ERROR: .*",
                default_timeout = 200,
                ignore_msg = { "uart.*" },
            } } ]])

            local out = drive(function()
                return scope.serial.send_and_wait("AT+COPS?")
            end, {
                [":serial.send_and_wait"] = function()
                    return { lines = { "+COPS: 0,0,\"TIM\",7" }, matched = "OK" }
                end,
            })

            expect_eq(out.requests[1][2], {
                message = "modem at AT+COPS?\r\n",
                expected = "OK",
                timeout_ms = 200,
                error_pattern = "\\+CME ERROR: .*",
                ignore = { "uart.*" },
            })
            expect_eq(out.results, { { "+COPS: 0,0,\"TIM\",7" }, "OK" })
        "#);
    }

    #[test]
    fn send_and_wait_arguments_override_the_config() {
        run(r#"
            load_plugin([[ return { config = { send_prefix = "at ", line_ending = "\r" } } ]])

            local out = drive(function()
                return scope.rtt.send_and_wait({ 0x41, 0x54 }, "READY", 50)
            end, {
                [":rtt.send_and_wait"] = function() return { lines = {}, matched = "READY" } end,
            })

            local opts = out.requests[1][2]
            expect_eq({ opts.message, opts.expected, opts.timeout_ms }, { "at AT\r", "READY", 50 })
        "#);
    }

    #[test]
    fn send_and_wait_returns_nil_and_the_reason_on_failure() {
        run(r#"
            load_plugin("return {}")

            local out = drive(function()
                return scope.serial.send_and_wait("AT")
            end, {
                [":serial.send_and_wait"] = function() return { err = "timeout" } end,
            })

            expect_eq(out.results, { nil, "timeout" })
        "#);
    }

    #[test]
    fn send_and_wait_rejects_a_bad_timeout() {
        run(r#"
            load_plugin([[ return { config = { default_timeout = "200" } } ]])

            local ok, err = pcall(drive, function() scope.serial.send_and_wait("AT") end)
            assert(not ok and err:find("the timeout must be a number of milliseconds"), err)
        "#);
    }

    #[test]
    fn a_suite_runs_the_hooks_around_each_test_in_order() {
        run(r#"
            local calls = {}
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.setup_suite() table.insert(CALLS, "setup_suite") return "fx" end
                function M.teardown_suite(results, fixture)
                    table.insert(CALLS, "teardown_suite " .. fixture)
                    RESULTS = results
                end
                function M.setup_test(name, fixture) table.insert(CALLS, "setup_test " .. name .. " " .. fixture) end
                function M.teardown_test(index, total, name, result)
                    table.insert(CALLS, "teardown_test " .. index .. "/" .. total .. " " .. name .. " " .. result)
                end
                function M.test_one(fixture) table.insert(CALLS, "one " .. fixture) end
                function M.test_two() test.assert(false) end
                function M.test_three() test.skip() end
                return M
            ]])
            CALLS = calls

            drive(scope.test._run_suite, {}, plugin, { "test_one", "test_two", "test_three" })

            expect_eq(calls, {
                "setup_suite",
                "setup_test one fx", "one fx", "teardown_test 1/3 one pass",
                "setup_test two fx", "teardown_test 2/3 two fail",
                "setup_test three fx", "teardown_test 3/3 three skip",
                "teardown_suite fx",
            })
            expect_eq(RESULTS, {
                ok = 1, fail = 1, skip = 1, total = 3,
                tests = {
                    { name = "one", result = "pass" },
                    { name = "two", result = "fail" },
                    { name = "three", result = "skip" },
                },
            })
        "#);
    }

    #[test]
    fn a_failed_assert_stops_the_test_and_logs_its_line() {
        run(r#"
            local reached = false
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.test_cops()
                    test.assert_eq("CREG", "COPS")
                    REACHED = true
                end
                return M
            ]])

            local out = drive(scope.test._run_suite, {}, plugin, { "test_cops" })

            expect_eq(REACHED, nil)
            expect_eq(out.logs, {
                'error: cops: 4: expected "COPS", got "CREG"',
                "error: [1/1] cops: fail",
                "error: 0 passed, 1 failed, 0 skipped",
            })
        "#);
    }

    #[test]
    fn asserts_describe_what_did_not_match() {
        run(r#"
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.test_a() test.assert(nil, "no answer") end
                function M.test_b() test.assert_opt(3, { 0, 1, 2, 4 }) end
                function M.test_c() test.assert_re("12a", "\\d+") end
                function M.test_d() test.assert_eq({ 1, { 2 } }, { 1, { 3 } }, "bytes") end
                function M.test_e() test.assert_re("0", "\\d+") test.assert_eq({ 1, { 2 } }, { 1, { 2 } }) end
                function M.teardown_test() end
                function M.teardown_suite() end
                return M
            ]])

            local out = drive(scope.test._run_suite, {}, plugin, { "test_a", "test_b", "test_c", "test_d", "test_e" })

            expect_eq(out.logs, {
                "error: a: 3: no answer",
                "error: b: 4: 3 is not one of {0, 1, 2, 4}",
                'error: c: 5: "12a" does not match \\d+',
                "error: d: 6: bytes: expected {1, {3}}, got {1, {2}}",
            })
        "#);
    }

    #[test]
    fn a_skip_reason_and_a_runtime_error_are_logged() {
        run(r#"
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.test_skip() test.skip("no SIM") end
                function M.test_crash() local cops = nil; return cops.mode end
                function M.teardown_test() end
                function M.teardown_suite() end
                return M
            ]])

            local out = drive(scope.test._run_suite, {}, plugin, { "test_skip", "test_crash" })

            expect_eq(out.logs, {
                "warning: skip: skipped: no SIM",
                "error: crash: 4: attempt to index a nil value (local 'cops')",
            })
        "#);
    }

    #[test]
    fn a_failed_setup_suite_runs_no_test() {
        run(r#"
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.setup_suite() error("no modem", 0) end
                function M.setup_test() RAN = true end
                function M.test_one() RAN = true end
                function M.teardown_suite(results) RESULTS = results end
                return M
            ]])

            local out = drive(scope.test._run_suite, {}, plugin, { "test_one" })

            expect_eq(RAN, nil)
            expect_eq(RESULTS.fail, 1)
            expect_eq(out.logs, { "error: setup_suite: no modem" })
        "#);
    }

    #[test]
    fn a_skip_in_setup_suite_skips_every_test() {
        run(r#"
            local plugin = load_plugin([[
                local test = require("scope").test
                local M = {}
                function M.setup_suite() test.skip() end
                function M.test_one() end
                function M.test_two() end
                return M
            ]])

            local out = drive(scope.test._run_suite, {}, plugin, { "test_one", "test_two" })

            expect_eq(out.logs, { "success: 0 passed, 0 failed, 2 skipped" })
        "#);
    }

    #[test]
    fn the_issue_example_suite_runs_against_a_modem() {
        run(r#"
            local plugin = load_plugin([[
                local scope = require("scope")
                local at = scope.codec.at
                local serial = scope.serial
                local test = scope.test
                local M = {}

                function M.setup_suite()
                    M.config.error_pattern = "\\+CMR.*"
                    M.config.default_timeout = 200
                    M.config.send_prefix = "modem at"
                    M.config.ignore_msg = { "uart" }
                end

                function M.test_at()
                    test.skip()
                end

                function M.test_cops()
                    local lines = serial.send_and_wait("AT+COPS?")
                    test.assert(lines)

                    local cops = at.decode(lines[1])
                    assert(cops)

                    test.assert_eq(cops.id, "COPS")
                    test.assert_opt(cops.mode.value, { 0, 1, 2, 4 })
                    test.assert_opt(cops.format.value, { 0, 1, 2 })
                    test.assert_re(cops.format.raw, "\\d+")
                    test.assert_eq(cops.provider.value, "TIMBrasil")
                    test.assert_eq(cops.act.value, 7)
                end

                return M
            ]])

            local out = drive(scope.test._run_suite, {
                [":serial.send_and_wait"] = function(req)
                    expect_eq(req[2].message, "modem at AT+COPS?\r\n")
                    expect_eq(req[2].ignore, { "uart" })
                    return { lines = { '+COPS: 0,0,"TIMBrasil",7' }, matched = "OK" }
                end,
            }, plugin, { "test_at", "test_cops" })

            expect_eq(out.logs, {
                "warning: [1/2] at: skip",
                "success: [2/2] cops: pass",
                "success: 1 passed, 0 failed, 1 skipped",
            })
        "#);
    }

    #[test]
    fn a_test_signal_reads_as_its_message() {
        run(r#"
            local ok, err = pcall(scope.test.skip, "no SIM")
            expect_eq(tostring(err), "skipped: no SIM")
        "#);
    }

    #[test]
    fn decode_reads_a_cops_read_response() {
        run(r#"
            local cops = scope.codec.at.decode('+COPS: 0,0,"TIMBrasil",7\r\n')
            expect_eq(cops.id, "COPS")
            expect_eq(cops.mode, { raw = "0", value = 0 })
            expect_eq(cops.format, { raw = "0", value = 0 })
            expect_eq(cops.provider, { raw = '"TIMBrasil"', value = "TIMBrasil" })
            expect_eq(cops.act, { raw = "7", value = 7 })
            expect_eq(#cops.params, 4)

            local unregistered = scope.codec.at.decode("+COPS: 0")
            expect_eq({ unregistered.mode.value, unregistered.format }, { 0, nil })
        "#);
    }

    #[test]
    fn decode_rejects_out_of_range_values() {
        run(r#"
            expect_eq({ scope.codec.at.decode('+COPS: 3,0,"TIM",7') }, { nil, "+COPS: invalid mode 3" })
            expect_eq({ scope.codec.at.decode('+COPS: 0,0,"TIM",10') }, { nil, "+COPS: invalid act 10" })
            expect_eq({ scope.codec.at.decode("+CSQ: 32,99") }, { nil, "+CSQ: invalid rssi 32" })
            expect_eq({ scope.codec.at.decode("+CSQ: 20") }, { nil, "+CSQ: missing ber" })
            expect_eq({ scope.codec.at.decode("+CPIN: BUSY") }, { nil, "+CPIN: invalid code BUSY" })
        "#);
    }

    #[test]
    fn decode_tells_a_registration_read_response_from_its_urc() {
        run(r#"
            local at = scope.codec.at

            local read = at.decode('+CREG: 2,5,"00C3","0000A1B2",7')
            expect_eq({ read.n.value, read.stat.value, read.lac.value, read.ci.value, read.act.value },
                { 2, 5, 0xC3, 0xA1B2, 7 })

            local urc = at.decode('+CEREG: 1,"1A2B","01A2B3C4",7')
            expect_eq({ urc.n, urc.stat.value, urc.tac.value, urc.act.value }, { nil, 1, 0x1A2B, 7 })

            local short = at.decode("+CGREG: 0,1")
            expect_eq({ short.n.value, short.stat.value }, { 0, 1 })
        "#);
    }

    #[test]
    fn decode_reads_the_basic_27007_responses() {
        run(r#"
            local at = scope.codec.at

            local csq = at.decode("+CSQ: 99,99")
            expect_eq({ csq.rssi.value, csq.ber.value }, { 99, 99 })
            expect_eq(at.decode("+CPIN: SIM PIN").code.value, "SIM PIN")
            expect_eq(at.decode("+CFUN: 1").fun.value, 1)
            expect_eq(at.decode("+CGATT: 1").state.value, 1)

            local cgact = at.decode("+CGACT: 1,0")
            expect_eq({ cgact.cid.value, cgact.state.value }, { 1, 0 })

            local cgdcont = at.decode('+CGDCONT: 1,"IPV4V6","apn,with,commas","0.0.0.0",0,0')
            expect_eq({ cgdcont.cid.value, cgdcont.pdp_type.value, cgdcont.apn.value, cgdcont.h_comp.value },
                { 1, "IPV4V6", "apn,with,commas", 0 })

            local cgpaddr = at.decode('+CGPADDR: 1,"10.0.0.2"')
            expect_eq({ cgpaddr.cid.value, cgpaddr.pdp_addr.value, cgpaddr.pdp_addr_2 }, { 1, "10.0.0.2", nil })
        "#);
    }

    #[test]
    fn decode_reads_result_codes_errors_and_unknown_commands() {
        run(r#"
            local at = scope.codec.at

            expect_eq(at.decode("OK\r\n"), { id = "OK", params = {} })
            expect_eq(at.decode("NO CARRIER").id, "NO CARRIER")
            expect_eq(at.decode("+CME ERROR: 10").err.value, 10)
            expect_eq(at.decode("+CMS ERROR: SIM busy").err.value, "SIM busy")

            local qnwinfo = at.decode('+QNWINFO: "FDD LTE","72404","LTE BAND 3",1300')
            expect_eq(qnwinfo.id, "QNWINFO")
            expect_eq(qnwinfo.params[2], { raw = '"72404"', value = "72404" })
            expect_eq(qnwinfo.params[4], { raw = "1300", value = 1300 })

            expect_eq({ at.decode("uart:~$ modem at AT") }, { nil, "not an AT response: uart:~$ modem at AT" })
            expect_eq(at.decode({ 0x4f, 0x4b }).id, "OK")
        "#);
    }

    #[test]
    fn encode_builds_set_read_and_test_commands() {
        run(r#"
            local at = scope.codec.at

            expect_eq(at.encode("+CGDCONT", 1, "IP", "apn"), 'AT+CGDCONT=1,"IP","apn"')
            expect_eq(at.encode("+COPS", 1, nil, nil), "AT+COPS=1")
            expect_eq(at.encode("+CREG", nil, 2), "AT+CREG=,2")
            expect_eq(at.encode("+CIMI"), "AT+CIMI")
            expect_eq(at.encode("E0"), "ATE0")
            expect_eq(at.encode("+CFUN", 1.0, true), "AT+CFUN=1,1")
            expect_eq(at.encode_read("+COPS"), "AT+COPS?")
            expect_eq(at.encode_test("+COPS"), "AT+COPS=?")
        "#);
    }
}
