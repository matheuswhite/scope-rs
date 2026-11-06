//! Shell completion (issues #231, #236).
//!
//! `scope completions <SHELL>` prints a *registration* script: a few lines that
//! make the shell call `scope` back on every Tab (`SCOPE_COMPLETE=<shell> scope
//! -- <words>`), and `main` answers that call through [`clap_complete`]'s
//! `CompleteEnv` before anything else runs. Asking the binary at Tab time
//! instead of shipping a frozen script is what lets values come from the live
//! system:
//!
//! * `scope serial <TAB>` — the ports [`serialport::available_ports`] reports
//!   (not just the USB ones `scope list` shows, so a `/dev/ttyS0` counts), and
//!   file names when none match, so a socat PTY the OS doesn't enumerate can
//!   still be completed by path;
//! * `scope serial <PORT> <TAB>` — the picker's common baud rates;
//! * `scope rtt <TAB>` — every chip name in probe-rs' built-in target registry;
//! * `--speed <TAB>` — the picker's common probe clocks;
//! * `--tag-file`/`--elf <TAB>` — file names, via their `ValueHint`.
//!
//! Everything else (the subcommands, flags and their help) comes from the
//! `Cli` definition itself, exactly as the static scripts did.

use crate::selector::{COMMON_BAUDS, COMMON_SPEEDS};
use clap_complete::aot::Shell;
use clap_complete::engine::ValueCompleter;
use clap_complete::env::Shells;
use clap_complete::{CompletionCandidate, PathCompleter};
use serialport::{SerialPortInfo, SerialPortType};
use std::ffi::OsStr;
use std::io::{self, Write};

/// Environment variable through which the registration script calls `scope`
/// back. Scoped to us rather than clap's default `COMPLETE`, a name generic
/// enough to already be set in someone's environment — which would turn every
/// `scope` run into a completion request.
pub const COMPLETE_VAR: &str = "SCOPE_COMPLETE";

/// The command the shell completes, and the one the registration script runs to
/// get candidates. A bare name rather than an absolute path, so the script keeps
/// working when scope is upgraded or reinstalled elsewhere on `PATH` — the
/// shell can only complete a `scope` it finds there anyway.
pub const BIN: &str = "scope";

/// Writes the registration script for `shell` to `out`.
///
/// Each one has the same install shape as the static script #231 shipped (zsh's
/// starts with `#compdef scope`, so it still goes on `$fpath` as `_scope`), so
/// upgrading is re-running the same `scope completions <SHELL> > <file>`.
pub fn write_registration(shell: Shell, out: &mut dyn Write) -> io::Result<()> {
    let name = shell.to_string();
    let shells = Shells::builtins();
    let completer = shells.completer(&name).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("no dynamic completion for {name}"),
        )
    })?;
    completer.write_registration(COMPLETE_VAR, BIN, BIN, BIN, out)?;
    if shell == Shell::Zsh {
        out.write_all(ZSH_AUTOLOAD_TAIL.as_bytes())?;
    }
    Ok(())
}

/// clap's zsh registration only *defines* the completer and `compdef`s it,
/// which is right for `source <(...)` but not for the `$fpath` install #231
/// documented: there `compinit` autoloads `_scope` on the first Tab and runs
/// the file's body as that call, so the first Tab registers and offers nothing.
/// This is the same tail clap's static zsh script ends with — when the file is
/// running *as* the autoloaded `_scope`, complete right away.
const ZSH_AUTOLOAD_TAIL: &str = r#"
if [ "$funcstack[1]" = "_scope" ]; then
    _clap_dynamic_completer_scope "$@"
fi
"#;

/// `scope serial <TAB>`: the live serial ports starting with `current`, or —
/// when none does and something has been typed — the file names it could be
/// completing, so a path to a port the OS doesn't enumerate (a socat PTY, say)
/// still completes.
pub fn serial_ports(current: &OsStr) -> Vec<CompletionCandidate> {
    let ports = port_candidates(serialport::available_ports().unwrap_or_default(), current);
    if ports.is_empty() && falls_back_to_paths(current) {
        return PathCompleter::any().complete(current);
    }
    ports
}

