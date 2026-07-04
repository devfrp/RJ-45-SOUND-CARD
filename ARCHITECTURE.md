# Architecture

Internal design and protocols of RJ45 Sound Card (RJSC).

## Overview

RJSC is a network audio bridge composed of two roles — **server** and **client** — that communicate over three protocols on three distinct ports.

```
┌───────────────────┐                           ┌───────────────────┐
│     SERVER        │                           │      CLIENT       │
│                   │◄─────────────────────────►│                   │
│  ┌─────────────┐  │   UDP 42001 (audio)       │  ┌─────────────┐  │
│  │ cpal capture │──┼───────────────────────────┼─►│ cpal output  │  │
│  │ cpal output  │◄─┼───────────────────────────┼──│ cpal capture │  │
│  └─────────────┘  │                           │  └─────────────┘  │
│                   │   TCP 42002 (control)      │                   │
│  ┌─────────────┐  │◄─────────────────────────►│  ┌─────────────┐  │
│  │ JSON handler │  │                           │  │ JSON handler │  │
│  └─────────────┘  │                           │  └─────────────┘  │
│                   │   UDP 42000 (discovery)     │                   │
│  ┌─────────────┐  │──► 255.255.255.255 ───────►│  ┌─────────────┐  │
│  │ broadcast   │  │    every 3 seconds          │  │ listener    │  │
│  └─────────────┘  │                           │  └─────────────┘  │
└───────────────────┘                           └───────────────────┘
```

## Thread Model

RJSC uses a hybrid async + thread model:

```
main thread (tokio async)
├── discovery broadcast task (tokio::spawn)
├── audio capture → network task (tokio::spawn)
├── network → audio playback task (tokio::spawn)
├── TCP accept loop (main task)
│   └── per-client handler tasks (tokio::spawn)
│
GUI thread (separate, eframe)
└── backend Tokio runtime (std::thread::spawn)
    ├── server sub-thread (std::thread::spawn → Tokio runtime)
    └── client event loop (same runtime)
```

Audio data flows through `crossbeam` bounded channels (capacity 32) between the audio thread (cpal callbacks) and the network thread (tokio async tasks). The client also maintains a jitter buffer for reordering and smoothing incoming UDP audio packets.

## Audio Pipeline

```
Capture (cpal callback, real-time thread)
    │  f32 samples
    ▼
crossbeam::channel (bounded, 32 slots)
    │  Vec<f32>
    ▼
tokio::spawn task (async)
    │  Apply per-channel volume (RwLock<Vec<f32>>)
    │  Build AudioPacketHeader (40-byte binary)
    │  Serialize as: header_bytes + sample_le_bytes
    ▼
UdpSocket::send_to()  ─────  network  ─────►  UdpSocket::recv_from()
                                                   │
                                                   ▼
                                              Parse header (validate magic 0x524A3435)
                                                   │
                                                   ▼
                                              crossbeam::channel (bounded, 32 slots)
                                                   │  Vec<f32>
                                                   ▼
                                              Playback (cpal callback, real-time thread)
                                                   │  Write to output buffer or fill silence
```

### AudioPacketHeader (40 bytes, little-endian)

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 4 | magic | Constant `0x524A3435` = ASCII "RJ45" |
| 4 | 4 | stream_id | Assigned by server, increments per connection |
| 8 | 8 | sequence | Monotonically increasing per stream |
| 16 | 8 | timestamp_us | Microseconds (reserved, always 0) |
| 24 | 2 | channels | Number of audio channels |
| 26 | 4 | sample_rate | Sample rate in Hz |
| 30 | 1 | sample_format | 0 = f32, 1 = i16, 2 = i24, 3 = i32 |
| 31 | 1 | _reserved | Padding |
| 32 | 4 | frame_count | Number of frames in this packet |
| 36 | 4 | data_len | Bytes of audio data following header |

The audio data follows immediately: `frame_count * channels * sample_bytes` of little-endian PCM (see `sample_format`).

When encryption is enabled, a 4-byte CRC32 authentication tag is appended to the packet:

```
header_bytes + encrypted_audio + auth_tag_le_bytes
```

## Control Protocol (TCP 42002)

JSON messages, each prefixed by a 4-byte little-endian length field.

### Message Types

| Message | Direction | Fields |
|---------|-----------|--------|
| `list_devices` | Client → Server | (none) |
| `device_list` | Server → Client | `devices: [DeviceInfo]` |
| `select_device` | Client → Server | `device_id`, `channels`, `sample_rate` |
| `device_selected` | Server → Client | `device_id`, `channels`, `sample_rate`, `stream_id` |
| `start_stream` | Client → Server | `direction: "both"` |
| `stop_stream` | Client → Server | (none) |
| `status` | Server → Client | `running`, `uptime_secs`, `clients`, `stream_id`, `audio_{rx,tx}_kbps` |
| `set_volume` | Client → Server | `channel: u16`, `volume: f32` |
| `error` | Server → Client | `message: string` |
| `ping` | Client → Server | (none) |
| `pong` | Server → Client | (none) |
| `auth` | Server → Client | `challenge: hex string` |
| `auth_response` | Client → Server | `response: hex string` |
| `auth_ok` | Server → Client | (none) |

