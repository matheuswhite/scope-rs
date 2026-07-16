# Contributing to Scope

First off — thank you for taking the time to contribute! `Scope` is a
community-driven serial monitor, and every bug report, idea, plugin, and pull
request helps make it better.

This guide explains how to get involved, set up a development environment, and
get your changes merged. It's meant to be read top to bottom the first time,
and skimmed afterwards.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Project philosophy](#project-philosophy)
- [Ways to contribute](#ways-to-contribute)
- [Before you start](#before-you-start)
- [Reporting bugs](#reporting-bugs)
- [Requesting features](#requesting-features)
- [Development setup](#development-setup)
- [Project layout](#project-layout)
- [Coding conventions](#coding-conventions)
- [Testing and the quality checklist](#testing-and-the-quality-checklist)
- [Commit and branch conventions](#commit-and-branch-conventions)
- [Opening a pull request](#opening-a-pull-request)
- [Contributing plugins](#contributing-plugins)
- [Documentation and demo GIFs](#documentation-and-demo-gifs)
- [License](#license)
- [Getting help](#getting-help)

## Code of conduct

This project and everyone participating in it is governed by the
[Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to
uphold it. Please report unacceptable behavior to the maintainer at
<matheuswhite1@protonmail.com>.

## Project philosophy

`Scope` is guided by five pillars (see
[Project Goals](README.md#project-goals) for the full text). Keep them in mind
when proposing or reviewing changes:

1. **Intuitive usage** — behavior should follow the conventions of popular
   tools (e.g. `Up`/`Down` navigating history like a shell).
2. **Compactness and orthogonality** — prefer small, composable features over
   large, overlapping ones.
3. **User-centric development** — deliver value to users first; prioritize
   critical, user-reported bugs over new features.
4. **Multiplatform** — every release must work on Linux, Windows, and macOS.
5. **Extensible** — favor user-scriptable extension points (Lua plugins) over
   hard-coding niche behavior.

## Ways to contribute

You don't have to write Rust to help:

- **Report bugs** you hit while using `Scope`.
- **Request features** or share use cases we haven't considered.
- **Improve the documentation** — the README, this guide, or the
  [Plugins Developer Guide](plugins/README.md).
- **Write or share plugins** (see [Contributing plugins](#contributing-plugins)).
- **Test on your platform / hardware** — different OSes, serial adapters, and
  RTT probes surface bugs the maintainers can't reproduce.
- **Fix bugs or implement features** with a pull request.

## Before you start

- Browse the [open issues](https://github.com/matheuswhite/scope-rs/issues) and
  the [roadmap project](https://github.com/users/matheuswhite/projects/5) to see
  what's already planned or in progress, and to avoid duplicating work.
- For anything larger than a small fix, **open an issue first** (or comment on
  an existing one) to discuss the approach before you write code. It saves
  everyone time and avoids surprises at review.
- Small, self-contained fixes (typos, obvious bugs) can go straight to a pull
  request.

## Reporting bugs

Open a [bug report](https://github.com/matheuswhite/scope-rs/issues/new/choose)
and include as much of the following as you can:

- **What happened** and **what you expected** to happen.
- **Steps to reproduce** — the exact command you ran (e.g.
  `scope serial /dev/ttyUSB0 115200`) and what you typed.
- **Environment** — your OS and version, the `Scope` version
  (`scope --version`), and whether you were using a serial port or RTT.
- **Logs or a screenshot/GIF** of the TUI, if relevant. You can save the
  session with `Ctrl+S` and attach the `.txt`.

## Requesting features

Open a [feature request](https://github.com/matheuswhite/scope-rs/issues/new/choose)
describing the problem you're trying to solve (not just the solution you have in
mind), who it helps, and how it fits the [project philosophy](#project-philosophy).

## Development setup

**Prerequisites:**

- **Rust `1.92.0` or newer** (the crate uses edition 2024). Install via
  [rustup](https://rustup.rs/).
- **Linux only:** the `libudev` development headers —
  `sudo apt-get install libudev-dev` on Debian/Ubuntu.
- **For the RTT interface:** a debug probe supported by
  [`probe-rs`](https://probe.rs/) (J-Link, ST-Link, CMSIS-DAP, …). Not needed
  for serial-only work.

**Build and run:**

```shell
git clone https://github.com/matheuswhite/scope-rs
cd scope-rs

cargo build                                   # debug build -> target/debug/scope
cargo run --bin scope -- list                 # list available serial ports
cargo run --bin scope -- serial /dev/ttyUSB0 115200   # open a serial port
cargo run --bin scope -- rtt STM32F303 0      # attach to an RTT target
```

## Project layout

`Scope` (crate `scope-monitor`) is a **binary-only** crate — there is no library
target, so use `cargo test --bin scope`, not `cargo test --lib`. It's built as a
multi-threaded actor system; the main subsystems live under `src/`:

| Path | Responsibility |
|------|----------------|
| `src/main.rs` | Wires up the tasks and CLI (`app_serial` / `app_rtt`). |
| `src/interfaces/` | Owns the serial port / RTT connection. |
| `src/inputs/` | The command bar: keystrokes, history, search. |
| `src/graphics/` | Renders the TUI, scrollback, selection, session saving. |
| `src/plugin/` | Hosts the Lua plugin engine. |
| `src/infra/` | Shared plumbing: tasks, channels, logger, tags, config. |

For a deeper architectural tour (tasks, the MPMC data buses, command-bar
syntax), see [`CLAUDE.md`](CLAUDE.md). For the plugin API, see the
[Plugins Developer Guide](plugins/README.md).

## Coding conventions

- **Format your code** with `cargo fmt --all` before committing. CI runs
  `cargo fmt --all -- --check` and fails on any diff.
- **Keep the tree warning-clean.** `src/main.rs` has `#![deny(warnings)]`, so
  any compiler warning fails the build.
- **Stay cross-platform.** Guard platform-specific code with `cfg` and don't
  break Linux, Windows, or macOS. If you can't test all three locally, CI will —
  but call out in your PR what you were able to verify.
- **Add tests** for new behavior (see below).
- **Match the surrounding code** in naming, structure, and comment density.

## Testing and the quality checklist

Unit tests live in `#[cfg(test)] mod tests` blocks inside the files they cover.
End-to-end TUI tests are in `tests/tui_e2e.rs` (Unix only): they drive the real
binary in a PTY and assert on the reconstructed screen.

```shell
cargo test --bin scope                 # unit tests
cargo test --bin scope <substring>     # a single test, e.g. cargo test --bin scope test_rhs
cargo test --test tui_e2e              # end-to-end TUI tests (Unix only)
cargo test --test tui_e2e -- --ignored # includes the platform-dependent serial-RX test
```

Before opening a pull request, run the same checks CI does so it passes on the
first try (CI runs these on Linux, Windows, and macOS):

```shell
cargo fmt --all -- --check
cargo test --locked
cargo build --locked --release
```

You can also drive and eyeball the running TUI without hardware using the
`test-tui` helper described in [`CLAUDE.md`](CLAUDE.md) (virtual serial port via
`socat`, keystroke injection via `tmux`).

## Commit and branch conventions

- **Branch off `main`** and name your branch after the work, e.g.
  `feat/45-contributing-guide`, `fix/123-reconnect-windows`, or
  `docs/...`.
- **Write [Conventional Commits](https://www.conventionalcommits.org/).** Use a
  type prefix and an imperative summary:

  ```
  feat: add flow-control command to the serial interface
  fix: keep auto-reconnect alive after a port replug on Windows
  docs: document the config.toml resolution order
  ```

  Common types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`.
- **Reference the issue** in the commit body or the PR (e.g. `(#45)` or
  `Closes #45`).
- **Do not bump the version** in `Cargo.toml`. Releases are automated: when the
  version on `main` changes, CI tags the release, publishes to crates.io, and
  uploads binaries. Version bumps are a maintainer action, and a CI check
  (`version-guard`) will fail your pull request if it changes the version field.

## Opening a pull request

1. Push your branch and open a PR against `main`.
2. Fill in the pull-request template: what changed, why, how you tested it, and
   the issue it closes (`Closes #NN`).
3. Make sure CI is green (build + tests on all three OSes, and `rustfmt`).
4. Be responsive to review feedback — small follow-up commits are fine; the
   maintainer will squash/merge as appropriate.

Keep pull requests focused: one logical change per PR is much easier to review
than a large mixed one.

## Contributing plugins

Extensibility is a core pillar, and plugins are a great first contribution.
Plugins are Lua scripts that hook into `Scope`'s lifecycle and I/O events. See
the [Plugins Developer Guide](plugins/README.md) for the API, and the existing
scripts under `plugins/` for working examples. If you build something broadly
useful, feel free to propose adding it (or a link to it) via an issue or PR.

## Documentation and demo GIFs

Documentation changes are very welcome. If your change affects behavior shown in
the README, note it in your PR.

The animated GIFs in the README are generated **headlessly** — no real hardware
and no manual screen recording. Each demo is a `videos/NNN_name/steps.sh` script
driven by `socat` + `tmux` + `asciinema` + `agg`. To add or update one, see
[`videos/README.md`](videos/README.md).

## License

By contributing to `Scope`, you agree that your contributions will be dual
licensed under the project's [MIT](LICENSE-MIT) and [Apache-2.0](LICENSE-APACHE)
licenses, without any additional terms or conditions.

## Getting help

- Questions, bugs, and ideas: open an
  [issue](https://github.com/matheuswhite/scope-rs/issues).
- If you'd like to support the project, there's a
  [Ko-fi](https://ko-fi.com/matheuswhite) link.

Thanks again for contributing! 🎉
