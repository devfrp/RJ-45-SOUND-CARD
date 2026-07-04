use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const DEFAULT_CONTROL_PORT: u16 = 42002;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlMessage {
    #[serde(rename = "auth")]
    Auth { challenge: String },

    #[serde(rename = "auth_response")]
    AuthResponse { hash: String },

    #[serde(rename = "auth_ok")]
    AuthOk,

    #[serde(rename = "auth_required")]
    AuthRequired,

    #[serde(rename = "list_devices")]
    ListDevices,

    #[serde(rename = "device_list")]
    DeviceList { devices: Vec<DeviceInfo> },

    #[serde(rename = "select_device")]
    SelectDevice {
        device_id: usize,
        channels: u16,
        sample_rate: u32,
    },

    #[serde(rename = "device_selected")]
    DeviceSelected {
        device_id: usize,
        channels: u16,
        sample_rate: u32,
        stream_id: u32,
    },

    #[serde(rename = "start_stream")]
    StartStream { direction: String },

    #[serde(rename = "stop_stream")]
    StopStream,

    #[serde(rename = "status")]
    Status {
        running: bool,
        uptime_secs: u64,
        clients: u32,
        stream_id: Option<u32>,
        audio_rx_kbps: f64,
        audio_tx_kbps: f64,
    },

    #[serde(rename = "set_volume")]
    SetVolume { channel: u16, volume: f32 },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub index: usize,
    pub name: String,
    pub input_channels: u16,
    pub output_channels: u16,
    pub sample_rates: Vec<u32>,
}

pub async fn send_control(
    stream: &mut (impl AsyncWrite + Unpin),
    msg: &ControlMessage,
) -> Result<()> {
    let data = serde_json::to_vec(msg)?;
    let len = (data.len() as u32).to_le_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn recv_control(stream: &mut (impl AsyncRead + Unpin)) -> Result<ControlMessage> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 65536 {
        anyhow::bail!("Control message too large: {} bytes", len);
    }
    let mut data = vec![0u8; len];
    stream.read_exact(&mut data).await?;
    let msg: ControlMessage = serde_json::from_slice(&data)?;
    Ok(msg)
}

pub async fn run_control_server(addr: &str) -> Result<(tokio::net::TcpListener, u16)> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let port = listener.local_addr()?.port();
    log::info!("Control server listening on {}", addr);
    Ok((listener, port))
}

pub fn compute_auth_response(challenge: &str, pre_shared_key_hex: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = siphasher::sip::SipHasher::new_with_keys(0, 0);
    challenge.hash(&mut hasher);
    pre_shared_key_hex.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