### DeviceInfo

```json
{
  "index": 0,
  "name": "MOTU 424",
  "input_channels": 24,
  "output_channels": 24,
  "sample_rates": [44100, 48000, 96000, 192000]
}
```

### Typical Session Flow

```
Client                              Server
  │                                    │
  │──── TCP connect ──────────────────►│
  │──── list_devices ─────────────────►│
  │◄─── device_list ──────────────────│
  │──── select_device ────────────────►│
  │◄─── device_selected ──────────────│  (stream_id assigned)
  │──── start_stream {"direction":"both"}►│
  │◄─── status {"running":true} ──────│
  │                                    │
  │◄═══ UDP audio (42001) ════════════│  (continuous)
  │════ UDP audio (42001) ════════════►│  (bidirectional)
  │                                    │
  │──── ping (every 10s) ─────────────►│
  │◄─── pong ─────────────────────────│
  │                                    │
  │──── stop_stream ──────────────────►│
  │──── TCP close ────────────────────►│
```

## Discovery (UDP 42000)

The server broadcasts a JSON payload to `255.255.255.255:42000` every 3 seconds:

```json
{
  "hostname": "studio-pc",
  "device_name": "MOTU 424",
  "device_channels": 24,
  "audio_port": 42001,
  "control_port": 42002,
  "protocol_version": "1.0.0"
}
```

The client listens on `0.0.0.0:42000` for the configured timeout (default 5s) and collects all unique servers (deduplicated by socket address).

## Jitter Buffer

The client maintains a jitter buffer for incoming UDP audio packets to handle network jitter, packet reordering, and packet loss:

- Implemented as a `BTreeMap` keyed by sequence number, ensuring packets are played in order regardless of arrival order.
- **Configurable max latency** — defaults to 50 ms (`jitter_buffer_latency_ms` in config). Packets older than this threshold are dropped.
- **Duplicate removal** — if a packet with a sequence number already buffered arrives, it is silently discarded.
- **Gap skipping** — when a sequence gap is detected and the next expected packet hasn't arrived within the latency window, playback advances past the gap (silence is inserted).

## Encryption

Optional pre-shared key (PSK) encryption is available for both audio and control traffic:

- **Audio encryption**: XOR-based stream cipher using per-packet key derivation via [SipHash](https://en.wikipedia.org/wiki/SipHash). The SipHash of the 40-byte header (excluding magic) produces a unique key for each packet.
- **Authentication tag**: A 4-byte CRC32 checksum of the plaintext audio data is appended after the encrypted audio payload, allowing the receiver to verify integrity.
- **Control connection**: Challenge-response authentication over TCP. The server sends a random challenge (`auth` message), the client responds with a SipHash-based hash of (challenge + PSK) (`auth_response`), and the server replies with `auth_ok` on success.
- Configured via the `[encryption]` section in `rjsc.toml` (see below).

## Configuration System

Configuration is loaded from a TOML file (`rjsc.toml` by default). If the file doesn't exist, hardcoded defaults are used. The `Settings` struct is deserialized via `serde` + `toml`. The `[encryption]` section allows setting a pre-shared key and toggling encryption on/off.

See [docs/config.md](docs/config.md) for the full reference.

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `cpal` | Cross-platform audio capture/playback (ALSA/CoreAudio/WASAPI) |
| `tokio` | Async runtime for network I/O |
| `crossbeam` | Fast MPMC channels for audio buffer passing |
| `clap` | CLI argument parsing with derive macros |
| `serde`/`serde_json` | JSON serialization for control protocol |
| `toml` | Configuration file parsing |
| `eframe`/`egui` | Optional GUI control panel |
| `anyhow` | Error handling |
| `log`/`env_logger` | Structured logging |
| `siphasher` | SipHash for per-packet key derivation |
| `hex` | Hex encoding/decoding for keys |
| `crc32fast` | CRC32 checksums for auth tags |
| `fastrand` | Fast random number generation for challenges |

## Build and Packaging

`scripts/build-release.sh` orchestrates the full release pipeline:

1. Builds Linux binary (`cargo build --release --features gui`)
2. Cross-compiles Windows binary (`--target x86_64-pc-windows-gnu --no-default-features`)
3. Generates role-specific wrappers (`rjsc-server`, `rjsc-client`, `.bat` files)
4. Generates role-specific configs (`rjsc-server.toml`, `rjsc-client.toml`)
5. Generates the man page (`rjsc.1`)
6. Packages into DEB, RPM, Arch Linux, Windows zip, and macOS zip formats

## Thread Safety

- `crossbeam::channel` for audio data (lock-free MPMC, suitable for real-time callbacks)
- `std::sync::atomic::{AtomicU32, AtomicU64}` for shared counters (stream ID, sequence, client count)
- `tokio::sync::Mutex` for `current_client_addr` (async-compatible lock)
- `tokio::sync::RwLock` for per-channel volumes (many readers, few writers)
- `Arc` for shared ownership of sockets and state across tasks

## Limitations

- Virtual device name must be configured (auto-detection is attempted but may require manual fallback)
- Server binds to a single network interface (`bind_address`)
