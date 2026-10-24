# Security

## Reporting a vulnerability

Please report security problems privately through
[GitHub's private vulnerability reporting](https://github.com/Virel0/glorious-linux/security/advisories/new)
rather than in a public issue. Include the version (`glorious --version`), what you found, and
how to reproduce it. You'll get an answer as soon as possible; fixes are released as a new
version with a note in the [changelog](CHANGELOG.md).

## What the app can touch

glorious-linux runs as your user and never needs root. The installer asks for `sudo` once, for
two things only: the udev rule below and (optionally) installing `dfu-util` from your
distribution.

The udev rule (`packaging/70-glorious-linux.rules`) gives the logged-in user access to exactly
the interfaces the app uses:

| Device | Access | Why |
|---|---|---|
| V3 wireless receiver | interface 0 (vendor settings) | mouse settings and firmware commands |
| Model D3 / O3 on USB | interface 0 (vendor settings) | settings and the firmware-update command |
| GMMK 3 HE 75% | interface 2 (vendor settings) | keyboard settings |
| Mouse / receiver bootloaders | serial port, GD32 DFU | firmware updates |

HID interfaces that carry typing - the keyboard's own key reports, and the receiver's and the
wired mouse's interfaces for keys bound to mouse buttons - are deliberately **not** opened to your user, so other programs
can't read your keystrokes through them. The keyboard's live key-travel meter needs one of those
interfaces and is therefore off; the rule file has a commented-out line to enable it if you
accept that trade-off.

## Firmware updates

- Glorious' release list and firmware are downloaded over HTTPS from the same server Glorious
  CORE uses.
- Every firmware bundle is checked against a SHA-256 pinned in the app before anything is
  flashed. A known bundle with a different hash is refused; a bundle newer than the app knows
  needs explicit confirmation (`--allow-unverified` in the CLI).
- Only the three files the update needs are read from the bundle and written, under fixed names,
  into a private (`0700`, randomly named) temporary directory that is removed afterwards.
- The mouse and the receiver's radio verify the signature of their firmware themselves (Nordic
  secure DFU) and keep the old firmware until a complete, verified image has arrived.
- Updates refuse to start unless the right device is connected (the mouse over its USB cable).

## Releases and dependencies

- Release binaries are built by GitHub Actions from the tagged commit, with a `.sha256` file and a
  signed [build provenance attestation](https://docs.github.com/actions/security-for-github-actions/using-artifact-attestations).
  `install.sh` checks the checksum; to verify provenance yourself:

  ```sh
  gh attestation verify glorious-linux-x86_64-linux.tar.gz --repo Virel0/glorious-linux
  ```

- `Cargo.lock` is committed and every build uses `--locked`. CI runs `cargo audit` on every push
  and weekly, and Dependabot proposes dependency and GitHub Actions updates.
- GitHub Actions are pinned to commit hashes and run with read-only permissions unless a job
  needs more (only the release job can write, to publish the release).
- The keyboard protocol library (inglorious-core-linux) is pinned to a reviewed commit.
