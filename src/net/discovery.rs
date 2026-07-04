use anyhow::Result;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::time::{interval, Duration};

pub const DISCOVERY_PORT: u16 = 42000;
pub const DISCOVERY_INTERVAL_SECS: u64 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryMessage {
    pub hostname: String,
    pub device_name: String,
    pub device_channels: u16,
    pub audio_port: u16,
    pub control_port: u16,
    pub protocol_version: String,
}

pub async fn announce_server(
    hostname: &str,
    device_name: &str,
    device_channels: u16,
    audio_port: u16,
    control_port: u16,
) -> Result<()> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.set_broadcast(true)?;

    let msg = DiscoveryMessage {
        hostname: hostname.to_string(),
        device_name: device_name.to_string(),
        device_channels,
        audio_port,
        control_port,
        protocol_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let payload = serde_json::to_vec(&msg)?;

    let mut ticker = interval(Duration::from_secs(DISCOVERY_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        socket
            .send_to(&payload, &format!("255.255.255.255:{}", DISCOVERY_PORT))
            .await?;
        log::debug!("Broadcast discovery announcement");
    }
}

pub async fn discover_servers(
    timeout_secs: u64,
) -> Result<Vec<(std::net::SocketAddr, DiscoveryMessage)>> {
    let std_socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    std_socket.set_reuse_address(true)?;
    std_socket.set_broadcast(true)?;
    std_socket.bind(
        &format!("0.0.0.0:{}", DISCOVERY_PORT)
            .parse::<std::net::SocketAddr>()?
            .into(),
    )?;
    std_socket.set_nonblocking(true)?;
    let socket = UdpSocket::from_std(std_socket.into())?;

    let mut servers = Vec::new();
    let mut buf = vec![0u8; 4096];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        tokio::select! {
            result = tokio::time::timeout(remaining, socket.recv_from(&mut buf)) => {
                match result {
                    Ok(Ok((len, addr))) => {
                        if let Ok(msg) = serde_json::from_slice::<DiscoveryMessage>(&buf[..len]) {
                            if !servers.iter().any(|(a, _): &(std::net::SocketAddr, DiscoveryMessage)| *a == addr) {
                                log::info!("Discovered server: {} at {} (device: {})", msg.hostname, addr, msg.device_name);
                                servers.push((addr, msg));
                            }
                        }
                    }
                    Ok(Err(e)) => log::debug!("Discovery recv error: {}", e),
                    Err(_) => {}
                }
            }
        }
    }

    Ok(servers)
}
