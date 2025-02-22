# Plugin's Development Guide v1

Welcome to plugin's development guide v1. This document is going to guide you to develop plugins for `Scope` serial monitor tool. But, **What is a plugin**? A plugin is a script written in `lua` that adds new behaviors of the `Scope` serial monitor tool. You can imagine plugins as an external chip that you connect to a handheld tool to add a new feature to it.

![Sketch handheld with chip]()

Ok, you already know what is a plugin, and now you may be wondering: **What I can do with plugins?**. With plugins, you can:

- Simulate devices with automatic responses;
- Build test suites to send messages and check responses, automatically;
- Decode codified messages that coming from serial;
- Execute terminal programs inside `Scope`;
- And what your creativity and skill allow.

## Prerequisites

Before we start to develop plugins, we need two files: [scope.lua](scope.lua) and [shell.lua](shell.lua). These two files could be found at `plugins` folder of the `Scope` repository. These files must be at same folder of our plugin. Think that files as the standard libraries of our plugin.

## Getting Started

The best way to understand how plugins works is seeing a simple sample running. You can copy the snippet below and save it at `hello.lua` or download it from [here](hello.lua).

```lua
local serial = require('scope').serial

local M = {}

function M.on_serial_recv(msg)
    serial.send("Hello," .. msg)
end

return M
```

To execute this plugin you need to load it into the `Scope`. With the `Scope` open, you could type `!plugin load hello.lua`. If you remember the analogy of the chip and the handheld tool, then you need to insert the chip into handheld to it works. Likewise, we need to "insert" (or load) our plugin into our "handheld" (or the `Scope` program). With the plugin loaded, all messages will be replied. The replied message will have the following suffix: `Hello,`.

## Hello, World

Let's break down each line of the sample above. At the first line we're importing the `serial` functions from the scope standard library. We need this import to interact with the current connected serial port.

We create a local table at the third line. This local table is our plugin, and we name it `M` by a convention. We could name it as our plugin name `hello` for example. What's matter is the table must be returned at the end of file.

Through line 5 to 7 we write a function and there are two notes about this function: the function is associated to our plugin table, and it has a reserved name `on_serial_recv`. The former tell us that any function outside our plugin table will be ignored and the later tell us that this function will be called on every message received from serial interface. Inside the function body, we're sending a message through serial, using `serial.send` function. As function argument, we're concatenating `"Hello,"` to the received message. There are two other functions for serial interaction inside scope standard library: `serial.info` which returns the configured serial port and its baud rate; and `serial.recv` which waits and returns a tuple with the error and the received message. For `serial.recv`, you can pass a table as argument to specify what is the timeout to wait. If the timeout is reach, then the error returned isn't `nil`.

The last line is already explained: it returns our plugin table. Without this line, anything inside our plugin will have effect.

## Analytics Plugin

After understand the basic plugin sample shown above, let's move on to a more complex and functional sample. Let's build an analytics plugin. You can use the code of `hello.lua` as base and edit the same file or duplicate the file and rename it to `analytics.lua`. This plugin is going to count the number of times we receive and send a message through the serial port. We already use `on_serial_recv` on the previous sample to get the received messages. To get the messages sent we're going to use `on_serial_send` function. See the snippet below:

```lua
local serial = require("scope").serial

local M = {}

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
    serial.send("Hello," .. msg)
end

function M.on_serial_send(msg)

end

return M
```

As we're planning, let's increase these values to register our analytics. So, inside `on_serial_recv` and `on_serial_send` function body we'll increase `recv` and `send` respectively. The bellow snippet shows where to increase these variables:

```lua
local serial = require("scope").serial

local M = {
    recv = 0,
    send = 0,
}

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
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

function M.on_serial_recv(msg)
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

- [ ] Explain regex single match

- [ ] Explain regex literal

- [ ] Explain regex multiple matches

## Serial Functions

- [ ] Show which types exist and how each one works

- [ ] Explain `on_` prefix reserved for future use

## System Infos

- [ ] Explain os_name

- [ ] Explain sleep

- [ ] Explain parse_args

## Run shell commands

- [ ] Explain run and exist functions
