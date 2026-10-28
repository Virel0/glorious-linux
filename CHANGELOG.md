# Changelog

All notable changes to this project are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Releases can be rebuilt for an existing tag from the Actions tab, and are GPG-signed (`.asc`)
  once a signing key is added as a repo secret.
- README privacy section: what the app sends over the network and what it stores.
- Feature-request issue form.

### Changed
- Keymap: mouse-button codes read "Mouse Left", "Mouse Right"... in title case like the other keys.
- Internal cleanup with no other visible changes: the GUI is split into one file per page, long
  functions are broken up, the firmware "ready to flash?" rules are shared by the CLI and GUI, and
  the mouse protocol's report building and parsing now have tests.
- Release builds run with a read-only token; a separate job that builds nothing attests and
  uploads them.
- CI also rejects banned dependencies (aws-lc-rs, OpenSSL), reports file and function sizes, and
  clippy enforces the 100-line function limit and no `unwrap` outside tests.

## [0.5.1] - 2026-10-24

### Changed
- Updated the GUI toolkit (eframe/egui 0.37) and the HTTP client (ureq 3).
- The keyboard protocol code is now part of the project (from inglorious-core-linux, MIT), and
  the Phosphor icon font is bundled directly - no git dependency, fewer crates, and icon updates
  can't block GUI upgrades.

### Fixed
- The keyboard connects reliably right after the app was closed mid-read (it now retries a stale
  reply instead of showing "not found" for 20 seconds), and retries every 2 s until connected.

## [0.5.0] - 2026-10-24

### Security
- The udev rule now only opens the interfaces the app needs (verified with `udevadm test` and on
  real devices). The keyboard's key-report interfaces and the receiver's and wired mouse's
  keystroke interfaces are no longer accessible to your user programs, so they can't be used to
  read your typing. The keyboard's live travel meter is off as a result (opt-in line in the rule
  file).
- Firmware bundles are checked against pinned SHA-256 hashes before flashing; unknown (newer)
  bundles need explicit confirmation. Downloads are size-capped.
- Only the three needed files are read from a firmware bundle, into a private random temporary
  directory - no archive extraction. Upgraded `zip` to 9.x (RUSTSEC-2026-0367/0368/0369).
- Releases ship a SHA-256 checksum (checked by `install.sh`) and a signed build-provenance
  attestation. CI runs `cargo audit` weekly; Actions are pinned to commits with read-only
  permissions; Dependabot is enabled. Added SECURITY.md.

### Added
- CONTRIBUTING.md, issue templates and this changelog.

### Changed
- Refreshed README. Code is `rustfmt`-formatted and checked in CI.

## 0.4.1 - 2026-10-24

Never tagged; these changes first shipped in 0.5.0.

### Added
- Firmware updates show step and progress bars.

### Changed
- Firmware updates refuse to start unless the mouse is on its cable (or the receiver is
  plugged in).
- App id, desktop entry and icon renamed to `io.github.virel0.GloriousLinux`.

### Fixed
- Receiver firmware is read even while the mouse is on its cable.

## [0.4.0] - 2026-10-24

### Added
- New app logo in the app menu, search and taskbar.

### Changed
- Licensed under GPL-3.0-or-later (earlier releases were MIT).
- Keyboard pages show a slim "Connecting…" banner instead of a full-page placeholder.

## [0.3.0] - 2026-10-24

### Added
- Keyboard pages built in: profiles, lighting with per-key painting, actuation and rapid
  trigger, keymap.
- Mouse settings are read back from the mouse; the active DPI stage follows the DPI button live.

### Changed
- Built-in Nordic DFU replaces `nrfutil`; the installer installs `dfu-util`.

## [0.2.0] - 2026-10-24

### Added
- The one-line installer.

### Changed
- Redesigned GUI.

## [0.1.0] - 2026-10-24

### Added
- First release: Model D3 settings and firmware check (GUI + CLI).

[Unreleased]: https://github.com/Virel0/glorious-linux/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/Virel0/glorious-linux/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/Virel0/glorious-linux/compare/v0.4.0...v0.5.0
[0.4.0]: https://github.com/Virel0/glorious-linux/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/Virel0/glorious-linux/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/Virel0/glorious-linux/releases/tag/v0.2.0
[0.1.0]: https://github.com/Virel0/glorious-linux/commit/2a386b8
