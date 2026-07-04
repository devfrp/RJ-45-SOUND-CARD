#!/usr/bin/env bash
#
# RJ45 Sound Card - Linux Setup Script
# Creates a virtual ALSA loopback device so the remote PC sees
# the shared sound card as a local audio device.
#
# Usage:
#   sudo ./linux_setup.sh [install|remove|status]
#
set -euo pipefail

SCRIPT_NAME="$(basename "$0")"
ALSACONF="/etc/modprobe.d/rjsc-alsa-loopback.conf"
MODULE="snd_aloop"
VIRTUAL_DEVICE_NAME="RJ45 Virtual Audio"
ASOUNDRC="$HOME/.asoundrc"

print_usage() {
    cat <<EOF
Usage: sudo $SCRIPT_NAME [command]

Commands:
  install   Load and configure snd_aloop module, create ALSA config
  remove    Unload snd_aloop and remove configuration
  status    Show status of virtual audio device
  help      Show this help message

Examples:
  sudo ./$SCRIPT_NAME install
  sudo ./$SCRIPT_NAME status
EOF
}

check_root() {
    if [[ "$EUID" -ne 0 ]]; then
        echo "Error: This script must be run as root (sudo)." >&2
        exit 1
    fi
}

install_module() {
    echo "==> Loading snd_aloop kernel module..."
    if lsmod | grep -q "^$MODULE"; then
        echo "    Module $MODULE already loaded."
    else
        if modinfo "$MODULE" &>/dev/null; then
            modprobe "$MODULE" index=2 enable=1,1,1,1,1,1,1,1 pcm_substreams=2,2,2,2,2,2,2,2
            echo "    Module $MODULE loaded successfully."
        else
            echo "Error: $MODULE module not found. Install it with:"
            echo "  sudo apt-get install linux-modules-extra-$(uname -r)"
            exit 1
        fi
    fi

    # Make persistent
    echo "==> Making module persistent..."
    if ! grep -q "^$MODULE" /etc/modules 2>/dev/null; then
        echo "$MODULE" >> /etc/modules
        echo "    Added $MODULE to /etc/modules."
    else
        echo "    Already in /etc/modules."
    fi

    # Module options for persistence
    echo "==> Setting module options..."
    mkdir -p /etc/modprobe.d
    cat > "$ALSACONF" <<EOF
# RJ45 Sound Card - ALSA loopback module options
# Installed by $SCRIPT_NAME
options snd_aloop index=2 enable=1,1,1,1,1,1,1,1 pcm_substreams=2,2,2,2,2,2,2,2
EOF
    echo "    Module options written to $ALSACONF."

    echo ""
    echo "==> Setup complete!"
    echo ""
    echo "A virtual ALSA device is now available. It has:"
    echo "  - Playback device: hw:Loopback,0 (output to virtual device)"
    echo "  - Capture device:  hw:Loopback,1 (input from virtual device)"
    echo ""
    echo "To use it with rjsc client, run:"
    echo "  rjsc connect --virtual-device \"$VIRTUAL_DEVICE_NAME\""
    echo ""
    echo "List all ALSA devices:"
    echo "  aplay -l | grep -i loop"
    echo "  arecord -l | grep -i loop"
}

remove_module() {
    echo "==> Removing module options..."
    rm -f "$ALSACONF"
    echo "    Removed $ALSACONF."

    sed -i "/^$MODULE/d" /etc/modules 2>/dev/null || true
    echo "    Removed from /etc/modules."

    echo "==> Unloading module..."
    if lsmod | grep -q "^$MODULE"; then
        rmmod "$MODULE"
        echo "    Module $MODULE unloaded."
    else
        echo "    Module not loaded."
    fi

    echo "==> Done."
}

show_status() {
    echo "=== Status of $VIRTUAL_DEVICE_NAME ==="
    echo ""

    if lsmod | grep -q "^$MODULE"; then
        echo "Module: LOADED"
        lsmod | grep "^$MODULE"
    else
        echo "Module: NOT LOADED"
    fi
    echo ""

    if aplay -l 2>/dev/null | grep -qi loopback; then
        echo "Playback devices:"
        aplay -l 2>/dev/null | grep -i loopback
    else
        echo "Playback devices: NONE"
    fi
    echo ""

    if arecord -l 2>/dev/null | grep -qi loopback; then
        echo "Capture devices:"
        arecord -l 2>/dev/null | grep -i loopback
    else
        echo "Capture devices: NONE"
    fi
    echo ""

    echo "Available PCM devices:"
    aplay -L 2>/dev/null | grep -i loop || echo "  No loopback PCM devices found"
    echo ""

    echo "For client usage, configure rjsc with:"
    echo "  output_device = \"hw:Loopback,0,0\""
    echo "  input_device  = \"hw:Loopback,1,0\""
}

case "${1:-help}" in
    install)
        check_root
        install_module
        ;;
    remove)
        check_root
        remove_module
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
