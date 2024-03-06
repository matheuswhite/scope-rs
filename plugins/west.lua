---
--- Generated by EmmyLua(https://github.com/EmmyLua)
--- Created by matheuswhite.
--- DateTime: 26/02/24 21:00
---

require "scope"

function check_zephyr_base()
    stdout, stderr = scope.exec('echo $ZEPHYR_BASE')
    return stdout ~= nil and stdout ~= '' and stdout ~= '\n'
end

function serial_rx(msg)
end

function user_command(arg_list)
    if not check_zephyr_base() then
        scope.eprintln('$ZEPHYR_BASE is empty')
        return
    end

    local cmd = 'west ' .. table.concat(arg_list, ' ')
    
    if osname() == 'unix' then
        cmd = 'source $ZEPHYR_BASE/../.venv/bin/activate && ' .. cmd
    end
    
    scope.exec(cmd)
end
