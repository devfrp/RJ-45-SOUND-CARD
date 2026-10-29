# Changelog

All notable changes to RJ45 Sound Card are documented here.

## [1.1.0] — 2026-10-29

### Added
- **Installation without commands**: `curl -fsSL https://devfrp.github.io/RJ-45-SOUND-CARD/install.sh | sh` on Linux (x86_64, arm64; apt/dnf/pacman/zypper) and macOS; **RJ45 Sound Card.app** (universal) on macOS; standalone `RJ45-Sound-Card.exe` (static C runtime) on Windows. Release assets come with `SHA256SUMS`.
- `rjsc autostart enable --role server|client` and `rjsc driver install|status`, used by the install script (e.g. on a server without a screen).
- **Web control panel**: `rjsc serve` serves a page on port 42080 (`[web]` section) to see the status and connected clients, choose devices and format, set per-channel volumes and read the logs, in French or English. Settings are saved to the configuration file and applied by restarting the audio server.
- **Automatic retry**: when the audio server fails (e.g. sound card not plugged in yet), it retries every 10 seconds while the panel stays available.
- **`rjsc autostart enable|disable|status`**: start the server at every startup (systemd user service, launchd agent or Windows scheduled task).
- Discovery announcements include the panel port; the client logs the panel URL.
- **No commands needed**: double-clicking `rjsc` opens a page that asks whether this PC is the server (sound cards) or the client (DAW), saves the choice (`role`), enables autostart and hands over to the background instance. `rjsc start` runs the saved role.
- **Laptop page** (`http://localhost:42080`): connection status, link to the server panel, one-click virtual sound card install through the OS administrator prompt.

- **Clock drift compensation**: incoming audio goes through a playout buffer that resamples by a few ppm to follow the sender's sound card clock, so long sessions no longer click periodically or drift in latency. Buffer level, drift and underruns are logged every 30 s.
- `audio.latency_ms` (default 10 ms): network margin for incoming audio, editable in the panel.

### Changed
- Lower latency defaults: `client.jitter_buffer_ms` 50 → 10 ms.
- `audio.buffer_frames` is now applied to the sound card (it was ignored); a size the device refuses or fails with falls back to its default.

### Fixed
- Playback no longer chops the audio: it used one network packet per sound card callback, dropping or padding the difference whenever their sizes differed.
- The jitter buffer counted missing packets as samples, so one lost packet stalled playback for about 12 seconds.
- A failing audio stream no longer floods the log with thousands of errors per second.
- Client and server no longer wait on each other forever at connection time when encryption is off (no audio was streamed).
- Ctrl-C now stops the client when `auto_reconnect` is on, and stops the server without waiting for another client connection.
- Reconnecting no longer fails because the previous session still held the UDP audio port.
- Clippy errors in `net/audio_stream.rs` with recent toolchains.
- Discovery announcements are sent on every network interface, so a direct cable is found next to Wi-Fi or on a PC without a default route, and a failed send no longer stops them for good.

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

[1.0.1]: https://github.com/devfrp/RJ-45-SOUND-CARD/releases/tag/v1.0.1
[1.0.0]: https://github.com/devfrp/RJ-45-SOUND-CARD/releases/tag/v1.0.0
