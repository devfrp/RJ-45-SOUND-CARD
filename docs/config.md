# Configuration Reference

Complete documentation for the `rjsc.toml` configuration file.

Generate a default config with:

```bash
rjsc init
```

## Sections

### `role` — What this PC is (top-level key)

`"server"` or `"client"`. Set by the setup page the first time `rjsc` is double-clicked, and read by `rjsc start` (double-click and autostart): a server runs `serve` with its web panel, a client keeps looking for the server and connects whenever it is reachable. Absent until chosen.

### `[audio]` — Audio device and format

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `input_device` | string | `"@default"` | Input/capture device name. Use `@default` for system default, or a device name from `rjsc list`. |
| `output_device` | string | `"@default"` | Output/playback device name. Use `@default` for system default, or a device name from `rjsc list`. |
| `channels` | u16 | `2` | Number of audio channels. Range: 1–64. |
| `sample_rate` | u32 | `48000` | Sample rate in Hz. Recommended: `44100`, `48000`, `96000`, `192000`. |
| `buffer_frames` | usize | `256` | Sound card buffer size in frames: `64`, `128`, `256`, `512`, `1024`. If the device refuses it or fails with it, its default size is used (logged). |
| `sample_format` | string | `"f32"` | Audio sample format. Options: `"f32"`, `"i16"`, `"i24"`, `"i32"`. Use integer formats to reduce bandwidth or match hardware. |
| `latency_ms` | u32 | `10` | Network margin: target of the buffer that absorbs jitter and clock drift for incoming audio, on both sides. Raised automatically to 1.5 packets + 1 sound card buffer when needed. Range 1–500. Raise it if you hear clicks. |

**Latency per buffer size (at 48 kHz):**

| Buffer | Latency | Bandwidth (2 ch) | Use case |
|--------|---------|-------------------|----------|
| 64 | ~1.3 ms | ~6.1 Mbps | Real-time monitoring |
| 128 | ~2.7 ms | ~6.1 Mbps | Optimal performance |
| 256 | ~5.3 ms | ~3.1 Mbps | Standard usage (default) |
| 512 | ~10.7 ms | ~3.1 Mbps | High-latency network |
| 1024 | ~21.3 ms | ~3.1 Mbps | Troubleshooting |

Bandwidth formula: `channels × sample_rate × 4 bytes × 8 bits / 1,000,000 = Mbps`

### `[network]` — Network configuration

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `audio_port` | u16 | `42001` | UDP port for raw PCM audio streaming. |
| `control_port` | u16 | `42002` | TCP port for JSON control protocol. |
| `discovery_port` | u16 | `42000` | UDP port for broadcast discovery announcements. |
| `bind_address` | string | `"0.0.0.0"` | Network interface address to bind to. Use `0.0.0.0` to listen on all interfaces. |

### `[server]` — Server-specific settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `auto_accept` | bool | `true` | Whether to automatically accept incoming client connections. |
| `max_clients` | u32 | `4` | Maximum number of simultaneously connected clients. Additional connections are rejected. |

### `[client]` — Client-specific settings

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `server_address` | string | `""` | Server address in `ip:port` format. Leave empty to auto-discover via UDP broadcast. Example: `"192.168.1.100:42002"`. |
| `use_virtual_device` | bool | `true` | Whether to send audio to a virtual audio device. Set to `false` to use the system default output directly. |
| `virtual_device_name` | string | `"RJ45 Virtual Audio"` | Name of the virtual audio device, as shown by `rjsc list`. Platform-specific examples below. |
| `auto_reconnect` | bool | `true` | Automatically reconnect to the server if the connection drops. Retry delay: 5 seconds. |
| `jitter_buffer_ms` | u64 | `10` | How long to wait for a missing or late packet (measured as audio queued behind it) before skipping it. Only adds latency when a packet is missing. Use 10 ms on a direct cable, 30-50 ms on Wi-Fi. |

**Virtual device names per platform:**

