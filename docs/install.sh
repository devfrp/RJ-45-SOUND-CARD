#!/bin/sh
# RJ45 Sound Card installer for Linux and macOS.
#
#   curl -fsSL https://devfrp.github.io/RJ-45-SOUND-CARD/install.sh | sh
#
# Asks whether this PC is the server (sound cards) or the client (DAW),
# installs the latest release and starts it now and at every startup.
# Non-interactive (e.g. over SSH):  ... | sh -s -- server   (or: client, uninstall)
#
# RJSC_DOWNLOAD_URL overrides the archive to install (mirrors, testing).

set -eu

REPO="devfrp/RJ-45-SOUND-CARD"
RELEASES="https://github.com/$REPO/releases/latest/download"

say() { printf '%s\n' "$*"; }
step() { printf '\n==> %s\n' "$*"; }
die() { printf 'Erreur / Error: %s\n' "$*" >&2; exit 1; }

# Root only where needed (packages, audio group, driver).
as_root() {
    if [ "$(id -u)" -eq 0 ]; then
        "$@"
    elif command -v sudo >/dev/null 2>&1; then
        sudo "$@"
    else
        die "sudo is required for: $*"
    fi
}

download() {
    if command -v curl >/dev/null 2>&1; then
        curl -fSL --progress-bar "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress -O "$2" "$1"
    else
        die "curl or wget is required"
    fi
}

# Reads an answer from the terminal even when the script itself comes from a pipe.
ask() {
    # /dev/tty can exist yet fail to open (no controlling terminal).
    if (: </dev/tty) 2>/dev/null; then
        printf '%s' "$1" >/dev/tty
        read -r answer </dev/tty || answer=""
    else
        answer=""
    fi
    printf '%s' "$answer"
}

choose_role() {
    say ""
    say "Ce PC est… / This PC is…"
    say "  1) le PC avec les cartes son (serveur) / the PC with the sound cards (server)"
    say "  2) le PC avec le DAW (client) / the PC with the DAW (client)"
    case "$(ask 'Choix / Choice [1/2]: ')" in
        1) role=server ;;
        2) role=client ;;
        *) die "no choice; run again, or pass it: curl -fsSL … | sh -s -- server" ;;
    esac
}

# ─── Linux ────────────────────────────────────────────────

linux_arch_asset() {
    case "$(uname -m)" in
        x86_64 | amd64) printf 'rjsc-linux-x86_64.tar.gz' ;;
        aarch64 | arm64) printf 'rjsc-linux-arm64.tar.gz' ;;
        *) die "unsupported processor: $(uname -m) (x86_64 and arm64 only)" ;;
    esac
}

# The binary needs ALSA's runtime library, missing on some minimal installs.
linux_install_alsa() {
    if command -v ldconfig >/dev/null 2>&1 && ldconfig -p 2>/dev/null | grep -q 'libasound\.so\.2'; then
        return
    fi
    for lib in /usr/lib/*/libasound.so.2 /usr/lib/libasound.so.2 /usr/lib64/libasound.so.2; do
        [ -e "$lib" ] && return
    done
    step "Installing the ALSA library"
    if command -v apt-get >/dev/null 2>&1; then
        as_root apt-get update -q
        as_root apt-get install -y libasound2t64 2>/dev/null || as_root apt-get install -y libasound2
    elif command -v dnf >/dev/null 2>&1; then
        as_root dnf install -y alsa-lib
    elif command -v pacman >/dev/null 2>&1; then
        as_root pacman -S --needed --noconfirm alsa-lib
    elif command -v zypper >/dev/null 2>&1; then
        as_root zypper --non-interactive install libasound2
    else
        say "Install ALSA (libasound2 / alsa-lib) with your package manager if rjsc fails to start."
    fi
}

install_linux() {
    bin_dir="$HOME/.local/bin"
    rjsc="$bin_dir/rjsc"

    if [ "$role" = uninstall ]; then
        [ -x "$rjsc" ] && "$rjsc" autostart disable || true
        rm -f "$rjsc"
        say "RJ45 Sound Card removed (settings kept in ~/.config/rjsc)."
        return
    fi

    linux_install_alsa

    step "Downloading RJ45 Sound Card"
    url="${RJSC_DOWNLOAD_URL:-$RELEASES/$(linux_arch_asset)}"
    download "$url" "$tmp/rjsc.tar.gz"
    tar -xzf "$tmp/rjsc.tar.gz" -C "$tmp"
    mkdir -p "$bin_dir"
    # Stop a running copy first, so the new binary can replace it.
    [ -x "$rjsc" ] && "$rjsc" autostart disable >/dev/null 2>&1 || true
    install -m 755 "$tmp/rjsc" "$rjsc"
    say "Installed: $rjsc"

    if [ "$role" = server ]; then
        # Without a desktop session (started at boot), sound cards are only
        # reachable through the audio group.
        if ! id -nG | tr ' ' '\n' | grep -qx audio; then
            step "Adding $(id -un) to the audio group"
            as_root usermod -aG audio "$(id -un)"
            need_reboot=1
        fi
    fi

    step "Starting RJ45 Sound Card at every startup"
    "$rjsc" autostart enable --role "$role"

    if [ "$role" = client ] && ! "$rjsc" driver status | grep -q '^Virtual sound card:'; then
        step "Installing the virtual sound card"
        as_root "$rjsc" driver install || say "Could not install it now; use the button on http://localhost:42080"
    fi

    say ""
    say "Direct cable: in the network settings, set the wired connection to"
    say "\"Link-Local Only\" (Lien local uniquement) so both PCs get an address on it."
    if [ "${need_reboot:-0}" = 1 ]; then
        say "Restart this PC once so the audio group applies."
    fi
}

# ─── macOS ────────────────────────────────────────────────

install_macos() {
    app_name="RJ45 Sound Card.app"
    if [ -w /Applications ]; then apps=/Applications; else apps="$HOME/Applications"; fi
    app="$apps/$app_name"
    rjsc="$app/Contents/MacOS/rjsc"

    if [ "$role" = uninstall ]; then
        [ -x "$rjsc" ] && "$rjsc" autostart disable || true
        rm -rf "$app"
        say "RJ45 Sound Card removed."
        return
    fi

    step "Downloading RJ45 Sound Card"
    download "${RJSC_DOWNLOAD_URL:-$RELEASES/RJ45-Sound-Card-macOS.zip}" "$tmp/rjsc.zip"
    [ -x "$rjsc" ] && "$rjsc" autostart disable >/dev/null 2>&1 || true
    mkdir -p "$apps"
    rm -rf "$app"
    # Installed by curl, the app carries no quarantine flag: Gatekeeper won't block it.
    ditto -x -k "$tmp/rjsc.zip" "$apps"
    say "Installed: $app"

    if [ -n "$role" ]; then
        step "Starting RJ45 Sound Card at every login"
        "$rjsc" autostart enable --role "$role"
    else
        # Same first-run page as a double-click.
        open "$app"
    fi
}

# ─── Main ─────────────────────────────────────────────────

role="${1:-}"
case "$role" in
    "" | server | client | uninstall) ;;
    *) die "unknown option: $role (server, client or uninstall)" ;;
esac

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

case "$(uname -s)" in
    Linux)
        [ -n "$role" ] || choose_role
        install_linux
        ;;
    Darwin)
        install_macos
        ;;
    *)
        die "this script is for Linux and macOS; on Windows, download RJ45-Sound-Card.exe from https://github.com/$REPO/releases/latest"
        ;;
esac

if [ "$role" != uninstall ]; then
    say ""
    say "Done. Page: http://localhost:42080"
fi
