# glorious-linux

Configure Glorious mice and check Glorious firmware on Linux, without Glorious CORE.
A Rust GUI and CLI in a single `glorious` binary.

> Unofficial. Not affiliated with or endorsed by Glorious. Use at your own risk.

## Supported devices

| Device | USB ID | Status |
|---|---|---|
| Model D3 Wireless (via V3 receiver or cable) | `3794:a300` / `3794:a322` | Tested on hardware |
| Model O3 Wireless | `3794:a300` / `3794:a310` | Same protocol, untested |
| GMMK 3 HE 65 / 75 / 100% ANSI | `342d:e3dd` / `e3de` / `e3df` | Firmware status (tested on 75%) |

Keyboard **configuration** (lighting, actuation, rapid trigger, keymap) is handled by
[inglorious-core-linux](https://github.com/tg5ee/inglorious-core-linux); the Keyboard tab
launches it if `inglorious-core` is on your `PATH`.

## Features

- **Mouse status**: model, colour, firmware, receiver firmware, battery (level, voltage, charging).
- **Performance**: six DPI stages with colours (50-26000), active stage, wireless and wired
  polling rate (125-8000 Hz), lift-off distance (0.7 / 1 / 2 mm), motion sync, mechanical and
  optical debounce, sleep timer.
- **Lighting**: all 12 effects, separate wired and wireless brightness, speed, single colour.
- **Buttons**: remap any of the six buttons to a mouse function or a keyboard key.
- **Firmware**: compare the mouse and receiver with Glorious' latest release and update them
  natively on Linux (with a dry run that shows every step first).
- **Keyboard**: GMMK 3 HE firmware versions against the latest release, active profile.

The mouse cannot report its DPI, polling, lift-off, debounce or button settings back, so - like
Glorious CORE - the last applied values are stored in `~/.config/glorious-linux/mouse.json`.

## Install

Build dependencies: Rust (stable), `pkgconf`, `systemd-libs` (libudev), and the usual
Wayland/X11 libraries. On Arch / CachyOS these are normally already present.

```sh
git clone https://github.com/Virel0/glorious-linux
cd glorious-linux
cargo install --path .          # installs `glorious` into ~/.cargo/bin
```

Give your user access to the devices (no root needed afterwards):

```sh
sudo cp packaging/70-glorious-linux.rules /etc/udev/rules.d/
sudo udevadm control --reload && sudo udevadm trigger
```

Then unplug and replug the receiver, mouse and keyboard.

Optional desktop entry:

```sh
cp packaging/glorious-linux.desktop ~/.local/share/applications/
```

## Usage

`glorious` with no arguments opens the GUI. The CLI covers the same ground:

```sh
glorious mouse info
glorious mouse dpi 400,800,1600,3200 --active 3 --colors ff0000,00ff00,0000ff,ffffff
glorious mouse polling 4000            # wireless; add --wired for the cable rate
glorious mouse liftoff 1               # 0.7 | 1 | 2 (mm)
glorious mouse motion-sync on
glorious mouse debounce --mechanical 2 --optical 0
glorious mouse sleep 10                # minutes, 0 = never
glorious mouse lighting solid --color ff8800 --brightness 60
glorious mouse bind dpi middle         # or key:a, key:f12, key:0x04 ...
glorious mouse firmware check
glorious mouse firmware flash --dry-run
glorious keyboard info
```

Add `-v` to print every HID report sent and received.

If commands report "no reply from the mouse", it is asleep: move it and try again.

## Firmware updates

Updates use Glorious' own firmware packages, downloaded from the same public server Glorious
CORE uses. Flashing needs two external tools:

```sh
paru -S nrfutil && nrfutil install nrf5sdk-tools   # Nordic nRF52 serial DFU
sudo pacman -S dfu-util                            # GD32 USB DFU (receiver)
```

The mouse update needs the USB cable plugged into the mouse. The sequence is:

1. **Mouse**: a HID command reboots it into its nRF52 bootloader, then
   `nrfutil dfu serial` flashes the package.
2. **Receiver, radio chip**: same, with the receiver's nRF52 bootloader.
3. **Receiver, USB chip**: a HID command reboots the GD32 into its ROM DFU bootloader
   (`28e9:0189`), then `dfu-util` flashes it.

Both bootloaders survive an interrupted update, so a failed flash can simply be retried.
Always run `glorious mouse firmware flash --dry-run` first to see exactly what will happen.
The real flashing path matches Glorious CORE's updater byte for byte but has not yet been
exercised on hardware, because no update newer than mouse 1.3.3 / receiver 1.3.0 + 1.1.0 has
been released.

GMMK 3 HE keyboard updates use a separate Windows-only updater and are not supported yet; the
Keyboard tab tells you when one is available.

## How it works

The protocol was re-implemented for interoperability from Glorious CORE 2.1.23 (an Electron app
whose device logic is plain JavaScript) and checked against the real hardware:

- Mouse: 64-byte HID feature reports with report ID 3,
  `[0x03, target, command, length, payload...]`, where target `0xBB` is the mouse (relayed by the
  receiver over 2.4 GHz) and `0xAA` the receiver. Replies echo the 4-byte header.
- Keyboard: 64-byte feature reports without a report ID, byte 7 a checksum
  (`255 - sum(bytes 0..6)`).
- Firmware updaters: CORE ships them as packaged Node.js executables; their exact HID commands
  and tool invocations were captured by running them against a mocked HID layer.

No Glorious code, binaries or firmware are included in this repository.

## License

MIT - see [LICENSE](LICENSE).
