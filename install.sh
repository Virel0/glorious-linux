#!/usr/bin/env bash
# glorious-linux installer.
#
#   ./install.sh                       from a clone or an unpacked release
#   curl -fsSL https://raw.githubusercontent.com/Virel0/glorious-linux/main/install.sh | bash
#
# Installs `glorious` into ~/.local/bin, an app-menu entry and icon, and (with sudo, after
# asking) the udev rule that lets your user talk to the devices. Builds from source when Rust
# is available, otherwise downloads the latest release binary.
#
# Options: --source | --binary   force how `glorious` is obtained
#          --keyboard            also install the keyboard configurator (inglorious-core-linux)
#          --no-keyboard         don't offer it
#          --no-udev             skip the udev rule
#          -y, --yes             answer yes to every question
set -euo pipefail

REPO="Virel0/glorious-linux"
# inglorious-core-linux commit whose protocol was checked against Glorious CORE.
KEYBOARD_REPO="https://github.com/tg5ee/inglorious-core-linux"
KEYBOARD_REV="2bf5c6faecd7d7337e7f6e0e4e25dfb64524b9f3"
PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
RULE_DEST="/etc/udev/rules.d/70-glorious-linux.rules"

mode="auto" keyboard="ask" udev=1 yes=0
for arg in "$@"; do
    case "$arg" in
        --source) mode="source" ;;
        --binary) mode="binary" ;;
        --keyboard) keyboard="yes" ;;
        --no-keyboard) keyboard="no" ;;
        --no-udev) udev=0 ;;
        -y|--yes) yes=1 ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "Unknown option: $arg" >&2; exit 2 ;;
    esac
done

bold=$'\e[1m' amber=$'\e[38;5;214m' green=$'\e[32m' red=$'\e[31m' dim=$'\e[2m' reset=$'\e[0m'
step() { printf '\n%s==>%s %s%s%s\n' "$amber" "$reset" "$bold" "$*" "$reset"; }
info() { printf '    %s\n' "$*"; }
ok()   { printf '    %s✓%s %s\n' "$green" "$reset" "$*"; }
die()  { printf '\n%serror:%s %s\n' "$red" "$reset" "$*" >&2; exit 1; }
ask() {  # ask "question" -> 0 for yes
    [ "$yes" = 1 ] && return 0
    local reply
    printf '    %s [Y/n] ' "$1"
    read -r reply < /dev/tty || reply=""
    [[ -z "$reply" || "$reply" =~ ^[Yy] ]]
}

printf '%s%sGlorious for Linux%s installer\n' "$bold" "$amber" "$reset"

[ "$(uname -s)" = Linux ] || die "this only works on Linux"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# Where are we running from?
script_dir=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi
src=""
[ -n "$script_dir" ] && grep -qs '^name = "glorious-linux"' "$script_dir/Cargo.toml" && src="$script_dir"
prebuilt=""
[ -n "$script_dir" ] && [ -x "$script_dir/glorious" ] && prebuilt="$script_dir/glorious"

have_cargo() {
    command -v cargo >/dev/null || return 1
    # rustup without a default toolchain: set one up.
    cargo --version >/dev/null 2>&1 || { command -v rustup >/dev/null && rustup default stable >/dev/null 2>&1; }
    cargo --version >/dev/null 2>&1
}

if [ "$mode" = auto ]; then
    if [ -n "$prebuilt" ]; then mode="prebuilt"
    elif have_cargo; then mode="source"
    else mode="binary"
    fi
fi

# ------------------------------------------------------------------ glorious binary
step "Installing glorious"
mkdir -p "$BIN"
case "$mode" in
    prebuilt)
        install -m755 "$prebuilt" "$BIN/glorious"
        assets="$script_dir/packaging"
        ;;
    source)
        have_cargo || die "Rust is not installed. Install it (e.g. 'sudo pacman -S rustup && rustup default stable') or run with --binary."
        if [ -z "$src" ]; then
            command -v git >/dev/null || die "git is needed to fetch the source"
            info "Fetching source..."
            git clone --quiet --depth 1 "https://github.com/$REPO" "$work/src"
            src="$work/src"
        fi
        info "Building (the first build takes a minute or two)..."
        cargo build --release --locked --manifest-path "$src/Cargo.toml" --quiet
        install -m755 "$src/target/release/glorious" "$BIN/glorious"
        assets="$src/packaging"
        ;;
    binary)
        command -v curl >/dev/null || die "curl is needed to download the release"
        info "Downloading the latest release..."
        curl -fsSL "https://github.com/$REPO/releases/latest/download/glorious-linux-x86_64-linux.tar.gz" | tar -xz -C "$work" \
            || die "download failed - is there a release yet? Otherwise install Rust and use --source."
        install -m755 "$work/glorious-linux/glorious" "$BIN/glorious"
        assets="$work/glorious-linux/packaging"
        ;;
