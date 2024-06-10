# Plugin API

### `on_load`

```lua
function M.on_load()
    -- code ...
end
```

## Serial

### `on_send`

```lua
--- Event function called when a message is sent via serial. 
--- 
--- @param msg string The message sent via serial.
function M.serial.on_send(msg)
    -- code ...
end
```

The message transmission occurs before the call of the `on_send` event function. 
It'll call this function again, only if the current call finish. 
So, be careful when using blocking functions, such as `sys.sleep` and `serial.recv`, 
inside a event function.

```lua
--- @type { pattern: string, evt_fn: function}[]
M.serial.on_send = {
    {
        -- Sample pattern
        pattern = "AT\r",
        evt_fn = function (msg)
        end
    }
    {
        -- Any message
        pattern = ".*",
        evt_fn = function (msg)
        end
    },
}
```

The on_send could also be a list of event functions. Each event function must has a pattern. 
So, a event function is called, when the sent message matches the pattern. The patterns is tested
in order it appers on the array. The messages sent to serial ends with `\n`. So the pattern, must
not contains `\n` in the end.

### `on_recv`

```lua
--- Event function called when a message is received from serial
--- 
--- @param msg string The message received from serial
function on_recv(msg)
    -- code ...
end
```

```lua
--- @type { pattern: string, evt_fn: function}[]
M.serial.on_recv = {
    {
        -- Sample pattern
        pattern = "OK\r",
        evt_fn = function (msg)
        end
    }
    {
        -- Any message
        pattern = ".*",
        evt_fn = function (msg)
        end
    },
}
```


### `on_connect`

```lua
--- Event function called when Scope connects to a serial port
--- 
--- @param port string Name of the serial port
--- @param baudrate integer Baudrate of the serial port
function on_connect(port, baudrate)
    -- code ...
end
```

```lua
--- @type { pattern: string, evt_fn: function}[]
M.serial.on_connect = {
    {
        -- Connected on Unix
        port = "/dev/tty.*",
        baudrate = ".*",
        evt_fn = function (port, baudrate)
        end
    }
    {
        -- Connected on Windows
        port = "COM[1-9][0-9]*",
        baudrate = ".*",
        evt_fn = function (msg)
        end
    },
}
```


### `on_disconnect`

```lua
--- Event function called when Scope disconnect from a serial port
--- 
function on_disconnect()
    -- code ...
end
```

## BLE

```lua
local M = {
    ble = {
        on_connect = function (uuid) end
        on_disconnect = function (uuid) end
        on_read = function (serv, char, val) end
        on_write = function (serv, char, val) end
        on_write_without_rsp = function (serv, char, val) end
        on_notify = function(serv, char, val) end
        on_indicate = function(serv, char, val) end
        on_mtu_change = function (uuid, val) end
    }, 
}
```


## User Commands

```lua
local M = { 
    cmd = {
        <cmd_name> = function (arg_1, arg_2, ...)
            -- code here
        end
        -- Examples:
--   !plugin bye
        --   bye = function()
        --       log.inf("Good bye!")
        --   end
        --  
        --   !plugin world "Matheus" 28
        --   greetings = function(name, age)
        --       log.inf("Hello, " .. name)
        --       log.inf("Are you " .. tostring(age) .. " years old?")
        --   end
    },
}
```


# Scope API

## Plugin

```lua
--- @return 
function scope.plugin.new()
end
```

## Bytes

```lua
--- @param bytes integer[]
--- @return string
function scope.bytes.to_str(bytes)
end
```

## String

```lua
--- @param str string
--- @return integer[]
function scope.str.to_bytes(str)
end
```

## Log

```lua
--- @param msg string
function scope.log.dbg(msg)
end
```

## Serial

```lua
--- @return {port: string, baudrate: integer}
function scope.serial.info()
end

--- @param msg string
function scope.serial.send(msg)
end

--- @param opts {timeout_ms: integer}
--- @return {err: integer, msg: string}
function scope.serial.recv(opts)
end
```

## System

```lua
--- @return "windows" | "unix"
function scope.sys.os_name()
end

--- @param time integer
function scope.sys.sleep_ms(time)
end
```

# Shell API


