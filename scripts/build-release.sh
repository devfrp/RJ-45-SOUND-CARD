#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-1.0.0}"
TOP="$(cd "$(dirname "$0")/.." && pwd)"
RELEASE_DIR="$TOP/release"
TARGET_DIR="$TOP/target"

log() { echo "==> $*"; }
small() { echo "    $*"; }

log "Building release $VERSION"

# Build Linux binary (full with GUI)
log "Building Linux binary..."
cargo build --release --features gui

# Build Windows binary (headless, no GUI)
log "Building Windows binary (headless)..."
cargo build --release --target x86_64-pc-windows-gnu --no-default-features

BIN_LINUX="$TARGET_DIR/release/rjsc"
BIN_WIN="$TARGET_DIR/x86_64-pc-windows-gnu/release/rjsc.exe"

# ─── helpers ──────────────────────────────────────────────

make_control() {
  local pkg="$1" mode="$2" desc="$3"
  cat > "$RELEASE_DIR/deb/$pkg/DEBIAN/control" << CONTROLEOF
Package: $pkg
Version: $VERSION
Section: sound
Priority: optional
Architecture: amd64
Maintainer: RJ45 Sound Card Team
Description: $desc
 Share any audio device between two PCs via an RJ45 Ethernet cable.
 This package contains the $mode binary and configuration.
CONTROLEOF
}

make_deb() {
  local pkg="$1" mode="$2" pkgdesc="$3"
  local dir="$RELEASE_DIR/deb/$pkg"
  mkdir -p "$dir/DEBIAN" "$dir/usr/bin" "$dir/usr/share/doc/$pkg" "$dir/usr/share/man/man1"
  make_control "$pkg" "$mode" "$pkgdesc"
  cp "$BIN_LINUX" "$dir/usr/bin/rjsc"
  if [ -f "$RELEASE_DIR/wrappers/rjsc-$mode" ]; then
    install -m755 "$RELEASE_DIR/wrappers/rjsc-$mode" "$dir/usr/bin/rjsc-$mode"
  fi
  cp "$RELEASE_DIR/rjsc.1" "$dir/usr/share/man/man1/" 2>/dev/null || true
  gzip -9 -n -f "$dir/usr/share/man/man1/rjsc.1" 2>/dev/null || true
  if [ -f "$RELEASE_DIR/configs/rjsc-$mode.toml" ]; then
    mkdir -p "$dir/etc/rj45-sound-card"
    cp "$RELEASE_DIR/configs/rjsc-$mode.toml" "$dir/etc/rj45-sound-card/rjsc.toml"
  fi
  dpkg-deb --build --root-owner-group "$dir" "$RELEASE_DIR/"
  small "deb: $(ls $RELEASE_DIR/${pkg}_${VERSION}_amd64.deb)"
}

make_rpm() {
  local pkg="$1" mode="$2"
  mkdir -p "$RELEASE_DIR/rpm/BUILD" "$RELEASE_DIR/rpm/RPMS" "$RELEASE_DIR/rpm/SOURCES" "$RELEASE_DIR/rpm/SPECS" "$RELEASE_DIR/rpm/rpmdb"
  cp "$BIN_LINUX" "$RELEASE_DIR/rpm/SOURCES/rjsc"
  if [ -f "$RELEASE_DIR/wrappers/rjsc-$mode" ]; then
    install -m755 "$RELEASE_DIR/wrappers/rjsc-$mode" "$RELEASE_DIR/rpm/SOURCES/rjsc-$mode"
  fi
  cp "$RELEASE_DIR/rjsc.1" "$RELEASE_DIR/rpm/SOURCES/"
  [ -f "$RELEASE_DIR/configs/rjsc-$mode.toml" ] && cp "$RELEASE_DIR/configs/rjsc-$mode.toml" "$RELEASE_DIR/rpm/SOURCES/rjsc.toml"

  cat > "$RELEASE_DIR/rpm/SPECS/$pkg.spec" << SPEOF
Name: $pkg
Version: $VERSION
Release: 1
Summary: RJ45 Sound Card - $mode package
License: MIT
URL: https://github.com/devfrp/rj45-sound-card
Group: Applications/Multimedia
BuildArch: x86_64

%description
Network audio bridge - share any sound card over Ethernet/RJ45.
This package contains the $mode binary and configuration.

%install
mkdir -p %{buildroot}%{_bindir}
mkdir -p %{buildroot}%{_mandir}/man1
install -m 755 %{_sourcedir}/rjsc %{buildroot}%{_bindir}/rjsc
install -m 755 %{_sourcedir}/rjsc-$mode %{buildroot}%{_bindir}/rjsc-$mode
gzip -9 -n -c %{_sourcedir}/rjsc.1 > %{buildroot}%{_mandir}/man1/rjsc.1.gz
%if 0%{?fedora} || 0%{?rhel}
%else
%endif

%files
%{_bindir}/rjsc
%{_bindir}/rjsc-$mode
%{_mandir}/man1/rjsc.1.gz

%changelog
* Tue Jun 16 2026 RJ45 Sound Card Team <devfrp@users.noreply.github.com> - $VERSION-1
- Initial release
SPEOF

  rpmbuild -bb --define "_topdir $RELEASE_DIR/rpm" --define "_dbpath $RELEASE_DIR/rpm/rpmdb" "$RELEASE_DIR/rpm/SPECS/$pkg.spec"
  cp "$RELEASE_DIR/rpm/RPMS/x86_64/$pkg-$VERSION-1.x86_64.rpm" "$RELEASE_DIR/"
  small "rpm: $(ls $RELEASE_DIR/${pkg}-${VERSION}-1.x86_64.rpm)"
}

