# RJ45 Sound Card

*Read this in: **English** · [Français](README.fr.md)*

🌐 **Website:** <https://devfrp.github.io/RJ-45-SOUND-CARD/>

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
git clone https://github.com/devfrp/RJ-45-SOUND-CARD.git
cd RJ-45-SOUND-CARD
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

## Quick start (no commands)

1. Copy `rjsc` (`rjsc.exe` on Windows) to both PCs, somewhere it will stay.
2. Double-click it. A page opens in the browser and asks what this PC is:
   - **The PC with the sound cards** (server): it starts with the computer from now on and relays
     its sound cards. Adjust it from any browser at `http://<server-ip>:42080`.
   - **The PC with the DAW** (client): it starts with the computer, looks for the server and
     connects as soon as the RJ45 cable is plugged in. Its page (`http://localhost:42080`) shows
     the connection and installs the virtual sound card once (the computer asks for administrator
     permission; this needs internet).
3. Double-clicking `rjsc` again later just opens the page.

Both PCs need an IPv4 address on the cable. Windows and macOS assign one by themselves
(169.254.x.x) after about a minute; on Linux, set the wired connection to "Link-Local Only" (or
"Shared to other computers") in the network settings.

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

The server also serves a **web control panel**: open `http://<server-ip>:42080` in any browser
(on the laptop, for example). It shows the status and connected clients, lets you pick the audio
devices, channels, sample rate and buffer size, set per-channel volumes, and read the logs.
Changes are saved to `rjsc.toml` and applied by restarting the audio server. If the sound card is
missing at startup, the panel stays reachable and the server retries every 10 seconds.

The panel has no password: anyone who can reach the server on the network can change its settings.
Use it on a direct cable or a trusted network, or disable it with `[web] enabled = false`.

#### Start the server automatically

Choosing a role on the double-click page does this for you. From a terminal:

```bash
rjsc autostart enable     # start now and at every startup (as a server unless a role is set)
rjsc autostart status
rjsc autostart disable
```

| OS | Mechanism | Starts |
|----|-----------|--------|
| Linux | systemd user service | at boot (or at login if lingering can't be enabled) |
| macOS | launchd agent | at login |
| Windows | scheduled task "RJ45 Sound Card" | at login |

The server runs in the user session because that is where audio devices are available. On macOS
and Windows, enable automatic login so it starts without anyone logging in. The configuration file
is `~/.config/rjsc/rjsc.toml` (Linux), `~/Library/Application Support/rjsc/rjsc.toml` (macOS) or
`%APPDATA%\rjsc\rjsc.toml` (Windows), created if missing; use `-c` to choose another one. The `rjsc`
binary must stay where it was when you ran the command.

### 4. Client (laptop)

```bash
# Auto-connect (network discovery)
rjsc connect

# Or with a specific address
rjsc connect --server 192.168.1.100:42002
```

## Latency

One-way latency ≈ capture buffer + network margin + playback buffer:

- **Buffer size** (`audio.buffer_frames`): 64 frames → ~1.3ms, 256 → ~5.3ms, 1024 → ~21ms (at 48kHz).
  It is applied to the sound card; if the card or driver refuses it (or fails with it), its default size is used and the log says so.
- **Network margin** (`audio.latency_ms`, default 10 ms): absorbs network jitter and the drift between
  the two sound cards' clocks. It is raised automatically when the packet sizes need more. Raise it if you hear clicks.
- **Audio drivers**: ASIO (Windows) / JACK (Linux) offer the lowest latency.

Two sound cards never run at exactly the same speed. Each side measures how fast its buffer fills and
resamples the incoming audio by a few ppm to compensate, so there are no periodic clicks or slowly growing
latency. Every 30 s the log shows the buffer level, the measured clock drift and any underruns.

For real-time monitoring, use 64 or 128 frames on a direct cable or Gigabit network.

## Troubleshooting

**Client can't find the server:**
- Make sure both PCs are on the same network
- Check firewall (open UDP ports 42000-42001, TCP ports 42002 and 42080)
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
