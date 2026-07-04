> [🇬🇧 English version](README.md)

# RJ45 Sound Card

Partagez n'importe quelle carte son entre deux PC via Ethernet (RJ45).

Le **PC serveur** (avec la carte son physique, ex: MOTU, RME, Focusrite…) capture l'audio et le streame
via le réseau. Le **PC client** (portable) reçoit le flux et le diffuse sur un périphérique audio virtuel,
rendant la carte son distante accessible comme si elle était locale.

## Architecture

```
┌─────────────────────────┐       RJ45 (Ethernet)        ┌──────────────────────────┐
│  SERVEUR (PC Studio)    │◄──────────────────────────►│  CLIENT (PC Portable)     │
│                         │  UDP: flux audio (port 42001)│                          │
│  Carte son physique     │  TCP: contrôle   (port 42002)│  Virtual Audio Device     │
│  (MOTU, RME, …)         │  UDP: découverte (port 42000)│  (snd-aloop/BlackHole/   │
│                         │                               │   VB-Cable)               │
│  Capture → UDP Send     │                               │  UDP Receive → Playback   │
│  UDP Receive → Playback │                               │  Capture → UDP Send       │
└─────────────────────────┘                               └──────────────────────────┘
```

## Fonctionnalités

- **Multi-plateforme** : Windows, Linux, macOS
- **Toutes cartes son** : compatible avec tout périphérique audio reconnu par l'OS
  (MOTU, RME, Focusrite, Universal Audio, Presonus, etc.)
- **Bidirectionnel** : audio du serveur → client ET audio du client → serveur
- **Jitter buffer** : réordonnancement et compensation de gigue pour une lecture fluide
- **Chiffrement** : chiffrement par clé pré-partagée et authentification (optionnel)
- **PCM entier** : formats f32, i16, i24, i32 avec conversion automatique
- **Auto-détection** : détection automatique du périphérique virtuel (Loopback, BlackHole, VB-Cable)
- **Multi-client** : le serveur accepte plusieurs connexions simultanées
- **Mode daemon** : exécution en arrière-plan avec `--daemon` ou via systemd
- **Faible latence** : streaming UDP avec buffers configurables (64–1024 frames)
- **Auto-découverte** : le client trouve automatiquement les serveurs sur le réseau
- **Contrôle à distance** : volume, sélection de périphérique, statut
- **Multi-canal** : support de 1 à 64+ canaux selon la configuration

## Installation

### Prérequis

- **Rust** (pour compiler) : `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- **PortAudio** (bibliothèque audio) :
  - Linux : `sudo apt install libasound2-dev` (ou `pulseaudio`, `jack`)
  - macOS : déjà inclus avec CoreAudio
  - Windows : déjà inclus avec WASAPI

### Compilation

```bash
git clone https://github.com/devfrp/rj45-sound-card.git
cd rj45-sound-card
cargo build --release
./target/release/rjsc --help
```

Le binaire `rjsc` est autonome et peut être copié sur n'importe quelle machine du même OS.

### Configuration du périphérique audio virtuel (CLIENT)

Pour que le PC client voie la carte son distante comme un périphérique local :

**Linux** :
```bash
sudo ./scripts/linux_setup.sh install
```

**macOS** :
```bash
sudo ./scripts/mac_setup.sh install
```

**Windows** (PowerShell Administrateur) :
```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows_setup.ps1 install
```

## Utilisation

### 1. Lister les périphériques audio disponibles

```bash
# Sur le serveur (PC avec la carte son)
rjsc list

# Sur le client (PC portable)
rjsc list
```

### 2. Configurer

Créez un fichier de configuration et éditez-le :

```bash
rjsc init
```

Exemple de configuration (`rjsc.toml`) :

```toml
[audio]
input_device = "MOTU 424"      # Exemple : nom de votre périphérique
output_device = "MOTU 424"     # Exemple : nom de votre périphérique
channels = 8                   # Nombre de canaux à partager
sample_rate = 48000            # Fréquence d'échantillonnage
buffer_frames = 256            # Taille du buffer (latence)

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

### 3. Serveur (PC avec la carte son physique)

```bash
# Sur la machine de studio (avec la carte son physique)
rjsc serve
```

### 4. Client (PC portable)

```bash
# Connexion automatique (découverte réseau)
rjsc connect

# Ou avec adresse spécifique
rjsc connect --server 192.168.1.100:42002
```

## Latence

La latence dépend de :

- **Taille du buffer** : 64 frames → ~1.3ms, 256 → ~5.3ms, 1024 → ~21ms (à 48kHz)
- **Réseau** : latence du switch/routeur Ethernet
- **Pilotes audio** : ASIO (Windows) / JACK (Linux) offrent la plus faible latence

Pour une utilisation en monitoring temps réel, utilisez 64 ou 128 frames avec un réseau Gigabit.

## Dépannage

**Le client ne trouve pas le serveur :**
- Vérifiez que les deux PC sont sur le même réseau
- Vérifiez le pare-feu (ouvrir les ports UDP 42000-42001, TCP 42002)
- Utilisez `--server` pour spécifier l'adresse manuellement

**Pas de son sur le client :**
- Vérifiez le périphérique audio virtuel avec `rjsc list`
- Vérifiez les paramètres de son du système (sélectionnez le périphérique virtuel)
- Augmentez la taille du buffer

**Latence trop élevée :**
- Réduisez `buffer_frames` (64 ou 128)
- Utilisez un réseau Gigabit (pas WiFi)
- Activez JACK (Linux) ou ASIO (Windows) pour les pilotes à faible latence

## Licence

MIT

## Documentation

- [Référence de configuration](docs/config.md) — toutes les options du fichier `rjsc.toml`
- [Architecture](ARCHITECTURE.md) — design interne et protocoles
- [Contribuer](CONTRIBUTING.md) — guide de contribution
- [Journal des modifications](CHANGELOG.md) — historique des versions
- [Page de man](man/man1/rjsc.1) — `man rjsc`
- [English README](README.en.md)
