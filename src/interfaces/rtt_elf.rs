//! Reads the address of the RTT control block out of a firmware ELF
//! (issue #248).
//!
//! SEGGER's RTT implementation exports the control block as the symbol
//! `_SEGGER_RTT`, so a build artifact of the firmware that is already running
//! on the target answers "where is the control block?" exactly, with no
//! scanning at all — the same trick `probe-rs run` and `cargo-embed` use. It is
//! opt-in through `scope rtt --elf <PATH>`, since it needs a file only the user
//! can point at.

use object::{Object, ObjectSymbol};
use std::path::Path;

/// The symbol SEGGER's RTT implementation gives the control block.
const CONTROL_BLOCK_SYMBOL: &str = "_SEGGER_RTT";

/// Address of `_SEGGER_RTT` in `path`.
///
/// The error is a sentence meant to be logged verbatim: it is reported to the
/// user and attaching falls back to scanning, because a stale or unreadable ELF
/// should cost speed, not the connection.
pub fn control_block_address(path: &Path) -> Result<u64, String> {
    let data =
        std::fs::read(path).map_err(|err| format!("cannot read {}: {}", path.display(), err))?;

    address_in_bytes(&data).map_err(|err| format!("{} {}", path.display(), err))
}

/// The parsing half of [`control_block_address`], split out so the tests can
/// drive it with ELFs they build in memory.
fn address_in_bytes(data: &[u8]) -> Result<u64, String> {
    let file = object::File::parse(data).map_err(|err| format!("is not a readable ELF: {err}"))?;

    file.symbols()
        .find(|symbol| symbol.name() == Ok(CONTROL_BLOCK_SYMBOL))
        .map(|symbol| symbol.address())
        .ok_or_else(|| {
            format!(
                "has no {} symbol (is RTT enabled in this build?)",
                CONTROL_BLOCK_SYMBOL
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::write::{Object, StandardSection, Symbol, SymbolSection};
    use object::{Architecture, BinaryFormat, Endianness, SymbolFlags, SymbolKind, SymbolScope};

    /// A minimal ARM ELF exporting `symbols` as `(name, address)` pairs in a
    /// `.bss`-like section, which is where a linker puts the real control block.
    fn elf_with(symbols: &[(&str, u64)]) -> Vec<u8> {
        let mut obj = Object::new(BinaryFormat::Elf, Architecture::Arm, Endianness::Little);
        let section = obj.section_id(StandardSection::UninitializedData);

        for (name, address) in symbols {
            obj.add_symbol(Symbol {
                name: name.as_bytes().to_vec(),
                value: *address,
                size: 0x30,
                kind: SymbolKind::Data,
                scope: SymbolScope::Dynamic,
                weak: false,
                section: SymbolSection::Section(section),
                flags: SymbolFlags::None,
            });
        }

        obj.write().expect("write the test ELF")
    }

    #[test]
    fn the_control_block_symbol_is_found() {
        // The address from the issue: OCRAM at 0x20200000 + 0x410.
        let elf = elf_with(&[("_SEGGER_RTT", 0x2020_0410)]);

        assert_eq!(address_in_bytes(&elf), Ok(0x2020_0410));
    }

    #[test]
    fn other_symbols_are_ignored() {
        let elf = elf_with(&[
            ("__rtt_buff_data_start", 0x2020_0000),
            ("_SEGGER_RTT", 0x2020_0410),
            ("main", 0x6000_1234),
        ]);

        assert_eq!(address_in_bytes(&elf), Ok(0x2020_0410));
    }

    #[test]
    fn an_elf_without_the_symbol_says_so() {
        let elf = elf_with(&[("main", 0x6000_1234)]);

        let err = address_in_bytes(&elf).expect_err("no control block symbol");
        assert!(err.contains("_SEGGER_RTT"), "{err}");
    }

    #[test]
    fn a_file_that_is_not_an_elf_says_so() {
        let err = address_in_bytes(b"not an elf at all").expect_err("not an ELF");
        assert!(err.contains("ELF"), "{err}");
    }

    #[test]
    fn a_missing_file_reports_its_path() {
        let err =
            control_block_address(Path::new("/nonexistent/zephyr.elf")).expect_err("missing file");
        assert!(err.contains("/nonexistent/zephyr.elf"), "{err}");
    }
}
