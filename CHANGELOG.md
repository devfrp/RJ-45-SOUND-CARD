# Changelog

All notable changes to RJ45 Sound Card are documented here.

## [1.0.1] — 2026-06-20

### Added
- **Jitter buffer**: packet reordering and timing jitter compensation on the client side. Configurable via `client.jitter_buffer_ms` (default 50ms). Eliminates audio glitches from out-of-order or delayed UDP packets.
- **Encryption and authentication**: optional pre-shared key encryption for audio streams and challenge-response authentication for control connections. Configure via `[encryption]` section.
- **Integer PCM support**: i16, i24, and i32 sample formats in addition to f32. Configure via `audio.sample_format`. Useful for compatibility with audio hardware that expects integer formats.
- **Per-client stream tracking**: each connected client now gets its own stream ID and independent sequence counter, fixing the single-stream limitation for multi-client deployments.
- **Virtual audio device auto-detection**: automatically detects platform-specific virtual devices (ALSA Loopback, BlackHole, VB-Cable) when configured device is not found.
- **Daemon mode**: `rjsc serve --daemon` and `rjsc connect --daemon` flags for background operation.
- **systemd service**: `deploy/rjsc-server.service` for running the server as a system service on Linux.
- **CI/CD pipeline**: GitHub Actions workflow for cross-platform builds, tests, and releases.
- **Unit and integration tests**: comprehensive test suite covering audio formats, jitter buffer, encryption, and protocol.

### Changed
- Audio format now includes `sample_format` field (backward compatible, defaults to f32).
- `send_audio_packet` and `parse_audio_packet` now support optional encryption.
- Server uses per-client HashMap for stream state instead of single global stream_id.
- Client receive path uses jitter buffer for smoother audio playback.
- Control protocol now supports `auth`, `auth_response`, and `auth_ok` messages.

### Fixed
- Multi-client audio streaming now properly assigns independent stream IDs and sequences.

## [1.0.0] — 2026-06-16

### Added
- Initial release
- Server mode: share local audio devices over Ethernet/RJ45
- Client mode: connect to remote server and receive audio as a local virtual device
- Bidirectional audio streaming (server → client and client → server)
- UDP audio transport with custom 40-byte binary header (port 42001)
- TCP JSON control protocol (port 42002)
- UDP broadcast auto-discovery (port 42000, every 3 seconds)
- TOML configuration file (`rjsc.toml`) with `rjsc init`
- Graphical control panel (egui/eframe, feature-gated)
- Per-channel volume control (0.0–1.0 slider)
- Multi-channel support (1–64+ channels)
- Configurable sample rates: 44.1, 48, 96, 192 kHz
- Configurable buffer sizes: 64, 128, 256, 512, 1024 frames
- Auto-reconnect with configurable delay (client mode)
- Max clients limit (server mode)
- Keep-alive ping/pong over TCP
- Virtual audio device setup scripts:
  - Linux: ALSA snd-aloop
  - macOS: BlackHole
  - Windows: VB-Cable
- Multi-distro packaging: DEB, RPM, Arch Linux, Windows zip, macOS zip
- Man page (`rjsc.1`)
- Cross-compilation for Windows (headless)
- ARM64 support (Apple Silicon native)

[1.0.1]: https://github.com/devfrp/rj45-sound-card/releases/tag/v1.0.1
[1.0.0]: https://github.com/devfrp/rj45-sound-card/releases/tag/v1.0.0
