# Plugins Developer Guide

## Plugin Interfaces

You have a idea to extend some Scope's feature. The first question that you do to yourself is: _How can I implement this new 
feature?_. Actually, you would say this: _What are the functions I need to write to implement the feature a thought?_. In this
section, I'll show to you all the possible functions you can write, on your plugin, to implement you new feature. But, before
we start the function's descriptions, I need to say that you don't need to implement all functions. You only need to implement
the functions you want.


### `on_load`

This function is called when the user runs `load` or `reload` command. On `reload` command, the function call occurs after the 
`on_unload` call. 

```lua
function M.on_load()
    -- ...
end
```

You can use this function to initialize any data or behaviour of your plugin.

### `on_unload`

This function is called when the user runs `reload` or `unload` command. On `reload` command, the function call occurs before the
`on_load` call. 

```lua
function M.on_unload()
    -- ...
end
```

You can use this function to clean-up some resources or to unlock some resources.

### `on_serial_send`

This function is called when the user, or another plugin, sends a message to serial interface. This function has one parameter: 
`msg` the message sent to serial interface. This argument is a copy os message, so it's not possible to change the original message
sent to serial. This function isn't parallel, so the next call only occurs after the current call. Thus, be careful when using 
blocking function from Scope's API.

```lua
function M.on_serial_send(msg)
    -- ...
end
```

You can use this function to show aditional infos about the sent message, to send messages automaticaly based on sent messages
or to store some metadatas about sent messages.

### `on_serial_recv`

```lua
function M.on_serial_recv(msg)
    -- ...
end
```

### `on_serial_connect`

```lua
function M.on_serial_connect(port, baudrate)
    -- ...
end
```

### `on_serial_disconnect`

```lua
function M.on_serial_disconnect()
    -- ...
end
```

### `on_ble_connect`

```lua
function M.on_ble_connect(uuid)
    -- ...
end
```

### `on_ble_disconnect`

```lua
function M.on_ble_disconnect(uuid)
    -- ...
end
```

### `on_ble_write`

```lua
function M.on_ble_write(serv, char, val)
    -- ...
end
```

### `on_ble_write_nowait`

```lua
function M.on_ble_write_nowait(serv, char, val)
    -- ...
end
```

### `on_ble_read`

```lua
function M.on_ble_read(serv, char, val)
    -- ...
end
```

### `on_ble_notify`

```lua
function M.on_ble_notify(serv, char, val)
    -- ...
end
```

### `on_ble_indicate`

```lua
function M.on_ble_indicate(serv, char, val)
    -- ...
end
```

### `on_mtu_change`

```lua
function M.on_mtu_change(uuid, val)
    -- ...
end
```

### User Commands

```lua
--- A command to greeting the user using random words
--- @param name string The user name
--- @param age number The user age
function M.greetings(name, age)
    -- ...
end
```


## Plugin API

```lua
function scope.fmt.to_str(val)
end
```

```lua
function scope.fmt.to_bytes(val)
end
```

```lua
function scope.log.debug(msg)
end
```

```lua
function scope.log.info(msg)
end
```

```lua
function scope.log.success(msg)
end
```

```lua
function scope.log.warning(msg)
end
```

```lua
function scope.log.error(msg)
end
```

```lua
function scope.serial.info()
end
```

```lua
function scope.serial.send(msg)
end
```

```lua
function scope.serial.recv(opts)
end
```

```lua
function scope.sys.os_name()
end
```
    
```lua
function scope.sys.sleep_ms(time)
end
```

```lua
function Shell.new()
end
```

```lua
function Shell:run(cmd, opts)
end
```

```lua
function Shell:exist(program)
end
```
