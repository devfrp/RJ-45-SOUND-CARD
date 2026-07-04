#!/usr/bin/env bash
#
# RJ45 Sound Card - macOS Setup Script
# Installs and configures BlackHole virtual audio driver
# so the remote PC sees the shared sound card as a local device.
#
# Usage:
#   ./mac_setup.sh [install|remove|status]
#
set -euo pipefail

SCRIPT_NAME="$(basename "$0")"
VIRTUAL_DEVICE="RJ45 Virtual Audio"
BLACKHOLE_URL="https://github.com/ExistentialAudio/BlackHole/releases/latest/download/BlackHole.pkg"
BLACKHOLE_PKG="/tmp/BlackHole.pkg"
BLACKHOLE_BUNDLE="com.ExistentialAudio.BlackHole"

print_usage() {
    cat <<EOF
Usage: sudo $SCRIPT_NAME [command]

Commands:
  install   Install BlackHole virtual audio driver and configure
  remove    Uninstall BlackHole virtual audio driver
  status    Show status of virtual audio device
  help      Show this help message

Prerequisites:
  - macOS 10.13+ (High Sierra or later)
  - Homebrew (recommended for dependencies)
  - Administrator privileges (for driver installation)

BlackHole is an open-source virtual audio driver that creates
a loopback audio device with up to 16 channels.

After installation, the virtual device "BlackHole 16ch" will be
available. Use it with rjsc client:
  rjsc connect --virtual-device "BlackHole 16ch"

You can also create an Aggregate Device in Audio MIDI Setup
to combine BlackHole with your physical output for monitoring.
EOF
}

check_sudo() {
    if [[ "$EUID" -ne 0 ]] && [[ "$1" != "status" ]]; then
        echo "Error: install/remove requires sudo." >&2
        exit 1
    fi
}

install_driver() {
    echo "==> Checking for existing BlackHole installation..."
    if system_profiler SPAudioDataType 2>/dev/null | grep -qi "BlackHole"; then
        echo "    BlackHole is already installed."
        return 0
    fi

    echo "==> Downloading BlackHole virtual audio driver..."
    if command -v curl &>/dev/null; then
        curl -L -o "$BLACKHOLE_PKG" "$BLACKHOLE_URL" --progress-bar
    elif command -v wget &>/dev/null; then
        wget -O "$BLACKHOLE_PKG" "$BLACKHOLE_URL"
    else
        echo "Error: curl or wget required. Install via: brew install curl"
        exit 1
    fi

    echo "==> Installing BlackHole driver..."
    installer -pkg "$BLACKHOLE_PKG" -target /Local

    echo "==> Cleaning up..."
    rm -f "$BLACKHOLE_PKG"

    echo ""
    echo "==> BlackHole installed successfully!"
    echo ""
    echo "Next steps:"
    echo "  1. Open 'Audio MIDI Setup' (Applications/Utilities)"
    echo "  2. Verify 'BlackHole 16ch' appears in the device list"
    echo "  3. (Optional) Create an Aggregate Device to hear audio"
    echo "     from both BlackHole and your speakers/headphones"
    echo ""
    echo "To use with rjsc client:"
    echo "  rjsc connect --virtual-device \"BlackHole 16ch\""
}

remove_driver() {
    echo "==> Unloading kernel extension..."
    kextstat | grep "$BLACKHOLE_BUNDLE" | awk '{print $1}' | xargs -I{} kextunload {} 2>/dev/null || true

    echo "==> Removing BlackHole driver files..."
    sudo rm -rf "/Library/Audio/Plug-Ins/HAL/BlackHole.driver"
    sudo rm -rf "/Library/Extensions/BlackHole.kext"

    echo "==> Rebooting driver cache..."
    sudo touch /Library/Extensions
    sudo kextcache -clear

    echo "==> BlackHole removed."
    echo "    Reboot recommended to complete removal."
}

show_status() {
    echo "=== Virtual Audio Device Status ==="
    echo ""

    if system_profiler SPAudioDataType 2>/dev/null | grep -A 10 "BlackHole"; then
        echo "BlackHole: INSTALLED"
        system_profiler SPAudioDataType 2>/dev/null | grep -A 10 "BlackHole"
    else
        echo "BlackHole: NOT INSTALLED"
    fi
    echo ""

    if system_profiler SPAudioDataType 2>/dev/null | grep -q "Aggregate"; then
        echo "Aggregate Devices:"
        system_profiler SPAudioDataType 2>/dev/null | grep -A 5 "Aggregate" || true
    else
        echo "Aggregate Devices: NONE"
    fi
    echo ""

    echo "All audio devices:"
    system_profiler SPAudioDataType 2>/dev/null | grep -E "^\s+" | head -20
}

case "${1:-help}" in
    install)
        install_driver
        ;;
    remove)
        check_sudo "$@"
        remove_driver
        ;;
    status)
        show_status
        ;;
    help|--help|-h)
        print_usage
        ;;
    *)
        echo "Unknown command: $1"
        print_usage
        exit 1
        ;;
esac
