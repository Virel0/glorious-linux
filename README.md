# glorious-linux

Glorious mouse and keyboard software for Linux - settings, lighting, keymaps and firmware
updates without Glorious CORE. One Rust app (GUI + CLI), one installer.

> Unofficial. Not affiliated with or endorsed by Glorious. Use at your own risk.

![Overview](docs/overview.png)

<details>
<summary>More screenshots</summary>

![Performance](docs/performance.png)
![Key lighting](docs/key-lighting.png)
![Actuation](docs/actuation.png)
![Keymap](docs/keymap.png)

</details>

## Supported devices

| Device | USB ID | Status |
|---|---|---|
| Model D3 Wireless (V3 receiver or cable) | `3794:a300` / `3794:a322` | Everything tested on hardware except flashing |
| Model O3 Wireless | `3794:a300` / `3794:a310` | Same protocol, untested |
| GMMK 3 HE 75% ANSI | `342d:e3de` | Reads tested; lighting / actuation / rapid trigger tested upstream; keymap is new |

## Features

**Mouse**
- Reads the mouse's real settings - nothing to set up first.
- Battery, firmware and connection at a glance; reconnects on its own when a sleeping mouse wakes.
- Six DPI stages with colours (50-26000). The active stage follows the DPI button live.
- Wireless and cable polling rate (125-8000 Hz), lift-off distance, motion sync, debounce, sleep timer.
- All 12 lighting effects with an animated preview, separate cable / wireless brightness, colours.
- Remap any of the six buttons to a mouse function or a keyboard key.

**Keyboard (GMMK 3 HE)**
- Profiles 1-3, polling rate, debounce, idle timeout.
- 19 lighting effects, side lights, and per-key colours painted on a drawing of the board.
- Hall-effect press point, rapid trigger with press / release sensitivity, bottom dead zone,
  per-key overrides, and a live key-travel meter.
- Keymap for the Base, Fn and Fn2 layers.

**Firmware**
- Compares the mouse, receiver and keyboard with Glorious' latest release.
- Updates the mouse and receiver natively, with a dry run that shows every step first.

## Install

One line (builds from source if you have Rust, otherwise downloads the release binary):

```sh
curl -fsSL https://raw.githubusercontent.com/Virel0/glorious-linux/main/install.sh | bash
```

Or from a clone:

```sh
git clone https://github.com/Virel0/glorious-linux && cd glorious-linux && ./install.sh
```

The installer puts `glorious` in `~/.local/bin` and adds **Glorious** to your app menu, then
asks once for `sudo` to install the udev rule (so your user can talk to the devices) and
`dfu-util` (used for the receiver's USB chip; everything else is built in). Unplug and replug
your devices afterwards. `./uninstall.sh` removes it again.

**Arch / CachyOS / Manjaro** - build a proper package instead:

```sh
git clone https://github.com/Virel0/glorious-linux && cd glorious-linux/packaging/arch && makepkg -si
```

## Usage

Open **Glorious** from your app menu, or run `glorious`. There is also a CLI:

```sh
glorious mouse info                    # everything the mouse reports, including all settings
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

Add `-v` to print every HID report sent and received. If a command reports "no reply from the
mouse", it is asleep: move it and try again.

The keyboard can't report its lighting, actuation or keymap, so - like Glorious CORE - the last
applied values are stored, in `~/.config/glorious-core-rs/state.json`. That file is shared with
[inglorious-core-linux](https://github.com/tg5ee/inglorious-core-linux), so settings made there
show up here too.

## Firmware updates

Updates use Glorious' own firmware packages, downloaded from the same public server Glorious
CORE uses. The mouse update needs the USB cable plugged into the mouse. The sequence is:

1. **Mouse**: a HID command reboots it into its nRF52 bootloader, and the built-in Nordic DFU
   client sends the signed package (init packet, then the image in CRC-checked objects).
2. **Receiver, radio chip**: the same, with the receiver's nRF52 bootloader.
3. **Receiver, USB chip**: a HID command reboots the GD32 into its ROM DFU bootloader
   (`28e9:0189`), and `dfu-util` flashes it.

The nRF52 bootloaders keep the old firmware until a complete, verified image has arrived, so an
interrupted update can simply be retried. Run the dry run first to see exactly what will happen.
The HID commands match Glorious CORE's updater byte for byte, and the DFU client is covered by
tests against a simulated bootloader, but real flashing has not been exercised on hardware yet
because no firmware newer than mouse 1.3.3 / receiver 1.3.0 + 1.1.0 has been released.

GMMK 3 HE keyboard updates use a separate Windows-only updater and are not supported yet; the
Firmware page tells you when one is available.

## How it works

The protocols were re-implemented for interoperability from Glorious CORE 2.1.23 (an Electron
app whose device logic is plain JavaScript) and checked against real hardware:

- **Mouse**: 64-byte HID feature reports with report ID 3, `[0x03, target, command, length,
  payload...]`; target `0xBB` is the mouse (relayed by the receiver over 2.4 GHz), `0xAA` the
  receiver. Every setter has a getter at `command | 0x80`, which is how the app reads settings back.
- **Keyboard**: uses the protocol, state and device code of
  [inglorious-core-linux](https://github.com/tg5ee/inglorious-core-linux) (MIT, by thomas
  griffin), pinned to a commit whose reports were checked against Glorious CORE.
- **Firmware updaters**: CORE ships them as packaged Node.js executables; their exact HID
  commands and tool invocations were captured by running them against a mocked HID layer.

No Glorious code, binaries or firmware are included in this repository.

## Credits

- [inglorious-core-linux](https://github.com/tg5ee/inglorious-core-linux) (MIT) - keyboard
  protocol library and the board layout the keyboard pages are drawn from.
- Fonts: Space Grotesk, Work Sans, JetBrains Mono (SIL Open Font License, see `assets/fonts`).
- Icons: [Phosphor](https://phosphoricons.com) via egui-phosphor.

## License

MIT - see [LICENSE](LICENSE).
