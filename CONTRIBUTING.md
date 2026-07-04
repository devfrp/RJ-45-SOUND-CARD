# Contributing to RJ45 Sound Card

Thanks for your interest in contributing! This document explains how to get started.

## Development Setup

```bash
git clone https://github.com/devfrp/rj45-sound-card.git
cd rj45-sound-card
cargo build
cargo test
cargo run -- --help
```

### Dependencies

- **Rust** 1.96+
- **ALSA** development libraries (Linux): `sudo apt install libasound2-dev`
- macOS and Windows have built-in audio APIs (no extra dependencies)

### Feature Flags

- `gui` (default): includes the egui/eframe graphical control panel
- Build without GUI: `cargo build --no-default-features`

## Project Structure

```
src/
├── main.rs          # CLI entry point (clap)
├── gui.rs           # egui/eframe control panel (feature-gated)
├── audio/
│   ├── mod.rs       # Device listing and discovery
│   ├── capture.rs   # Audio input stream
│   ├── format.rs    # AudioFormat struct
│   └── playback.rs  # Audio output stream
├── config/
│   └── mod.rs       # TOML configuration loading
├── client/
│   └── mod.rs       # Client mode (discover, connect, stream)
├── server/
│   └── mod.rs       # Server mode (capture, broadcast, accept)
└── net/
    ├── mod.rs       # Network module root
    ├── audio_stream.rs  # UDP audio packet format
    ├── control.rs   # TCP control protocol
    └── discovery.rs # UDP broadcast auto-discovery
```

## Code Style

- Standard Rust conventions (`cargo fmt`, `cargo clippy`)
- Use `anyhow::Result` for error handling throughout
- Logging via `log` crate, configured with `env_logger`
- Async networking uses `tokio`, audio uses `cpal` + `crossbeam` channels

## Testing

```bash
cargo test
cargo clippy --all-targets
cargo fmt --check
```

Manual testing workflow:
1. On the server machine: `cargo run -- serve`
2. On the client machine: `cargo run -- connect --server <server_ip>:42002`
3. Test with `cargo run -- gui` for GUI testing

## Pull Request Process

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes and ensure tests pass
4. Run `cargo fmt` and `cargo clippy`
5. Submit a pull request with a clear description

## Reporting Issues

Use GitHub Issues at https://github.com/devfrp/rj45-sound-card/issues.

Include:
- OS and version
- Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs actual behavior
- Log output with `RUST_LOG=debug`

## Adding Documentation

- User-facing docs: `README.md`, `docs/`, `man/man1/rjsc.1`
- Developer docs: `ARCHITECTURE.md`
- Update `CHANGELOG.md` with any user-visible changes

## Release Process

Releases are built with `scripts/build-release.sh`:

```bash
./scripts/build-release.sh 0.2.0
```

This generates DEB, RPM, Arch, Windows, and macOS packages.
