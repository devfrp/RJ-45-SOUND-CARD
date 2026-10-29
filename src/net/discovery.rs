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
    /// Port of the web control panel, 0 when disabled (absent before 1.1.0).
    #[serde(default)]
    pub web_port: u16,
}

pub async fn announce_server(
    hostname: &str,
    device_name: &str,
    device_channels: u16,
    audio_port: u16,
    control_port: u16,
    web_port: u16,
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
        web_port,
    };
    let payload = serde_json::to_vec(&msg)?;

    let mut ticker = interval(Duration::from_secs(DISCOVERY_INTERVAL_SECS));
    loop {
        ticker.tick().await;
        // A failed send (e.g. cable unplugged, no route) must not stop later announcements.
        for target in broadcast_targets() {
            if let Err(e) = socket.send_to(&payload, target).await {
                log::debug!("Discovery announcement to {} failed: {}", target, e);
            }
        }
        log::debug!("Broadcast discovery announcement");
    }
}

/// Where to send announcements, listed again each time so a cable plugged in
/// later is used. 255.255.255.255 alone only leaves through the default route,
/// so a direct cable next to Wi-Fi (or a PC with no default route) needs each
/// interface's own broadcast address. Loopback makes a client on the same PC work.
fn broadcast_targets() -> Vec<std::net::SocketAddr> {
    let mut targets = vec![std::net::SocketAddr::from((
        std::net::Ipv4Addr::BROADCAST,
        DISCOVERY_PORT,
    ))];
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        if let if_addrs::IfAddr::V4(v4) = iface.addr {
            let ip = if v4.ip.is_loopback() {
                Some(v4.ip)
            } else {
                v4.broadcast
            };
            if let Some(ip) = ip {
                let target = std::net::SocketAddr::from((ip, DISCOVERY_PORT));
                if !targets.contains(&target) {
                    targets.push(target);
                }
            }
        }
    }
    targets
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
