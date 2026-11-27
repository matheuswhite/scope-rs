# Plugin's Development Guide v1

Welcome to plugin's development guide v1. This document is going to guide you to develop plugins for `Scope` serial monitor tool. But, **What is a plugin**? A plugin is a script written in `lua` that adds new behaviors of the `Scope` serial monitor tool. You can imagine plugins as an external chip that you connect to a handheld tool to add a new feature to it.

Ok, you already know what is a plugin, and now you may be wondering: **What I can do with plugins?**. With plugins, you can:

- Simulate devices with automatic responses;
- Build test suites to send messages and check responses, automatically;
- Decode codified messages that coming from serial;
- Execute terminal programs inside `Scope`;
- And what your creativity and skill allow.

## Prerequisites

Plugins rely on the standard libraries — [scope.lua](scope.lua), [shell.lua](shell.lua) and the codecs in [codec/](codec) — which provide the `require("scope")`, `require("shell")` and `require("codec.at")` modules used throughout this guide.

You don't need to copy them next to your plugin. When you load a plugin, `Scope` copies it into `<config_dir>/scope/plugins/` (e.g. `~/.config/scope/plugins/`) and writes the standard libraries there alongside it, so `require(...)` resolves automatically. The standard libraries belong to the `Scope` version you run: a copy that differs from it is **rewritten on every load**, so don't edit them there. Because your plugin is loaded from that directory, `reload` re-copies it from the original path — keep editing the file you loaded and reload as usual. (Both libraries also live in the `plugins` folder of the `Scope` repository if you want to read them.)

## Getting Started

The best way to understand how plugins works is seeing a simple sample running. You can copy the snippet below and save it at `hello.lua` or download it from [here](hello.lua).

```lua
local serial = require("scope").serial

local M = {}

function M.on_serial_recv_line(msg)
    serial.send("Hello," .. msg)
end

return M
```

To execute this plugin you need to load it into the `Scope`. With the `Scope` open, you could type `!plugin load hello.lua`. If you remember the analogy of the chip and the handheld tool, then you need to insert the chip into handheld to it works. Likewise, we need to "insert" (or load) our plugin into our "handheld" (or the `Scope` program). With the plugin loaded, all messages will be replied. The replied message will have the following suffix: `Hello,`.

`!plugin load` lasts only for the current session. If a plugin is part of your everyday workflow, **install** it instead with `!plugin install hello.lua`: this loads it now and records it so every subsequent `Scope` session auto-loads it at start-up (the name is stored in `installed.toml`, next to the staged plugins in `<config_dir>/scope/plugins/`). Use `!plugin list` to see what's installed, and `!plugin uninstall hello` to remove one from the auto-load set (it also deletes the staged copy; a plugin already running this session stays loaded until you `!plugin unload` it or restart).

## Hello, World

Let's break down each line of the sample above. At the first line we're importing the `serial` functions from the scope standard library. We need this import to interact with the current connected serial port.

We create a local table at the third line. This local table is our plugin, and we name it `M` by a convention. We could name it as our plugin name `hello` for example. What's matter is the table must be returned at the end of file.

