# scope icons

Windows `.ico` files used by the MSI installer's Start-Menu shortcuts (and, for
`scope.ico`, embedded into `scope.exe`). They are **generated**, not hand-drawn —
regenerate them with:

```bash
cargo run --manifest-path installer/gen-icons/Cargo.toml
```

The generator (`installer/gen-icons/`) crops the oscilloscope glyph out of
`imgs/scope-logo.png` and adds a small per-command badge:

| File                         | Launch command            | Badge                 |
| ---------------------------- | ------------------------- | --------------------- |
| `scope.ico`                  | — (the executable's icon) | none                  |
| `scope-serial.ico`           | `scope serial`            | green · plug          |
| `scope-serial-headless.ico`  | `scope --headless serial` | blue · terminal       |
| `scope-rtt.ico`              | `scope rtt`               | purple · microchip    |
| `scope-rtt-headless.ico`     | `scope --headless rtt`    | orange · terminal     |

## Attribution

The badge glyphs are from [Font Awesome Free 6](https://fontawesome.com)
(`plug`, `terminal`, `microchip`), licensed **CC BY 4.0**. The glyph SVGs live in
`installer/gen-icons/glyphs/`.