esac
ok "glorious -> $BIN/glorious"

# An older copy from `cargo install` would shadow this one on PATH.
if [ -x "$HOME/.cargo/bin/glorious" ] && [ "$BIN" != "$HOME/.cargo/bin" ]; then
    rm -f "$HOME/.cargo/bin/glorious"
    ok "removed an older copy from ~/.cargo/bin"
fi

# ------------------------------------------------------------------ desktop entry + icon
step "Adding Glorious to your app menu"
install -Dm644 "$assets/glorious-linux.svg" "$DATA/icons/hicolor/scalable/apps/glorious-linux.svg"
sed "s|^Exec=.*|Exec=$BIN/glorious|" "$assets/glorious-linux.desktop" > "$work/glorious-linux.desktop"
install -Dm644 "$work/glorious-linux.desktop" "$DATA/applications/glorious-linux.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATA/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$DATA/icons/hicolor" 2>/dev/null || true
ok "menu entry and icon installed"

# ------------------------------------------------------------------ udev rule
if [ "$udev" = 1 ]; then
    step "Device permissions"
    if cmp -s "$assets/70-glorious-linux.rules" "$RULE_DEST"; then
        ok "udev rule already installed"
    else
        info "A udev rule lets your user talk to Glorious devices without root."
        info "${dim}This needs sudo once: $RULE_DEST${reset}"
        if ask "Install the udev rule now?"; then
            sudo install -Dm644 "$assets/70-glorious-linux.rules" "$RULE_DEST"
            sudo udevadm control --reload
            sudo udevadm trigger --subsystem-match=hidraw --subsystem-match=tty --subsystem-match=usb
            ok "udev rule installed - unplug and replug the receiver, mouse and keyboard"
        else
            info "Skipped. Later: sudo install -Dm644 $assets/70-glorious-linux.rules $RULE_DEST"
        fi
    fi
fi

# ------------------------------------------------------------------ keyboard configurator
if [ "$keyboard" = ask ]; then
    if command -v inglorious-core >/dev/null || [ -x "$BIN/inglorious-core" ]; then
        keyboard="no"
    else
        step "Keyboard configurator (optional)"
        info "GMMK 3 HE lighting, actuation and keymap are set in inglorious-core-linux,"
        info "a separate open-source project. It's built from source (needs Rust)."
        ask "Install it too?" && keyboard="yes" || keyboard="no"
    fi
fi
if [ "$keyboard" = yes ]; then
    step "Installing the keyboard configurator"
    have_cargo || die "the keyboard configurator needs Rust: 'sudo pacman -S rustup && rustup default stable'"
    info "Building inglorious-core-linux at ${KEYBOARD_REV:0:7}..."
    cargo install --quiet --locked --git "$KEYBOARD_REPO" --rev "$KEYBOARD_REV" --root "$PREFIX" inglorious-core
    ok "inglorious-core -> $BIN/inglorious-core"
fi

# ------------------------------------------------------------------ done
step "Done"
case ":$PATH:" in
    *":$BIN:"*) info "Open ${bold}Glorious${reset} from your app menu, or run ${bold}glorious${reset} in a terminal." ;;
    *)
        info "Open ${bold}Glorious${reset} from your app menu."
        info "To run ${bold}glorious${reset} in a terminal, add $BIN to your PATH:"
        info "  fish: fish_add_path $BIN     bash/zsh: echo 'export PATH=\"$BIN:\$PATH\"' >> ~/.bashrc"
        ;;
esac
info "Firmware updates also need: ${dim}nrfutil (AUR) + 'nrfutil install nrf5sdk-tools', and dfu-util${reset}"
