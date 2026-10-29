# RJ45 Sound Card

*Lire en : **Français** · [English](README.md)*

🌐 **Site :** <https://devfrp.github.io/RJ-45-SOUND-CARD/>

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
git clone https://github.com/devfrp/RJ-45-SOUND-CARD.git
cd RJ-45-SOUND-CARD
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

## Démarrage rapide (sans commande)

1. Copiez `rjsc` (`rjsc.exe` sous Windows) sur les deux PC, à un endroit où il restera.
2. Double-cliquez dessus. Une page s'ouvre dans le navigateur et demande ce qu'est ce PC :
   - **Le PC avec les cartes son** (serveur) : il démarre désormais avec l'ordinateur et relaie ses
     cartes son. Réglez-le depuis n'importe quel navigateur sur `http://<ip-du-serveur>:42080`.
   - **Le PC avec le DAW** (client) : il démarre avec l'ordinateur, cherche le serveur et se connecte
     dès que le câble RJ45 est branché. Sa page (`http://localhost:42080`) affiche la connexion et
     installe une fois la carte son virtuelle (l'ordinateur demande l'autorisation administrateur ;
     il faut internet).
3. Un nouveau double-clic sur `rjsc` ouvre simplement la page.

Les deux PC ont besoin d'une adresse IPv4 sur le câble. Windows et macOS s'en attribuent une tout
seuls (169.254.x.x) au bout d'une minute environ ; sous Linux, réglez la connexion filaire sur
« Lien local uniquement » (ou « Partagée avec d'autres ordinateurs ») dans les paramètres réseau.

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

Le serveur fournit aussi un **panneau de contrôle web** : ouvrez `http://<ip-du-serveur>:42080` dans
n'importe quel navigateur (celui du portable, par exemple). Il affiche l'état et les clients connectés,
permet de choisir les périphériques audio, le nombre de canaux, la fréquence et la taille du buffer, de
régler le volume de chaque canal et de lire le journal. Les modifications sont enregistrées dans
`rjsc.toml` et appliquées en redémarrant le serveur audio. Si la carte son est absente au démarrage,
le panneau reste accessible et le serveur réessaie toutes les 10 secondes.

Le panneau n'a pas de mot de passe : toute machine qui joint le serveur sur le réseau peut modifier ses
réglages. Utilisez-le sur un câble direct ou un réseau de confiance, ou désactivez-le avec
`[web] enabled = false`.

#### Démarrer le serveur automatiquement

Le choix du rôle sur la page du double-clic le fait pour vous. Depuis un terminal :

```bash
rjsc autostart enable     # démarre maintenant et à chaque démarrage (en serveur si aucun rôle n'est choisi)
rjsc autostart status
rjsc autostart disable
```

| OS | Mécanisme | Démarre |
|----|-----------|---------|
| Linux | service utilisateur systemd | au boot (ou à l'ouverture de session si le « lingering » ne peut pas être activé) |
| macOS | agent launchd | à l'ouverture de session |
| Windows | tâche planifiée « RJ45 Sound Card » | à l'ouverture de session |

Le serveur tourne dans la session utilisateur, car c'est là que les périphériques audio sont disponibles.
Sous macOS et Windows, activez l'ouverture de session automatique pour qu'il démarre sans intervention.
Le fichier de configuration est `~/.config/rjsc/rjsc.toml` (Linux),
`~/Library/Application Support/rjsc/rjsc.toml` (macOS) ou `%APPDATA%\rjsc\rjsc.toml` (Windows), créé
s'il n'existe pas ; utilisez `-c` pour en choisir un autre. Le binaire `rjsc` doit rester à
l'emplacement où il était lors de la commande.

### 4. Client (PC portable)

```bash
# Connexion automatique (découverte réseau)
rjsc connect

# Ou avec adresse spécifique
rjsc connect --server 192.168.1.100:42002
```

## Latence

Latence dans un sens ≈ buffer de capture + marge réseau + buffer de lecture :

- **Taille du buffer** (`audio.buffer_frames`) : 64 frames → ~1.3ms, 256 → ~5.3ms, 1024 → ~21ms (à 48kHz).
  Elle est appliquée à la carte son ; si la carte ou son pilote la refuse (ou échoue avec), sa taille par défaut est utilisée et le journal l'indique.
- **Marge réseau** (`audio.latency_ms`, 10 ms par défaut) : absorbe la gigue du réseau et la dérive entre
  les horloges des deux cartes son. Elle est augmentée automatiquement si la taille des paquets l'exige. Augmentez-la si vous entendez des clics.
- **Pilotes audio** : ASIO (Windows) / JACK (Linux) offrent la plus faible latence.

Deux cartes son ne tournent jamais exactement à la même vitesse. Chaque côté mesure le remplissage de son
tampon et ré-échantillonne le son reçu de quelques ppm pour compenser : pas de clics réguliers ni de latence
qui augmente lentement. Toutes les 30 s, le journal indique le niveau du tampon, la dérive d'horloge mesurée et
les éventuelles coupures.

Pour du monitoring en temps réel, utilisez 64 ou 128 frames sur un câble direct ou un réseau Gigabit.

## Dépannage

**Le client ne trouve pas le serveur :**
- Vérifiez que les deux PC sont sur le même réseau
- Vérifiez le pare-feu (ouvrir les ports UDP 42000-42001, TCP 42002 et 42080)
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