/// Whether an unmatched port name is completed as a path. Not for an empty
/// word: on a machine with no ports, `scope serial <TAB>` would otherwise offer
/// `.` and whatever lives in the working directory, none of which is a port.
/// Once something is typed (`/dev/pts/`, `./`) it is presumably a path.
fn falls_back_to_paths(current: &OsStr) -> bool {
    !current.is_empty()
}

/// The pure half of [`serial_ports`]: `ports` filtered by the `current` prefix,
/// each described the way the picker and `scope list` describe it.
fn port_candidates(ports: Vec<SerialPortInfo>, current: &OsStr) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy();
    let mut ports: Vec<_> = ports
        .into_iter()
        .filter(|port| port.port_name.starts_with(current.as_ref()))
        .collect();
    ports.sort_by(|a, b| a.port_name.cmp(&b.port_name));
    ports
        .into_iter()
        .map(|port| {
            let help = port_description(&port.port_type).map(Into::into);
            CompletionCandidate::new(port.port_name).help(help)
        })
        .collect()
}

/// The help shown next to a port. Only for the kinds serialport identifies
/// reliably: on macOS every non-USB port — Bluetooth ones included — is
/// reported as `PciPort`, so labelling that would be a lie more often than not.
fn port_description(port_type: &SerialPortType) -> Option<String> {
    match port_type {
        SerialPortType::UsbPort(info) => Some(
            info.product
                .clone()
                .or_else(|| info.manufacturer.clone())
                .unwrap_or_else(|| "USB".to_string()),
        ),
        SerialPortType::BluetoothPort => Some("Bluetooth".to_string()),
        SerialPortType::PciPort | SerialPortType::Unknown => None,
    }
}

/// `scope serial <PORT> <TAB>`: the baud rates the picker offers. Any other
/// rate can still be typed; the engine filters these by prefix.
pub fn baud_rates() -> Vec<CompletionCandidate> {
    COMMON_BAUDS
        .iter()
        .map(|baud| CompletionCandidate::new(baud.to_string()))
        .collect()
}

/// `--speed <TAB>`: the probe clocks (kHz) the RTT picker offers.
pub fn probe_speeds() -> Vec<CompletionCandidate> {
    COMMON_SPEEDS
        .iter()
        .map(|speed| CompletionCandidate::new(speed.to_string()))
        .collect()
}

/// `scope rtt <TAB>`: the probe-rs chip names starting with `current`, each
/// described by its family. Matched case-insensitively, as probe-rs itself
/// resolves `--chip` names.
pub fn rtt_targets(current: &OsStr) -> Vec<CompletionCandidate> {
    let registry = probe_rs::config::Registry::from_builtin_families();
    let chips = registry.families().iter().flat_map(|family| {
        family.variants.iter().flat_map(move |chip| {
            chip.package_variants()
                .map(move |name| (name.as_str(), family.name.as_str()))
        })
    });
    target_candidates(chips, current)
}

