<p align="center">
    <br><img src="imgs/scope-logo.png" width="800">
</p>

### Send Data

With `Scope` serial monitor you can use the command bar (at bottom of screen) to send data.

![Send data gif]()

### Send in Hexadecimal

You also can send data in hexadecimal format. To do it, type `$` and write your hexadecimal data. The hexadecimal
could have spaces and commas inside (the `Scope` will send the data without spaces and commas).

![Send hex gif]()

### Send Commands

You can also send commands using the command bar. To send a command, type `/` and a list of all available commands is
shown above the command bar. Continue typing the command and hit `Enter` to send the command.

![Send command gif]()

The commands are loaded from a user YAML file, passed at start of program (using `-c` flag). An example of YAML file is
shown below:

```yaml
hello: 'world!'
spaces: 'a big frase with spaces'
double_quotes: '"double"'
single_quotes: "'single'"
json: '{"cmd":1, "args":[true, "hello", 2.1]}'
```

### Written History

Is possible to retrieve old data and commands sent. You can hit `Up Arrow` and `Down Arrow` to navigate through
the history of sent data and commands.

![Command history]()

### Auto Reconnect

The `Scope` has an auto-reconnect feature. When the serial port isn't available, the `Scope` will keep trying to
reconnect to serial port, until it's available again.

![Reconnect gif]()

### Colorful

The scope use color to transmit status of connection at the command bar: red to disconnected and green to connected.
Beyond status, the content read and written are colored too, to help understand. The value read is colored using ANSI
terminal color standard.

![Read ANSI color gif]()

The data sent to serial port always been use background color to differentiate it from read data.

![Written color gif]()

Characters outside the printable range of ASCII table are shown in magenta and in the hexadecimal format. Some
characters are printed as its representation, such as: '\n', '\r' and '\0'

![Special character gif]()

### Message Timestamp

All the data written and read has a timestamp when its was sent or captured. This time is shown at left of the message,
in gray. It has: the hour, the minutes, the seconds, and the milliseconds (after the dot).

![Timestamp gif]()

### Multiplatform

You can use `Scope` on multiple platforms, like: Linux, Windows and macOS*.

*Not tested yet

## Scope vs Others

The `Scope` combine multiple features. The table below list these features:

| Features                    | Scope (Free) | Docklight | Arduino | Tera Term | screen   | esp-idf  |
|-----------------------------|--------------|-----------|---------|-----------|----------|----------|
| Send Data                   | ✅            | ✅         | ✅       | ✅         | ✅        | ✅        |
| Send in Hexadecimal         | ✅            | ✅         | x       | x         | x        | x        |
| Send Commands               | ✅            | ✅         | x       | x         | x        | x        |
| Written History             | ✅            | ✅*        | x       | x         | x        | x        |
| Auto Reconnect              | ✅            | ✅         | x       | ✅         | x        | x        |
| Colorful                    | ✅            | x         | x       | ✅         | ✅        | ✅        |
| Message Timestamp           | ✅            | ✅         | x       | x         | x        | x        |
| Display non-printable chars | ✅            | ✅         | x       | x         | x        | x        |
| Multiplatform               | ✅            | Windows   | ✅       | Windows   | Linux    | ✅        |
| Interface                   | TUI          | GUI       | GUI     | GUI       | Terminal | Terminal |
| Price                       | Free         | €69       | Free    | Free      | Free     | Free     |

*The Docklight has a list of commands in lateral panel, so it doesn't need a command history

## Installation

You can use `cargo` to download and compile for your OS or download a pre-built binary at [Releases]() page

### Using `cargo`

```shell
cargo install scope
```

## Getting Started

After the installation, type `scope` followed by the serial port and the desired baud rate. For example, to open
the port `COM3` at `115200 bps` type:

```shell
scope COM3 115200
```

When the command bar at bottom is green, you can start to capture and send messages via serial port.

To load a list of command, from a YAML file, use cloud type `-c <YOUR_COMMANDS>.yml` or `--cmd-file <YOUR_COMMANDS>.yml`
between `scope` and the serial port. For example, to load `cmd.yml` file, use can type:

```shell
scope -c cmd.yml COM3 115200
```

or

```shell
scope --cmd-file cmd.yml COM3 115200
```

## Project Goals

This project has 5 pillars that will towards the development of this tool:

I. **Intuitive Usage:** The usage of the tool must be intuitive. This means implement the use of the
most established form, used in other tools. For example, the history navigation (`Up Arrow` and `Down Arrow`) follows
the history navigation of OS terminal like in the Unix shell and in the Windows Powershell.
<br>II. **Orthogonal Features:** A new feature must not share the behaviour with other features already implemented. So,
if two features share the same behaviour a third feature must be extracted from the others features. Orthogonal features
guarantee that problematic features doesn't affect other features. It also improves the ease of tool usage.
<br>III. **User Centric Development:** New features must deliver value to user in first place, instead of please the
developers of this tool. For example, the script language used to extend the tool must be a consolidated programming
language, instead of creating a new language. Other example, it's prioritize critical bugs related by the users,
instead of launch new features.
<br>IV. **Multiplatform:** All releases must work in Windows, Linux (zsh, shell and fish) and macOS.
<br>V. **Extensible:** Support user scripts to extend the base functionalities. These scripts are called plugins. For
more information about plugins see [Plugins Developer Guide]()

The roadmap, with next releases, cloud be found in [GitHub project](https://github.com/users/matheuswhite/projects/5)
of this tool.

## Community

For new feature requests and to report a bug, feel free to post a
new [issues](https://github.com/matheuswhite/scope-rs/issues) in GitHub.

## Contributing

Take a look at the [CONTRIBUTING]() guide

## Maintainers

+ [Matheus T. dos Santos](https://github.com/matheuswhite)

## Acknowledges

+ [Emilio Bottoni](https://github.com/MilhoNerfado) for be a heavy tester of this tool;
+ [José Gomes](https://github.com/JoseGomesJr) for some features and tests.

## License

Copyright (c) 2023 Matheus Tenório dos Santos

Scope is made available under the terms of BSD v3 Licence.

See the [LICENCE](https://github.com/matheuswhite/scope-rs/blob/main/LICENSE) for license details.