Through line 5 to 7 we write a function and there are two notes about this function: the function is associated to our plugin table, and it has a reserved name `on_serial_recv_line`. The former tell us that any function outside our plugin table will be ignored and the later tell us that this function will be called on every line received from serial interface (see [Receiving data](#receiving-data) for the other receive hook). Inside the function body, we're sending a message through serial, using `serial.send` function. As function argument, we're concatenating `"Hello,"` to the received message. There are two other functions for serial interaction inside scope standard library: `serial.info` which returns the configured serial port and its baud rate; and `serial.recv` which waits and returns a tuple with the error and the received message. For `serial.recv`, you can pass a table as argument to specify what is the timeout to wait. If the timeout is reach, then the error returned isn't `nil`.

The last line is already explained: it returns our plugin table. Without this line, anything inside our plugin will have effect.

## Receiving data

Two hooks receive what the serial interface reads. Define either or both:

| Hook | Called | Argument |
| --- | --- | --- |
| `on_serial_recv_line(line)` | once per received line | list of bytes (table of numbers), ending in the `\n` (a `\r` before it is kept too) |
| `on_serial_recv_byte(byte)` | once per received byte | the byte, as a number (`0`-`255`) |

Set `recv_as_string = true` in the [plugin configuration](#plugin-configuration) to get the line as a Lua string instead (the same applies to `on_serial_recv` and to the data `serial.recv`/`rtt.recv` return); the byte hook always gets a number.

Both behave exactly the same in the TUI and in headless mode (`--headless`). A line is delivered only once its `\n` arrives: text the device leaves without a line break, like a shell prompt, never reaches `on_serial_recv_line` — use `on_serial_recv_byte` to see it as it arrives. A partial line still pending when the port disconnects is discarded, and so is one that grows past 64 KiB without a `\n`.

```lua
local log = require("scope").log
local fmt = require("scope").fmt

local M = { prompt = "" }

function M.on_serial_recv_line(line)
    log.info("line: " .. fmt.to_str(line))
end

function M.on_serial_recv_byte(byte)
    M.prompt = M.prompt .. string.char(byte)
end

return M
```

### Deprecated: on_serial_recv and on_rtt_recv

`on_serial_recv(msg)` and `on_rtt_recv(msg)` still work, but are deprecated and log a warning when the plugin is loaded. They get each message in the shape the interface happened to read it, which is not the same in both modes: a whole line in the TUI, but a single byte (serial) or a read chunk (RTT) in headless mode — so a plugin that parses lines breaks under `--headless`. Rename `on_serial_recv` to `on_serial_recv_line` (and `on_rtt_recv` to `on_rtt_recv_line`) to keep line-based code working in both modes.

## RTT

Scope can also operate using **RTT** (Real-Time Transfer). When the active interface is RTT, plugins can interact with it through the `rtt` module from the Scope standard library.

Import it like this:

```lua
local rtt = require("scope").rtt
```

As with the serial APIs, RTT messages are byte-oriented. Depending on the call site, data may arrive as a Lua string or as a list of bytes (a table of numbers). If you need to convert between them, first import the Scope standard library with `require("scope")`, then use the `fmt` helpers:

```lua
local scope = require("scope")
local fmt = scope.fmt

local as_string = fmt.to_str(msg)
local as_bytes = fmt.to_bytes("hello")
```

### rtt.info()

Returns information about the current RTT session.

- Returns: `target, channel`
    - `target` (string): RTT target name.
    - `channel` (number): active RTT channel.

Notes:

- If the active interface is **not** RTT, Scope returns an empty target (`""`) and channel `0`.

### rtt.send(msg)

Sends a message through the RTT interface.

- `msg`: string or list of bytes (table of numbers).
- Returns: nothing.

### rtt.recv(opts)

Waits for the next RTT message.

- `opts` (table):
    - `timeout_ms` (number, optional): how long to wait in milliseconds. If omitted, it waits indefinitely.
- Returns: `err, data`
    - `err`: `nil` on success, or a string on error (currently the most common value is `"timeout"`).
    - `data`: list of bytes (table of numbers), or a string with `recv_as_string`. On timeout, it is empty.

Important:

- If your plugin implements `on_rtt_recv_line` or `on_rtt_recv_byte`, it will still be called for the same incoming data that unblocks `rtt.recv`. Avoid processing the same data twice.
- `rtt.recv` requires the active interface to be RTT. If another interface is active, this call may wait indefinitely and never complete.

### rtt.read(opts)

Reads raw memory from the target via the RTT backend.

- `opts` (table):
    - `address` (number): memory address to read from.
    - `size` (number): number of bytes to read (maximum: `1024`).
- Returns: `err, data`
    - `err`: `nil` on success, or a string describing the failure.
    - `data`: list of bytes (table of numbers). On error, it is an empty list.

Notes:

- This call requires the active interface to be **RTT**. If Scope is running with another interface selected, the request will immediately fail with an error indicating that RTT is not the active interface. Use `rtt.info()` to detect whether RTT is active.
- Lua numbers are typically floating-point; very large addresses may lose precision. In practice, this works best for 32-bit addresses.

### Callback: on_rtt_recv_line(msg) and on_rtt_recv_byte(byte)

The RTT counterparts of [`on_serial_recv_line` and `on_serial_recv_byte`](#receiving-data), with the same guarantees: `on_rtt_recv_line` gets each complete line (terminator included), `on_rtt_recv_byte` gets each byte as a number, identically in the TUI and in headless mode. They are called only **while the active interface is RTT**.

```lua
local scope = require("scope")
local fmt = scope.fmt
local log = scope.log

local M = {}

function M.on_rtt_recv_line(msg)
        log.info("RTT: " .. fmt.to_str(msg))
end

return M
```

Notes:

- When the active interface is RTT, the `on_serial_recv_*` hooks are not called; RTT uses `on_rtt_recv_*` instead.
- `on_rtt_recv` (no suffix) is deprecated, for the same reason as [`on_serial_recv`](#deprecated-on_serial_recv-and-on_rtt_recv).

### Callback: on_rtt_send(msg)

If your plugin table defines `on_rtt_send`, Scope will call it automatically every time a message is sent to RTT **while the active interface is RTT**.

```lua
local scope = require("scope")
local fmt = scope.fmt
local log = scope.log

local M = {}

function M.on_rtt_send(msg)
        log.info("RTT sent: " .. fmt.to_str(msg))
end

return M
```

Notes:

- When the active interface is RTT, `on_serial_send` is not called; RTT uses `on_rtt_send` instead.

## Analytics Plugin

After understand the basic plugin sample shown above, let's move on to a more complex and functional sample. Let's build an analytics plugin. You can use the code of `hello.lua` as base and edit the same file or duplicate the file and rename it to `analytics.lua`. This plugin is going to count the number of times we receive and send a message through the serial port. We already use `on_serial_recv_line` on the previous sample to get the received messages. To get the messages sent we're going to use `on_serial_send` function. See the snippet below:

```lua
local serial = require("scope").serial

local M = {}

function M.on_serial_recv_line(msg)
    serial.send("Hello," .. msg)
end

function M.on_serial_send(msg)

end

return M
```

Now, to count the messages sent and received let's create two variables to store these values. We could create this globally, however is better to create that inside our plugin table `M`. When the plugin is loaded by `Scope`, all global variable is lost, including the plugin table `M`. This is the reason because we return the plugin table `M`. So, the following snippet is going to show the two new variables: `recv` and `send`.

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
}

function M.on_serial_recv_line(msg)
    serial.send("Hello," .. msg)
end

function M.on_serial_send(msg)

end

return M
```

As we're planning, let's increase these values to register our analytics. So, inside `on_serial_recv_line` and `on_serial_send` function body we'll increase `recv` and `send` respectively. The bellow snippet shows where to increase these variables:

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
}

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)
end

function M.on_serial_send(msg)
    M.send = M.recv + 1
end

return M
```

Ok, we've already saved the amount of messages sent and received, however we can't access these values. To access these values we could save them into a file and saw it later. To save it into a file we're going to use the standard Lua file API. The following snippet is showing the new version of our plugin with file persistence.

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
}

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    local file = io.open('analytics.txt', 'w')
    file:write(tostring(M.send) .. '\n' .. tostring(M.recv))
    file:close()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    local file = io.open('analytics.txt', 'w')
    file:write(tostring(M.send) .. '\n' .. tostring(M.recv))
    file:close()
end

return M
```

If you notice, we're duplicating the file persistence code. Let's move it from functions body to its local own function.

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
}

local function save()
    local file = io.open('analytics.txt', 'w')
    file:write(tostring(M.send) .. '\n' .. tostring(M.recv))
    file:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

return M
```

Our plugin is almost done. We've already counted and saved the collected values. But, there is a little issue here: each time we receive and send a message we're opening and closing a file. This slow down our plugin. To fix that we need to open and close the file once. For this feature we'll use two new functions: `on_load` and `on_unload`. The `on_load` is called when the plugin is loaded and `on_unloaded` is called when the plugin is unloaded or the scope is closed. In the next snippet we're going to open the file at `on_load` and close it at `on_unload`.

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

return M
```

With that, we finish our analytics plugin. If you notice, we use a lot of functions that starts with `on_`. Functions inside our plugin's table that have `on_` prefix are called as **Event Callbacks**. The `Scope` calls these functions automatically when its conditions are matched. You shouldn't start your custom functions by `on_` to prevent confusion with event callbacks.

## Logs

There is a detail we don't check: the file open result. We need to check whether the result isn't `nil` and show an error to the user if it is. To show the error, we can import the `log` functions from scope standard library. These functions show messages inside the `Scope` main view (where the serial messages are displayed). As we need to show an error, we'll use the `log.error` function to print a message in red.

```lua
local serial = require("scope").serial
local log = require("scope").log

local M = {
    recv = 0,
    send = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

return M
```

There are other functions to show messages to the user:

- `log.debug`: To print debug messages in cyan;
- `log.info`: To print info messages in white;
- `log.success`: To print success messages in green;
- `log.warning`: To print warning messages in yellow;
- `log.error`: To print error messages in red.

## Commands

We can enhance our plugin adding a way to print the amount of messages received and sent. To implement this feature, we can add a command to our plugin. To add a command to a plugin, you only need to add a function to the plugin's table. The function name will be the name of the command. Let's create a command called `data` to show the analytics values.

```lua
local serial = require("scope").serial
local log = require("scope").log

local M = {
    recv = 0,
    send = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
```

To call this function we need to use `!` followed by: plugin's name, the command name and its arguments (split by spaces). For example, if your plugin name is `analytics.lua` you can call the `data` command using this command:

```
!analytics data
```

If you call this command, and it didn't work it's because you don't reload the plugin. Each change made inside the plugin's source code doesn't make effect until you reload the plugin. To reload the plugin (assuming its name as `analytics.lua`) you can run the command below:

```
!plugin load analytics.lua
```

## Regex

A good statistic for our analytics plugin is how many times a message that starts with `AT` appears. This is a good way to check how many AT commands has sent. First, let's import the regex functions from scope standard library.

```lua
local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re

local M = {
    recv = 0,
    send = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
```

After that, let's add the variable `at` to our plugin table. This variable will count how many times we detect an AT command.

```lua
local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re

local M = {
    recv = 0,
    send = 0,
    at = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
```

At `M.on_serial_send` we'll use the `re.match` function to check if the message starts with `AT`. The below snippet shows this inclusion.

```lua
local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re

local M = {
    recv = 0,
    send = 0,
    at = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    if re.match(msg, "AT.*") then
        M.at = M.at + 1
    end

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
```

We also could count the AT commands that contains `AT+`. However, the `+` character is a reserver symbol in regex. We could escape it using reverse slash `\` or using the `re.literal` function. This function will escape every special character for us.

```lua
local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re

local M = {
    recv = 0,
    send = 0,
    at = 0,
    at_plus = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
    M.recv = M.recv + 1
    serial.send("Hello," .. msg)

    save()
end

function M.on_serial_send(msg)
    M.send = M.recv + 1

    if re.match(msg, "AT.*") then
        M.at = M.at + 1
    end

    if re.match(msg, re.literal("AT+")) then
        M.at_plus = M.at_plus + 1
    end

    save()
end

function M.data()
    log.info("Tx: " .. tostring(M.send) .. ", Rx: " .. tostring(M.recv))
end

return M
```

If we add more patterns to match it'll become extensive and hard to maintain. There is a special function to help us. The function `re.matches` gets an input and matches against a list of pairs. Each pair must have a pattern followed by a function. If the pattern matches it'll call the associated function. Note that the `re.matches` will use the first matched pattern and will stop on that. The following snippet shows this function usage.

```lua
local serial = require("scope").serial
local log = require("scope").log
local re = require("scope").re

local M = {
    recv = 0,
    send = 0,
    at = 0,
    at_plus = 0,
    number = 0,
    f = nil
}

local function save()
    M.f:write(tostring(M.send) .. '\n' .. tostring(M.recv))
end

function M.on_load()
    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
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
```

## Shell

If you need to call a shell command inside your lua plugin, you need to create a shell session. This session is isolated from any other shell session, and it lives while the plugin is loaded. First we need to import the lua file [shell.lua](shell.lua). After that, we're going to create a new shell session with the `shell.new` function. This function will create an object to us. This object is our shell session.

```lua
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

    M.f = io.open('analytics.txt', 'w')

    if M.f == nil then
        log.error("File analytics.txt can't be opened")
    end
end

function M.on_unload()
    M.f:close()
end

function M.on_serial_recv_line(msg)
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
```

After this point, we'll always use `M.shell` object's method. To use methods in lua we use `:` instead of `.`. If you swap these symbols the code won't work. The shell object `M.shell` have two methods: `exist` which checks if a program exists, and `run` to run a command. We'll use `run` to run an echo. The output of the program is printed at the `Scope` main view.

```lua
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

function M.on_serial_recv_line(msg)
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
```

## System info

There is a set of functions to help the developer. This function are inside the `sys` of the scope standard library.

The function `os_name` returns the name of running operating system. If the OS is a Windows version, it'll return `windows` otherwise it'll return `unix`.

The function `sleep_ms` sleeps the current function for **x** milliseconds. Be careful with this function, because it could slow down the plugin execution.

And last but not least, we have the function `parse_args`. This is a helper function to check the input arguments of a custom command. It receives a list of tables. Each table check one argument. There are 2 mandatory fields for each table: `arg` which is the argument name and `ty`, its type. There are 3 possible values for `ty`: `string`, `number` and `boolean`. In addition to these mandatory fields, there are 2 other optional fields: `default` which replace a missing argument, and `validate` that runs to check if the input argument is valid. If it's not valid, so the `lua` assert is called and the command isn't run.

## Plugin configuration

A plugin can tune how `Scope` treats it through a `config` table in the plugin table. Define it with the table, or change its fields later (in `on_load`, in a command, in `setup_suite`…): `Scope` creates `M.config` before calling `on_load`, fills in the defaults for the fields you leave out, and reads each field when it is used.

```lua
local M = {
    config = {
        recv_as_string = true,
        default_timeout = 500,
    },
}

function M.on_load()
    M.config.error_pattern = "ERROR|\\+CME ERROR: .*"
end
```

| Field | Default | Meaning |
| --- | --- | --- |
| `recv_as_string` | `false` | Deliver received data as a Lua string instead of a list of bytes (see [Receiving data](#receiving-data)). |
| `default_timeout` | `1000` | Milliseconds `send_and_wait` waits when the call gives no timeout. |
| `default_expected` | `"OK"` | Pattern `send_and_wait` waits for when the call gives none. |
| `error_pattern` | none | Pattern of a line that makes `send_and_wait` fail at once. |
| `ignore_msg` | `{}` | Patterns of lines `send_and_wait` drops (e.g. the device's own logs). |
| `strip_msg` | `{}` | Patterns `send_and_wait` removes from every received line, wherever they appear (e.g. a shell prompt). |
| `ignore_echo` | `true` | Drop the device's echo of the message `send_and_wait` sent. |
| `send_prefix` | none | Text `send_and_wait` puts before each message, followed by a space unless it already ends with one. |
| `send_quoted` | `false` | Put the message between double quotes, escaping the ones inside, before the prefix. |
| `line_ending` | `"\r\n"` | Text `send_and_wait` puts after each message. |
| `retries` | `0` | How many times `send_and_wait` sends the message again after a failure. |
| `retry_interval` | `1000` | Milliseconds between two attempts. |
| `retry_pattern` | none | Pattern of the error lines worth a retry; without it, every failure (timeout or error line) is retried. |

## Sending data

`serial.send(msg)` and `rtt.send(msg)` take either a string or a list of bytes, and send it as is: `serial.send("AT\r\n")` and `serial.send({ 0x41, 0x54, 0x0d, 0x0a })` put the same bytes on the wire.

### serial.send_and_wait(msg, expected, timeout_ms)

Sends `msg` and waits for the device to answer with a line matching `expected`. `rtt.send_and_wait` is the same over RTT.

- `msg`: string or list of bytes. With `config.send_quoted` it is put between quotes; then `config.send_prefix` (and a space) is put before it and `config.line_ending` after it.
- `expected` (optional): pattern of the line that ends the wait; `config.default_expected` (`"OK"`) when omitted.
- `timeout_ms` (optional): how long to wait; `config.default_timeout` when omitted.
- Instead of `expected` and `timeout_ms`, the second argument can be a table of options: `expected`, `timeout`, and any of `error_pattern`, `ignore_msg`, `strip_msg`, `ignore_echo`, `send_prefix`, `send_quoted`, `line_ending`, `retries`, `retry_interval`, `retry_pattern`, each overriding the [config](#plugin-configuration) field of the same name for this call. `serial.send_and_wait('AT+QESIM="list",1', { timeout = 10000 })` waits longer for one slow command.
- Returns: `lines, matched`
    - `lines`: the lines received between the send and the expected line, as strings without their `\r\n`.
    - `matched`: the expected line itself.
- On timeout, or when a line matches `config.error_pattern`, it returns `nil` and the reason: `"timeout"` or the error line. With `config.retries`, it first sends the message again, `config.retry_interval` milliseconds later, as long as the error line matches `config.retry_pattern` (any failure, without it).

Each received line is cleaned before it is matched or returned: ANSI color codes are removed, then every match of `config.strip_msg`, and trailing spaces. A line is then left out when it is empty, when it is the device's echo of what was sent (the message itself, or a line ending with a space and the message, as after a shell prompt; turn this off with `ignore_echo = false`), or when it matches `config.ignore_msg`.

Only one `send_and_wait` is on the wire at a time, whichever plugin calls it: a call made while another one waits is sent once that one ends, and its timeout starts when it is sent. A reply therefore always answers the call that sent its command.

The patterns are regular expressions (the same syntax as [Regex](#regex)) that must match the **whole** line, except `strip_msg`, which removes every match: `"OK"` matches `OK`, but not `+CPBR: 1,"BOOK"`. Write `.*` to match part of a line, e.g. `"\\+CME ERROR: .*"`.

```lua
local log = require("scope").log
local serial = require("scope").serial

local M = {
    config = {
        send_prefix = "modem at",
        send_quoted = true,
        strip_msg = { "uart:~\\$ ?" },
        error_pattern = "ERROR|\\+CME ERROR: .*|script is already running",
        retries = 2,
        retry_pattern = "script is already running",
    },
}

-- Sends 'modem at "AT+CSQ"\r\n'; on success, lines is { "+CSQ: 20,99" }.
function M.csq()
    local lines, err = serial.send_and_wait("AT+CSQ")
    if lines == nil then
        log.error("AT+CSQ failed: " .. err)
    end
end

return M
```

## Strings

`fmt.split(str, sep, max)` splits a string (or a list of bytes) at each `sep`, taken literally (not as a pattern), and returns the pieces as multiple values. Empty pieces are kept. `max` (optional) caps the number of pieces: the last one keeps the rest of the string.

```lua
local fmt = require("scope").fmt

local prefix, args = fmt.split('+COPS: 0,0,"TIM",7', ":", 2)  -- "+COPS", ' 0,0,"TIM",7'
local fields = { fmt.split("a,,b", ",") }                     -- { "a", "", "b" }
```

`fmt.starts_with(str, prefix)` and `fmt.ends_with(str, suffix)` tell whether a string (or a list of bytes) starts or ends with another one, taken literally.

## Tests

Every function of the plugin table whose name starts with `test_` is a test. Run them from the command bar:

| Command | Runs |
| --- | --- |
| `!test <plugin>` | every test of the plugin, in the order they are defined |
| `!test <plugin> <test> ...` | only the tests named |
| `!test <test> ...` | the tests named, in the loaded plugin that defines them |

A test name may be written with or without the `test_` prefix (`!test cops` and `!test test_cops` are the same). A loaded plugin name wins over a test name; when two loaded plugins define the test, name the plugin. Calling a test as a regular command (`!at test_cops`) still runs the bare function, without the hooks below.

The asserts live in the `test` module of the standard library. A failed assert stops the test, which is reported as failed with the line of the assert:

| Function | Fails when |
| --- | --- |
| `test.assert(cond, msg)` | `cond` is `nil` or `false` |
| `test.assert_eq(actual, expected, msg)` | the values differ (tables are compared by content) |
| `test.assert_opt(value, options, msg)` | `value` is not one of the list `options` |
| `test.assert_re(value, pattern, msg)` | `value` does not match the regex `pattern` as a whole |
| `test.skip(reason)` | always: stops the test and reports it as skipped |

`msg` is optional. A Lua error raised by the test (`error(...)`, Lua's own `assert`, indexing `nil`…) also fails it.

Four optional functions run around the tests:

| Function | Called |
| --- | --- |
| `setup_suite()` | once before the tests; what it returns is the *fixture*, passed to every test and hook |
| `setup_test(name, fixture)` | before each test |
| `teardown_test(index, total, name, result, fixture)` | after each test; `result` is `"pass"`, `"fail"` or `"skip"` |
| `teardown_suite(results, fixture)` | once after the tests; `results` has the counts `ok`, `fail`, `skip` and `total`, and `tests`, a list of `{ name, result }` |

`name` is the test name without the `test_` prefix. Without `teardown_test`/`teardown_suite`, `Scope` logs each result and a summary. When `setup_suite` fails (or calls `test.skip`), no test runs: every one is counted as failed (or skipped) and `teardown_suite` is still called. A failing `setup_test` fails its test.

```lua
local scope = require("scope")
local at = scope.codec.at
local log = scope.log
local serial = scope.serial
local test = scope.test

local M = {}

function M.setup_suite()
    M.config.error_pattern = "\\+CME ERROR: .*"
    M.config.default_timeout = 300
end

function M.teardown_suite(results)
    log.info(results.ok .. " passed, " .. results.fail .. " failed, " .. results.skip .. " skipped")
end

function M.test_at()
    test.assert(serial.send_and_wait("AT"))
end

function M.test_cops()
    local lines, err = serial.send_and_wait("AT+COPS?")
    test.assert(lines, err)

    local cops = at.decode(lines)
    test.assert(cops)
    test.assert_opt(cops.mode, { 0, 1, 2, 4 })
    test.assert_eq(cops.act, 7)
end

return M
```

## Codecs

The `codec` table of the standard library encodes and decodes serial protocols.

### AT commands

`scope.codec.at` (also `require("codec.at")`) handles the AT commands of 3GPP TS 27.007.

`at.encode(cmd, ...)` builds a set or execute command, with `cmd` as written after `AT`: numbers are written as is, strings are quoted, `true`/`false` become `1`/`0` and `nil` leaves the parameter empty (trailing ones are dropped). `at.encode_read(cmd)` and `at.encode_test(cmd)` build the read (`?`) and test (`=?`) forms. No line ending is added: `send_and_wait` adds it.

```lua
at.encode("+CGDCONT", 1, "IP", "apn")  -- AT+CGDCONT=1,"IP","apn"
at.encode("+CIMI")                     -- AT+CIMI
at.encode("E0")                        -- ATE0
at.encode_read("+COPS")                -- AT+COPS?
at.encode_test("+COPS")                -- AT+COPS=?
```

`at.decode(lines, cmd)` turns a response into Lua tables:

- `lines` is the list of lines `send_and_wait` returned. The result is the response of the command: one table, or for a command that answers one line per entry (`+CGDCONT`, `+CGACT`, `+CGPADDR`), a list indexed by the command's key (`cid`): `contexts[1]` is context 1.
- `cmd`, the command sent, is optional. It is needed for a response without a `+NAME:` prefix, like the IMSI `AT+CIMI` answers, and to pick the spec of a subcommand (see `at.define`).
- `lines` can also be a single line (string or list of bytes), decoded into one table.

Each table has the command `id` (without the `+`), a field per parameter holding its value, `params` (every value, by position) and `raw` (each parameter's text as received, by position and by field name). Quoted parameters become strings, unquoted numbers become integers. On failure `decode` returns `nil` and the reason.

```lua
local lines = serial.send_and_wait("AT+COPS?")
local cops = at.decode(lines)
-- cops.id == "COPS", cops.provider == "TIMBrasil", cops.act == 7
-- cops.raw.provider == '"TIMBrasil"', cops.params[4] == 7

lines = serial.send_and_wait("AT+CGDCONT?")
local contexts = at.decode(lines)
-- contexts[1].apn == "", contexts[1].pdp_type == "IP"

lines = serial.send_and_wait("AT+CIMI")
local cimi = at.decode(lines, "AT+CIMI")
-- cimi.imsi == "724041234567890"
```

Keep the lines in a variable, as above: `at.decode(serial.send_and_wait(cmd))` would also pass `send_and_wait`'s second result, the matched line, as `cmd`, which `decode` rejects with an error.

The commands below are decoded into named fields, and a value outside the range the standard defines makes `decode` fail. Any other `+NAME: ...` line is decoded into `params` only, and the final result codes (`OK`, `ERROR`, `NO CARRIER`, …) into `{ id = "OK" }`.

| Response | Fields |
| --- | --- |
| `+COPS` | `mode`, `format`, `provider`, `act` |
| `+CREG`, `+CGREG` | `n` (read response only), `stat`, `lac`, `ci`, `act` |
| `+CEREG` | `n` (read response only), `stat`, `tac`, `ci`, `act` |
| `+CSQ` | `rssi`, `ber` |
| `+CPIN` | `code` |
| `+CFUN` | `fun` |
| `+CGATT` | `state` |
| `+CGACT` | `cid`, `state` (one per context) |
| `+CGDCONT` | `cid`, `pdp_type`, `apn`, `pdp_addr`, `d_comp`, `h_comp` (one per context) |
| `+CGPADDR` | `cid`, `pdp_addr`, `pdp_addr_2` (one per context) |
| `AT+CIMI` | `imsi` |
| `+CME ERROR`, `+CMS ERROR` | `err` (a number, or the text in verbose mode) |

`lac`, `tac` and `ci` are hexadecimal strings in the response; their value is the number.

`at.text(id, field, value)` describes an enumerated value: `at.text("CEREG", "stat", 1)` is `"registered, home"`, `at.text("COPS", "act", 7)` is `"E-UTRAN"`. `at.esm_cause(cause)` describes a session management cause (3GPP TS 24.008 / 24.301), as `+CEER` reports it after a failed data call: `at.esm_cause(27)` is `"missing or unknown APN"`.

`at.define(id, spec)` declares how to decode another command, such as a vendor one. `id` is the name after `AT+`, or the name and its first parameter when the response changes with it (`'QCFG="roamservice"'`, matched against the `cmd` given to `decode`). `spec` lists the fields in order, as `{ name, kind, check, text = { [value] = description } }`, where `kind` is `"int"` (a number, quoted or not), `"str"` (text, quotes removed), `"hex"` (a hexadecimal string, as a number) or `"any"`, and `check` (optional) is a function that returns whether the value is valid. These options apply to the whole response:

| Option | Meaning |
| --- | --- |
| `list = true` | The command answers one line per entry: `decode` returns a list. |
| `key = "<field>"` | Index that list by this field instead of by position. |
| `unprefixed = true` | The lines carry no `+NAME:` prefix. |
| `required = <n>` | The first `n` fields must be present. |

```lua
at.define("QNWINFO", { { "act", "str" }, { "oper", "str" }, { "band", "str" }, { "channel", "int" } })
at.define('QCFG="roamservice"', { { "setting", "str" }, { "mode", "int" } })
at.define('QESIM="list"', {
    list = true,
    unprefixed = true,
    { "iccid", "str" }, { "state", "int" }, { "nickname", "str" }, { "provider", "str" },
})

local cmd = 'AT+QCFG="roamservice"'
local lines = serial.send_and_wait(cmd)
local roam = at.decode(lines, cmd)
-- roam.mode == 2
```