make_arch() {
  local pkg="$1" mode="$2"
  local dir="$RELEASE_DIR/arch-$mode"
  mkdir -p "$dir/usr/bin" "$dir/usr/share/man/man1"
  cp "$BIN_LINUX" "$dir/usr/bin/rjsc"
  install -m755 "$RELEASE_DIR/wrappers/rjsc-$mode" "$dir/usr/bin/rjsc-$mode"
  gzip -9 -n -c "$RELEASE_DIR/rjsc.1" > "$dir/usr/share/man/man1/rjsc.1.gz"

  local size=$(stat -c%s "$BIN_LINUX" 2>/dev/null || echo 27000000)
  cat > "$dir/.PKGINFO" << PKGEOF
pkgname = $pkg
pkgver = $VERSION-1
pkgdesc = RJ45 Sound Card - $mode package
url = https://github.com/devfrp/rj45-sound-card
builddate = $(date +%Y-%m-%d)
packager = RJ45 Sound Card Team
size = $size
arch = x86_64
license = MIT
depend = alsa-lib
PKGEOF

  cd "$dir" && tar -c --zstd -f "$RELEASE_DIR/$pkg-$VERSION-1-x86_64.pkg.tar.zst" usr/ .PKGINFO
  cd "$TOP"
  small "arch: $(ls $RELEASE_DIR/${pkg}-${VERSION}-1-x86_64.pkg.tar.zst)"
}

make_windows_zip() {
  local pkg="$1" mode="$2"
  local dir="$RELEASE_DIR/win-$mode"
  mkdir -p "$dir"
  cp "$BIN_WIN" "$dir/rjsc.exe"
  cp "$RELEASE_DIR/wrappers/rjsc-$mode.bat" "$dir/"
  cp "$RELEASE_DIR/configs/rjsc-$mode.toml" "$dir/rjsc.toml" 2>/dev/null || true
  cd "$dir" && zip -q "$RELEASE_DIR/$pkg-$VERSION-x86_64-windows.zip" *
  cd "$TOP"
  small "win zip: $(ls $RELEASE_DIR/${pkg}-${VERSION}-x86_64-windows.zip)"
}

make_macos_app() {
  local pkg="$1" mode="$2"
  local dir="$RELEASE_DIR/macos-$mode"
  mkdir -p "$dir"
  cat > "$dir/install.sh" << 'ISHEOF'
#!/usr/bin/env bash
set -euo pipefail
echo "==> RJ45 Sound Card - macOS Installer"
echo ""
echo "Cette machine sera-t-elle le SERVEUR (carte son physique) ou le CLIENT (recoit l'audio) ?"
echo "1) Serveur"
echo "2) Client"
read -p "choix [1/2]: " choice
case "$choice" in
  1) MODE_FLAG="serve" ;;
  2) MODE_FLAG="connect" ;;
  *) echo "Invalide"; exit 1 ;;
esac
if command -v brew &>/dev/null; then
  echo "Installation via cargo..."
  brew install rust 2>/dev/null || true
  cargo install rj45-sound-card
else
  echo "Installez Rust depuis https://rustup.rs puis lancez:"
  echo "  cargo install rj45-sound-card"
  echo "  rjsc $MODE_FLAG"
fi
ISHEOF
  chmod +x "$dir/install.sh"
  cd "$dir" && zip -q "$RELEASE_DIR/$pkg-$VERSION-x86_64-macos.zip" *
  cd "$TOP"
  small "macos zip: $(ls $RELEASE_DIR/${pkg}-${VERSION}-x86_64-macos.zip)"
}

