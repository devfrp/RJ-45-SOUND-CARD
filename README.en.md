# RJ45 Sound Card

Share any sound card between two PCs over Ethernet (RJ45).

The **server PC** (with the physical sound card, e.g. MOTU, RME, Focusrite…) captures audio and streams it
over the network. The **client PC** (laptop) receives the stream and plays it through a virtual audio device,
making the remote sound card accessible as if it were local.

## Architecture

```
┌─────────────────────────┐       RJ45 (Ethernet)        ┌──────────────────────────┐
│  SERVER (Studio PC)     │◄──────────────────────────►│  CLIENT (Laptop)          │
│                         │  UDP: audio (port 42001)     │                          │
│  Physical sound card    │  TCP: control (port 42002)   │  Virtual Audio Device     │
│  (MOTU, RME, …)         │  UDP: discovery (port 42000) │  (snd-aloop/BlackHole/   │
│                         │                               │   VB-Cable)               │
│  Capture → UDP Send     │                               │  UDP Receive → Playback   │
│  UDP Receive → Playback │                               │  Capture → UDP Send       │
└─────────────────────────┘                               └──────────────────────────┘
```

## Features

- **Cross-platform** : Windows, Linux, macOS
- **Any sound card** : compatible with any audio device recognized by the OS
  (MOTU, RME, Focusrite, Universal Audio, Presonus, etc.)
- **Bidirectional** : audio from server → client AND client → server
- **Low latency** : UDP streaming with configurable buffers (64–1024 frames)
- **Auto-discovery** : client automatically finds servers on the local network
- **Remote control** : volume, device selection, status
- **Multi-channel** : 1 to 64+ channels depending on configuration

## Installation

### Prerequisites

- **Rust** (to compile): `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **Audio library** :
  - Linux: `sudo apt install libasound2-dev` (or pulseaudio/jack)
  - macOS: CoreAudio included
  - Windows: WASAPI included

### Build

```bash
git clone https://github.com/devfrp/rj45-sound-card.git
cd rj45-sound-card
cargo build --release
./target/release/rjsc --help
```

The `rjsc` binary is standalone and can be copied to any machine running the same OS.

### Virtual Audio Device Setup (CLIENT only)

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
# On both machines
rjsc list
```

### 2. Configure

Generate a configuration file and edit it:

```bash
rjsc init
```

Example configuration (`rjsc.toml`):

```toml
[audio]
input_device = "MOTU 424"      # Your device name
output_device = "MOTU 424"     # Your device name
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
# On the studio machine
rjsc serve
```

### 4. Client (laptop)

```bash
# Auto-connect via network discovery
rjsc connect

# Or with a specific address
rjsc connect --server 192.168.1.100:42002
```

## Latency

Latency depends on:

- **Buffer size**: 64 frames → ~1.3ms, 256 → ~5.3ms, 1024 → ~21ms (at 48kHz)
- **Network**: Ethernet switch/router latency
- **Audio drivers**: ASIO (Windows) / JACK (Linux) provide the lowest latency

For real-time monitoring, use 64 or 128 frames with a Gigabit network.

## Troubleshooting

**Client cannot find the server:**
- Ensure both PCs are on the same network
- Check the firewall (open UDP 42000-42001 and TCP 42002)
- Use `--server` to specify the address manually

**No sound on the client:**
- Verify the virtual audio device with `rjsc list`
- Check system sound settings (select the virtual device)
- Increase buffer size

**Latency too high:**
- Reduce `buffer_frames` (64 or 128)
- Use Gigabit Ethernet (not WiFi)
- Enable JACK (Linux) or ASIO (Windows) for low-latency drivers

## Ports

| Port | Protocol | Usage |
|------|----------|-------|
| 42000 | UDP | Service discovery (JSON broadcast every 3s) |
| 42001 | UDP | Audio stream (raw 32-bit float PCM) |
| 42002 | TCP | Control protocol (length-prefixed JSON messages) |

## Specifications

| Specification | Value |
|--------------|-------|
| Language | Rust (edition 2021) |
| OS support | Windows 10/11, Linux (kernel 5.10+), macOS 12+ |
| Architectures | x86_64 (AMD64), ARM64 (Apple Silicon native) |
| Sample rates | 44.1, 48, 88.2, 96, 176.4, 192 kHz |
| Audio format | PCM 32-bit float |
| Channels | 1 (mono) to 64+ simultaneous |
| Buffer | 64–1024 frames (configurable) |
| Minimum latency | ~1.3 ms @ 48 kHz / 64 frames (Gigabit Ethernet) |
| Bandwidth | ~55 Mbps for 48 channels @ 48 kHz |
| GUI | egui/eframe (Rust), real-time control panel |
| License | MIT |

## Documentation

- [Configuration Reference](docs/config.md) — full config file documentation
- [Architecture](ARCHITECTURE.md) — internal design and protocols
- [Contributing](CONTRIBUTING.md) — how to contribute
- [Changelog](CHANGELOG.md) — version history
- [Man page](man/man1/rjsc.1) — `man rjsc`

## License

MIT
