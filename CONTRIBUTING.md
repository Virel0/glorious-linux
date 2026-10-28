# Contributing

Thanks for helping! Bug reports, device reports and pull requests are all welcome.

## Building

```sh
cargo build --release          # binary: target/release/glorious
cargo test                     # unit tests (add `-- --include-ignored` for the network test)
cargo fmt && cargo clippy --all-targets -- -D warnings
scripts/size-report.sh         # largest files; warns on files > 800 and functions > 100 lines
```

CI runs the same checks plus `cargo audit` and a banned-dependency check (no aws-lc-rs or
OpenSSL; TLS stays on rustls + ring); please keep them green. Keep files focused (ideally under
300 lines) and functions short - clippy rejects functions over 100 lines and `unwrap` outside
tests (use `?`, or `expect` with the reason it can't fail).

## Layout

| File | What it does |
|---|---|
| `src/v3.rs` | Model O3/D3 + V3 receiver protocol (reads and writes) |
| `src/settings.rs` | Mouse settings model: read from / apply to the mouse |
| `src/firmware.rs` | Firmware check, download verification and the update sequence |
| `src/nordic_dfu.rs` | Nordic secure DFU over serial (SLIP), with a simulated bootloader in its tests |
| `src/magneton/` | GMMK 3 HE protocol, state and key tables (vendored from inglorious-core-linux, MIT) |
| `src/kbdevice.rs`, `src/keybed.rs` | GMMK 3 HE I/O worker and board drawing |
| `src/kb_gui/` | Keyboard pages: one file per page, shared state in `mod.rs` |
| `src/gui/` | The egui app: state and layout in `mod.rs`, background jobs in `worker.rs`, one file per tab |
| `src/theme.rs` | Colours, fonts and shared widgets |
| `packaging/` | udev rule, desktop entry, icons, Arch PKGBUILD |

## Adding a device

1. Find its USB ID with `lsusb`, and its HID interfaces with
   `udevadm info -a /dev/hidrawN | grep -E 'idProduct|bInterfaceNumber'`.
2. Glorious CORE describes every device in `out/main/index.js` inside its Electron `app.asar`
   (search for the product name). The connection methods give the vendor/product IDs and usage
   pages; the device's `*DeviceHandler` class shows the commands.
3. `glorious mouse raw <mouse|receiver> <cmd> [payload…]` (hidden) sends one command and prints
   the reply, and `-v` on any command shows the raw traffic - handy for checking a protocol
   against the real device.
4. Add the device to the tables, to the udev rule (only the interfaces you need - never the ones
   carrying keystrokes), and to the README's device table.

Please include the output of `glorious mouse info -v` (or `keyboard info`) in device reports.

## Security

Please report vulnerabilities privately - see [SECURITY.md](SECURITY.md).

## License

By contributing you agree that your contributions are licensed under the GPL-3.0-or-later.
