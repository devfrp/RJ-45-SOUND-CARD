use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::audio::format::{AudioFormat, SampleFormat};
use crate::net::crypto::PacketCrypto;

/// What this PC does, chosen once on the setup page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Has the sound card and relays it.
    Server,
    /// The laptop that uses the remote sound card.
    Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Unset until chosen; used by `rjsc start` (double-click and autostart).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,

    #[serde(default)]
    pub audio: AudioSettings,

    #[serde(default)]
    pub network: NetworkSettings,

    #[serde(default)]
    pub server: ServerSettings,

    #[serde(default)]
    pub client: ClientSettings,

    #[serde(default)]
    pub encryption: EncryptionSettings,

    #[serde(default)]
    pub web: WebSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    #[serde(default = "default_input_device")]
    pub input_device: String,

    #[serde(default = "default_output_device")]
    pub output_device: String,

    #[serde(default = "default_channels")]
    pub channels: u16,

    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    #[serde(default = "default_buffer_frames")]
    pub buffer_frames: usize,

    #[serde(default)]
    pub sample_format: SampleFormat,

    /// Target of the buffer that absorbs network jitter and clock drift for
    /// incoming audio (raised automatically if the packet sizes need more).
    #[serde(default = "default_latency_ms")]
    pub latency_ms: u32,
}

fn default_latency_ms() -> u32 {
    10
}

fn default_input_device() -> String {
    "@default".to_string()
}
fn default_output_device() -> String {
    "@default".to_string()
}
fn default_channels() -> u16 {
    2
}
fn default_sample_rate() -> u32 {
    48000
}
fn default_buffer_frames() -> usize {
    256
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSettings {
    #[serde(default = "default_audio_port")]
    pub audio_port: u16,

    #[serde(default = "default_control_port")]
    pub control_port: u16,

    #[serde(default = "default_discovery_port")]
    pub discovery_port: u16,

    #[serde(default = "default_bind_address")]
    pub bind_address: String,
}

fn default_audio_port() -> u16 {
    crate::net::audio_stream::DEFAULT_AUDIO_PORT
}
fn default_control_port() -> u16 {
    crate::net::control::DEFAULT_CONTROL_PORT
}
fn default_discovery_port() -> u16 {
    crate::net::discovery::DISCOVERY_PORT
}
fn default_bind_address() -> String {
    "0.0.0.0".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSettings {
    #[serde(default = "default_true")]
    pub auto_accept: bool,

    #[serde(default = "default_max_clients")]
    pub max_clients: u32,
}

fn default_true() -> bool {
    true
}
fn default_max_clients() -> u32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientSettings {
    #[serde(default)]
    pub server_address: String,

    #[serde(default = "default_true")]
    pub use_virtual_device: bool,

    #[serde(default)]
    pub virtual_device_name: String,

    #[serde(default = "default_true")]
    pub auto_reconnect: bool,

    #[serde(default = "default_jitter_ms")]
    pub jitter_buffer_ms: u64,
}

fn default_jitter_ms() -> u64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_web_port")]
    pub port: u16,
}

fn default_web_port() -> u16 {
    42080
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionSettings {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub pre_shared_key: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            role: None,
            audio: AudioSettings {
                input_device: default_input_device(),
                output_device: default_output_device(),
                channels: default_channels(),
                sample_rate: default_sample_rate(),
                buffer_frames: default_buffer_frames(),
                sample_format: SampleFormat::default(),
                latency_ms: default_latency_ms(),
            },
            network: NetworkSettings {
                audio_port: default_audio_port(),
                control_port: default_control_port(),
                discovery_port: default_discovery_port(),
                bind_address: default_bind_address(),
            },
            server: ServerSettings {
                auto_accept: default_true(),
                max_clients: default_max_clients(),
            },
            client: ClientSettings {
                server_address: String::new(),
                use_virtual_device: default_true(),
                virtual_device_name: "RJ45 Virtual Audio".to_string(),
                auto_reconnect: default_true(),
                jitter_buffer_ms: default_jitter_ms(),
            },
            encryption: EncryptionSettings {
                enabled: false,
                pre_shared_key: String::new(),
            },
            web: WebSettings {
                enabled: default_true(),
                port: default_web_port(),
            },
        }
    }
}

impl Default for AudioSettings {
    fn default() -> Self {
        Settings::default().audio
    }
}

impl Default for NetworkSettings {
    fn default() -> Self {
        Settings::default().network
    }
}

impl Default for ServerSettings {
    fn default() -> Self {
        Settings::default().server
    }
}

impl Default for ClientSettings {
    fn default() -> Self {
        Settings::default().client
    }
}

impl Default for EncryptionSettings {
    fn default() -> Self {
        Settings::default().encryption
    }
}

impl Default for WebSettings {
    fn default() -> Self {
        Settings::default().web
    }
}

impl Settings {
    pub fn audio_format(&self) -> AudioFormat {
        AudioFormat::with_format(
            self.audio.channels,
            self.audio.sample_rate,
            self.audio.buffer_frames,
            self.audio.sample_format,
        )
    }

    pub fn encryption_key(&self) -> Option<PacketCrypto> {
        if self.encryption.enabled && !self.encryption.pre_shared_key.is_empty() {
            PacketCrypto::from_hex(&self.encryption.pre_shared_key).ok()
        } else {
            None
        }
    }
}

pub fn load(path: &str) -> Result<Settings> {
    if !Path::new(path).exists() {
        log::info!("Config file '{}' not found, using defaults", path);
        return Ok(Settings::default());
    }
    let content = std::fs::read_to_string(path)?;
    let settings: Settings = toml::from_str(&content)?;
    log::info!("Loaded configuration from '{}'", path);
    Ok(settings)
}

pub fn save(path: &str, settings: &Settings) -> Result<()> {
    let content = toml::to_string_pretty(settings)?;
    std::fs::write(path, content)?;
    log::info!("Saved configuration to '{}'", path);
    Ok(())
}

pub fn save_default(path: &str) -> Result<()> {
    save(path, &Settings::default())
}
