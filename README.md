> [🇫🇷 Version française](README.fr.md)

# RJ45 Sound Card

Share any sound card between two PCs over Ethernet (RJ45).

The **server PC** (with the physical sound card, e.g. MOTU, RME, Focusrite...) captures audio and streams it
over the network. The **client PC** (laptop) receives the stream and plays it through a virtual audio device,
making the remote sound card accessible as if it were local.

## Architecture

```
┌─────────────────────────┐       RJ45 (Ethernet)        ┌──────────────────────────┐
│  SERVER (Studio PC)     │◄──────────────────────────►│  CLIENT (Laptop)          │
│                         │  UDP: audio stream (port 42001)│                         │
│  Physical sound card    │  TCP: control     (port 42002)│  Virtual Audio Device    │
│  (MOTU, RME, …)         │  UDP: discovery   (port 42000)│  (snd-aloop/BlackHole/  │
│                         │                               │   VB-Cable)              │
│  Capture → UDP Send     │                               │  UDP Receive → Playback  │
│  UDP Receive → Playback │                               │  Capture → UDP Send      │
└─────────────────────────┘                               └──────────────────────────┘
```

## Features

- **Cross-platform**: Windows, Linux, macOS
- **All sound cards**: compatible with any audio device recognized by the OS
  (MOTU, RME, Focusrite, Universal Audio, Presonus, etc.)
- **Bidirectional**: server → client audio AND client → server audio
- **Low latency**: UDP streaming with configurable buffers (64–1024 frames)
- **Jitter buffer**: packet reordering and jitter compensation for smooth playback
- **Encryption**: optional pre-shared key encryption and authentication
- **Integer PCM**: f32, i16, i24, i32 sample formats with automatic conversion
- **Auto-discovery**: the client automatically finds servers on the network
- **Auto-detect virtual device**: finds Loopback, BlackHole, or VB-Cable automatically
- **Remote control**: volume, device selection, status
- **Multi-channel**: supports 1 to 64+ channels depending on configuration
- **Multi-client**: server supports multiple simultaneous client connections
- **Daemon mode**: run as a background service with `--daemon` flag or systemd

## Installation

### Prerequisites

- **Rust** (to compile): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **PortAudio** (audio library):
  - Linux: `sudo apt install libasound2-dev` (or `pulseaudio`, `jack`)
  - macOS: already included with CoreAudio
  - Windows: already included with WASAPI

### Compilation

```bash
git clone https://github.com/devfrp/rj45-sound-card.git
cd rj45-sound-card
cargo build --release
./target/release/rjsc --help
```

The `rjsc` binary is self-contained and can be copied to any machine running the same OS.

### Virtual Audio Device Setup (CLIENT)

So the client PC sees the remote sound card as a local device:

**Linux**:
```bash
sudo ./scripts/linux_setup.sh install
```

**macOS**:
```bash
sudo ./scripts/mac_setup.sh install
```

**Windows** (Administrator PowerShell):
```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows_setup.ps1 install
```

## Usage

### 1. List available audio devices

```bash
# On the server (PC with the sound card)
rjsc list

# On the client (laptop)
rjsc list
```

### 2. Configure

Create a configuration file and edit it:

```bash
rjsc init
```

Example configuration (`rjsc.toml`):

```toml
[audio]
input_device = "MOTU 424"      # Example: your device name
output_device = "MOTU 424"     # Example: your device name
channels = 8                   # Number of channels to share
sample_rate = 48000            # Sample rate
buffer_frames = 256            # Buffer size (latency)

[network]
audio_port = 42001
control_port = 42002
bind_address = "0.0.0.0"

[client]
use_virtual_device = true
virtual_device_name = "hw:Loopback,0,0"  # Linux
# virtual_device_name = "BlackHole 16ch"  # macOS
# virtual_device_name = "CABLE Input (VB-Audio Virtual Cable)"  # Windows
auto_reconnect = true
```

### 3. Server (PC with the physical sound card)

```bash
# On the studio machine (with the physical sound card)
rjsc serve
```

### 4. Client (laptop)

```bash
# Auto-connect (network discovery)
rjsc connect

# Or with a specific address
rjsc connect --server 192.168.1.100:42002
```

## Latency

Latency depends on:

- **Buffer size**: 64 frames → ~1.3ms, 256 → ~5.3ms, 1024 → ~21ms (at 48kHz)
- **Network**: Ethernet switch/router latency
- **Audio drivers**: ASIO (Windows) / JACK (Linux) offer the lowest latency

For real-time monitoring, use 64 or 128 frames with a Gigabit network.

## Troubleshooting

**Client can't find the server:**
- Make sure both PCs are on the same network
- Check firewall (open UDP ports 42000-42001, TCP port 42002)
- Use `--server` to specify the address manually

**No sound on the client:**
- Check the virtual audio device with `rjsc list`
- Check system sound settings (select the virtual device)
- Increase the buffer size

**Latency too high:**
- Reduce `buffer_frames` (64 or 128)
- Use a Gigabit network (not WiFi)
- Enable JACK (Linux) or ASIO (Windows) for low-latency drivers

## License

MIT

## Documentation

- [Configuration reference](docs/config.md) — all `rjsc.toml` options
- [Architecture](ARCHITECTURE.md) — internal design and protocols
- [Contributing](CONTRIBUTING.md) — contribution guide
- [Changelog](CHANGELOG.md) — version history
- [Man page](man/man1/rjsc.1) — `man rjsc`
- [English README](README.en.md)