| Platform | Device name example |
|----------|-------------------|
| Linux (ALSA loopback) | `"hw:Loopback,0,0"` or `"hw:Loopback,0,1"` |
| Linux (PipeWire) | `"RJ45 Virtual Audio"` (use any name) |
| macOS (BlackHole) | `"BlackHole 16ch"` or `"BlackHole 2ch"` |
| Windows (VB-Cable) | `"CABLE Input (VB-Audio Virtual Cable)"` |

### `[encryption]` — Encryption and authentication

| Key | Type | Default | Description |
| `enabled` | bool | `false` | Enable encryption and authentication. |
| `pre_shared_key` | string | `""` | 64-character hex string (32 bytes) for symmetric encryption. Generate with: `openssl rand -hex 32` or `hexdump -n 32 -e '1/1 "%02x"' /dev/urandom`. |

If enabled, the server will challenge the client on connect. Both sides must have the same `pre_shared_key`. Audio packets are XOR-encrypted with per-packet key derivation and include a CRC32 authentication tag.

### `[web]` — Web control panel (server)

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Serve the control panel from `rjsc serve`. |
| `port` | u16 | `42080` | TCP port of the panel, bound on `network.bind_address`. |

Open `http://<server-ip>:<port>` in a browser. The panel edits `[audio]`, `server.max_clients` and `[encryption]`, writes them back to the configuration file and restarts the audio server. It never displays the pre-shared key. It has no password, so anyone who can reach the port can change the settings.

## Example: Dedicated Studio Server

```toml
[audio]
input_device = "MOTU 424"
output_device = "MOTU 424"
channels = 24
sample_rate = 48000
buffer_frames = 128

[network]
audio_port = 42001
control_port = 42002
discovery_port = 42000
bind_address = "0.0.0.0"

[server]
auto_accept = true
max_clients = 1

[client]
use_virtual_device = false
auto_reconnect = false

[encryption]
enabled = false
```

## Example: Laptop Client

```toml
[audio]
input_device = "@default"
output_device = "@default"
channels = 2
sample_rate = 48000
buffer_frames = 256

[network]
audio_port = 42001
control_port = 42002
discovery_port = 42000
bind_address = "0.0.0.0"

[client]
server_address = "192.168.1.100:42002"
use_virtual_device = true
virtual_device_name = "hw:Loopback,0,0"
auto_reconnect = true
jitter_buffer_ms = 50

[encryption]
enabled = false
```

## Example: Encrypted Secure Setup

```toml
[audio]
input_device = "MOTU 424"
output_device = "MOTU 424"
channels = 8
sample_rate = 48000
buffer_frames = 128
sample_format = "f32"

[network]
audio_port = 42001
control_port = 42002

[server]
auto_accept = true
max_clients = 1

[client]
use_virtual_device = true
virtual_device_name = "hw:Loopback,0,0"
server_address = "192.168.1.100:42002"
auto_reconnect = true
jitter_buffer_ms = 30

[encryption]
enabled = true
pre_shared_key = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
```

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `RUST_LOG` | Log level: `error`, `warn`, `info` (default), `debug`, `trace`. Example: `RUST_LOG=debug rjsc serve`. |
| `HOSTNAME` or `COMPUTERNAME` | Used by the server for discovery announcements. Falls back to the `hostname` command, then `"unknown"`. |

## Config File Location

`rjsc` looks for the config file in the current working directory by default. Use `-c` / `--config` to specify a path:

```bash
rjsc serve -c /etc/rj45-sound-card/rjsc.toml
rjsc connect -c ~/.config/rjsc/rjsc.toml
```

If the file is not found, hardcoded defaults are used with a warning.

`rjsc autostart enable` uses a per-user file instead, created if missing: `~/.config/rjsc/rjsc.toml` (Linux), `~/Library/Application Support/rjsc/rjsc.toml` (macOS) or `%APPDATA%\rjsc\rjsc.toml` (Windows).