/// The pure half of [`rtt_targets`]: `(chip, family)` pairs filtered by the
/// `current` prefix, ignoring case, sorted and deduplicated.
fn target_candidates<'a>(
    chips: impl Iterator<Item = (&'a str, &'a str)>,
    current: &OsStr,
) -> Vec<CompletionCandidate> {
    let current = current.to_string_lossy().to_ascii_lowercase();
    let mut chips: Vec<_> = chips
        .filter(|(name, _)| name.to_ascii_lowercase().starts_with(&current))
        .collect();
    chips.sort_unstable();
    chips.dedup_by(|a, b| a.0 == b.0);
    chips
        .into_iter()
        .map(|(name, family)| CompletionCandidate::new(name).help(Some(family.to_string().into())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::UsbPortInfo;

    fn values(candidates: &[CompletionCandidate]) -> Vec<String> {
        candidates
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect()
    }

    fn help(candidate: &CompletionCandidate) -> String {
        candidate
            .get_help()
            .map(|h| h.to_string())
            .unwrap_or_default()
    }

    fn port(name: &str, port_type: SerialPortType) -> SerialPortInfo {
        SerialPortInfo {
            port_name: name.to_string(),
            port_type,
        }
    }

    fn usb(product: Option<&str>, manufacturer: Option<&str>) -> SerialPortType {
        SerialPortType::UsbPort(UsbPortInfo {
            vid: 0x0403,
            pid: 0x6001,
            serial_number: None,
            manufacturer: manufacturer.map(str::to_string),
            product: product.map(str::to_string),
            interface: None,
        })
    }

    #[test]
    fn ports_are_filtered_by_prefix_and_sorted() {
        let ports = vec![
            port("/dev/ttyUSB1", usb(None, None)),
            port("/dev/ttyS0", SerialPortType::Unknown),
            port("/dev/ttyUSB0", usb(None, None)),
            port("/dev/ttyACM0", usb(None, None)),
        ];
        let got = port_candidates(ports, OsStr::new("/dev/ttyU"));
        assert_eq!(values(&got), ["/dev/ttyUSB0", "/dev/ttyUSB1"]);
    }

    #[test]
    fn non_usb_ports_are_offered_too() {
        // `scope list` hides these, but they are perfectly openable — the
        // reason the completion doesn't reuse `list::usb_ports` (issue #236).
        let ports = vec![
            port("/dev/ttyS0", SerialPortType::Unknown),
            port("/dev/ttyS1", SerialPortType::PciPort),
            port("/dev/rfcomm0", SerialPortType::BluetoothPort),
        ];
        let got = port_candidates(ports, OsStr::new(""));
        assert_eq!(values(&got), ["/dev/rfcomm0", "/dev/ttyS0", "/dev/ttyS1"]);
        assert_eq!(help(&got[0]), "Bluetooth");
        // macOS reports every non-USB port as PCI, so it isn't labelled.
        assert_eq!(help(&got[2]), "");
    }

    #[test]
    fn usb_ports_are_described_like_the_picker_does() {
        let ports = vec![
            port("a", usb(Some("FT232R"), Some("FTDI"))),
            port("b", usb(None, Some("FTDI"))),
            port("c", usb(None, None)),
        ];
        let got = port_candidates(ports, OsStr::new(""));
        let helps: Vec<_> = got.iter().map(help).collect();
        assert_eq!(helps, ["FT232R", "FTDI", "USB"]);
    }

    #[test]
    fn only_a_typed_word_falls_back_to_paths() {
        assert!(!falls_back_to_paths(OsStr::new("")));
        assert!(falls_back_to_paths(OsStr::new("/dev/pts/")));
    }

    #[test]
    fn no_matching_port_yields_nothing() {
        let ports = vec![port("COM3", SerialPortType::Unknown)];
        assert!(port_candidates(ports, OsStr::new("/dev/")).is_empty());
    }

    #[test]
    fn targets_match_case_insensitively_and_dedupe() {
        let chips = [
            ("STM32F303VCTx", "STM32F3"),
            ("STM32F303K8Tx", "STM32F3"),
            ("nRF52840_xxAA", "nRF52"),
            ("STM32F303VCTx", "STM32F3"),
        ];
        let got = target_candidates(chips.into_iter(), OsStr::new("stm32f303"));
        assert_eq!(values(&got), ["STM32F303K8Tx", "STM32F303VCTx"]);
        assert_eq!(help(&got[0]), "STM32F3");
    }

    #[test]
    fn the_builtin_registry_knows_common_chips() {
        // Guards against probe-rs dropping `builtin-targets`, which would leave
        // `scope rtt <TAB>` silently empty.
        let got = values(&rtt_targets(OsStr::new("nRF52840")));
        assert!(got.iter().any(|c| c.starts_with("nRF52840")), "{got:?}");
    }

    #[test]
    fn baud_and_speed_lists_come_from_the_picker() {
        assert_eq!(baud_rates().len(), COMMON_BAUDS.len());
        assert!(values(&baud_rates()).contains(&"115200".to_string()));
        assert!(values(&probe_speeds()).contains(&"4000".to_string()));
    }

    #[test]
    fn every_aot_shell_has_a_registration() {
        // `scope completions` accepts the static generator's shell list; each
        // one must have a dynamic counterpart or the command would error.
        for shell in [
            Shell::Bash,
            Shell::Elvish,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Zsh,
        ] {
            let mut buf = Vec::new();
            write_registration(shell, &mut buf).unwrap_or_else(|e| panic!("{shell}: {e}"));
            let script = String::from_utf8(buf).unwrap();
            assert!(script.contains(COMPLETE_VAR), "{shell}: {script}");
        }
    }
}
