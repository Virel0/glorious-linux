# Changelog

## v0.5.0

**Security**
- The udev rule now only opens the interfaces the app needs. The keyboard's key-report
  interfaces and the receiver's keystroke interface are no longer accessible to your user
  programs, so they can't be used to read your typing. The keyboard's live travel meter is off
  as a result (opt-in line in the rule file).
- Firmware bundles are checked against pinned SHA-256 hashes before flashing; unknown (newer)
  bundles need explicit confirmation. Downloads are size-capped.
- Only the three needed files are read from a firmware bundle, into a private random temporary
  directory - no archive extraction. Upgraded `zip` to 9.x (RUSTSEC-2026-0367/0368/0369).
- Releases ship a SHA-256 checksum (checked by `install.sh`) and a signed build-provenance
  attestation. CI runs `cargo audit` weekly; Actions are pinned to commits with read-only
  permissions; Dependabot is enabled. Added SECURITY.md.

**Other**
- Refreshed README, added CONTRIBUTING.md, issue templates and this changelog. Code is
  `rustfmt`-formatted and checked in CI.

## v0.4.1
- Firmware updates refuse to start unless the mouse is on its cable (or the receiver is
  plugged in), and show step and progress bars.
- App id, desktop entry and icon renamed to `io.github.virel0.GloriousLinux`.
- Receiver firmware is read even while the mouse is on its cable.

## v0.4.0
- New app logo in the app menu, search and taskbar.
- Licensed under GPL-3.0-or-later (earlier releases were MIT).
- Keyboard pages show a slim "Connecting…" banner instead of a full-page placeholder.

## v0.3.0
- Keyboard pages built in: profiles, lighting with per-key painting, actuation and rapid
  trigger, keymap.
- Mouse settings are read back from the mouse; the active DPI stage follows the DPI button live.
- Built-in Nordic DFU replaces `nrfutil`; the installer installs `dfu-util`.

## v0.2.0
- Redesigned GUI and the one-line installer.

## v0.1.0
- First release: Model D3 settings and firmware check (GUI + CLI).