# ─── Prepare common files ─────────────────────────────────

log "Preparing wrappers and configs..."

mkdir -p "$RELEASE_DIR/wrappers" "$RELEASE_DIR/configs"

# Server wrapper (Linux)
cat > "$RELEASE_DIR/wrappers/rjsc-server" << 'WSEOF'
#!/usr/bin/env bash
exec rjsc serve "$@"
WSEOF
chmod +x "$RELEASE_DIR/wrappers/rjsc-server"

# Client wrapper (Linux)
cat > "$RELEASE_DIR/wrappers/rjsc-client" << 'WCEOF'
#!/usr/bin/env bash
exec rjsc connect "$@"
WCEOF
chmod +x "$RELEASE_DIR/wrappers/rjsc-client"

# Server wrapper (Windows)
cat > "$RELEASE_DIR/wrappers/rjsc-server.bat" << 'WSBEOF'
@echo off
rjsc serve %*
WSBEOF

# Client wrapper (Windows)
cat > "$RELEASE_DIR/wrappers/rjsc-client.bat" << 'WCBEOF'
@echo off
rjsc connect %*
WCBEOF

# Server default config
cat > "$RELEASE_DIR/configs/rjsc-server.toml" << 'CSEOF'
# Configuration SERVEUR - PC avec la carte son physique
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

[server]
auto_accept = true
max_clients = 1

[client]
use_virtual_device = false
auto_reconnect = false
CSEOF

# Client default config
cat > "$RELEASE_DIR/configs/rjsc-client.toml" << 'CCEOF'
# Configuration CLIENT - PC portable qui recoit l'audio
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
use_virtual_device = true
virtual_device_name = "hw:Loopback,0,0"
auto_reconnect = true
CCEOF

# Man page (copy from repo if available, otherwise generate inline)
if [ -f "$TOP/man/man1/rjsc.1" ]; then
  cp "$TOP/man/man1/rjsc.1" "$RELEASE_DIR/rjsc.1"
else
cat > "$RELEASE_DIR/rjsc.1" << 'MANEOF'
.TH RJSC 1 "June 2026" "rjsc 0.1.0" "User Commands"
.SH NAME
rjsc \- RJ45 Sound Card \- share any audio device over Ethernet
.SH SYNOPSIS
.B rjsc
[\fIOPTIONS\fR] \fICOMMAND\fR
.SH DESCRIPTION
Network audio bridge - share any sound card over Ethernet/RJ45.
.SH COMMANDS
.TP
.B serve
Share this PC's audio devices over the network
.TP
.B connect
Use audio devices from a remote server
.TP
.B list
List available audio devices
.TP
.B gui
Open the graphical control panel
.TP
.B init
Generate default configuration file
.SH FILES
rjsc.toml - Configuration file
.SH LICENSE
MIT
MANEOF
fi

# ─── Build all packages ───────────────────────────────────

log "Building server packages..."
make_deb  "rj45-sound-card-server"  "server"  "RJ45 Sound Card - SERVER (share your audio devices)"
make_rpm  "rj45-sound-card-server"  "server"
make_arch "rj45-sound-card-server"  "server"
make_windows_zip "rj45-sound-card-server" "server"
make_macos_app "rj45-sound-card-server" "server"

log "Building client packages..."
make_deb  "rj45-sound-card-client"  "client"  "RJ45 Sound Card - CLIENT (receive remote audio)"
make_rpm  "rj45-sound-card-client"  "client"
make_arch "rj45-sound-card-client"  "client"
make_windows_zip "rj45-sound-card-client" "client"
make_macos_app "rj45-sound-card-client" "client"

# ─── Also keep universal packages ─────────────────────────
log "Building universal packages..."
cp "$BIN_LINUX" "$RELEASE_DIR/rj45-sound-card-v${VERSION}-x86_64-linux"
cp "$BIN_WIN"   "$RELEASE_DIR/rj45-sound-card-v${VERSION}-x86_64-windows.exe"
make_deb "rj45-sound-card" "universal" "RJ45 Sound Card - share any audio device over Ethernet"

log ""
log "========================================"
log "Release $VERSION built!"
log "========================================"
log ""
log "Server packages:"
ls -lh "$RELEASE_DIR"/rj45-sound-card-server-* 2>/dev/null
log ""
log "Client packages:"
ls -lh "$RELEASE_DIR"/rj45-sound-card-client-* 2>/dev/null
log ""
log "Universal packages:"
ls -lh "$RELEASE_DIR"/rj45-sound-card-* 2>/dev/null | grep -v server | grep -v client
