use anyhow::Result;
use cpal::traits::DeviceTrait;
use crossbeam::channel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::BufReader;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

use crate::audio::capture::AudioCapture;
use crate::audio::format::AudioFormat;
use crate::audio::jitter::JitterBuffer;
use crate::audio::playback::AudioPlayback;
use crate::config::Settings;
use crate::net::audio_stream::{self, AudioPacketHeader};
use crate::net::control::{self, ControlMessage};
use crate::net::crypto::PacketCrypto;
use crate::net::discovery;

pub async fn run(
    settings: Settings,
    server_addr: Option<String>,
    stop_flag: Arc<AtomicBool>,
) -> Result<()> {
    log::info!("RJ45 Sound Card - Client Mode");

    let (server_addr, discovered_audio_port) = if let Some(addr) = server_addr {
        (addr, None)
    } else if !settings.client.server_address.is_empty() {
        (settings.client.server_address.clone(), None)
    } else {
        log::info!("Discovering servers on the network...");
        let servers = discovery::discover_servers(5).await?;
        if servers.is_empty() {
            anyhow::bail!("No servers found. Specify --server or configure server_address.");
        }
        let (addr, msg) = &servers[0];
        log::info!(
            "Connecting to {} at {} (device: {}, audio port {})",
            msg.hostname,
            addr,
            msg.device_name,
            msg.audio_port
        );
        (
            format!("{}:{}", addr.ip(), msg.control_port),
            Some(msg.audio_port),
        )
    };

    let server_ip = server_addr
        .rsplitn(2, ':')
        .last()
        .unwrap_or("127.0.0.1")
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();

    loop {
        match run_session(
            &settings,
            &server_addr,
            &server_ip,
            discovered_audio_port,
            &stop_flag,
        )
        .await
        {
            Ok(()) => {
                if settings.client.auto_reconnect {
                    log::info!("Session ended, reconnecting in 5 seconds...");
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
                return Ok(());
            }
            Err(e) => {
                if settings.client.auto_reconnect {
                    log::error!("Session error: {}. Reconnecting in 5 seconds...", e);
                    sleep(Duration::from_secs(5)).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
}

async fn run_session(
    settings: &Settings,
    server_addr: &str,
    server_ip: &str,
    discovered_audio_port: Option<u16>,
    stop_flag: &AtomicBool,
) -> Result<()> {
    log::info!("Connecting to server at {}...", server_addr);
    let tcp_stream = TcpStream::connect(server_addr).await?;
    let (mut read_half, mut write_half) = tokio::io::split(tcp_stream);
    let mut reader = BufReader::new(&mut read_half);
    log::info!("Connected to server");

    let crypto = settings.encryption_key();

    let first_msg = control::recv_control(&mut reader).await?;
    match first_msg {
        ControlMessage::Auth { challenge } => {
            let psk = &settings.encryption.pre_shared_key;
            if psk.is_empty() {
                anyhow::bail!("Server requires authentication but no pre_shared_key configured");
            }
            let hash = control::compute_auth_response(&challenge, psk);
            control::send_control(&mut write_half, &ControlMessage::AuthResponse { hash }).await?;

            match control::recv_control(&mut reader).await? {
                ControlMessage::AuthOk => {
                    log::info!("Authenticated successfully");
                }
                ControlMessage::Error { message } => {
                    anyhow::bail!("Authentication failed: {}", message);
                }
                _ => {
                    anyhow::bail!("Unexpected auth response");
                }
            }

            control::send_control(&mut write_half, &ControlMessage::ListDevices).await?;
        }
        ControlMessage::DeviceList { .. } | ControlMessage::Error { .. } => {}
        _ => {
            control::send_control(&mut write_half, &ControlMessage::ListDevices).await?;
            return read_msg_loop(
                settings,
                &mut reader,
                &mut write_half,
                server_ip,
                discovered_audio_port,
                stop_flag,
                crypto,
            )
            .await;
        }
    }

    log::info!("Requested device list...");
    read_msg_loop(
        settings,
        &mut reader,
        &mut write_half,
        server_ip,
        discovered_audio_port,
        stop_flag,
        crypto,
    )
    .await
}

async fn read_msg_loop(
    settings: &Settings,
    reader: &mut BufReader<&mut tokio::io::ReadHalf<TcpStream>>,
    write_half: &mut tokio::io::WriteHalf<TcpStream>,
    server_ip: &str,
    discovered_audio_port: Option<u16>,
    stop_flag: &AtomicBool,
    crypto: Option<PacketCrypto>,
) -> Result<()> {
    let device_list = loop {
        match control::recv_control(reader).await? {
            ControlMessage::DeviceList { devices } => {
                log::info!("Available devices on server:");
                for (i, d) in devices.iter().enumerate() {
                    log::info!(
                        "  {}. {} ({} in / {} out, rates: {:?})",
                        i,
                        d.name,
                        d.input_channels,
                        d.output_channels,
                        d.sample_rates
                    );
                }
                break devices;
            }
            other => {
                log::warn!("Unexpected message: {:?}", other);
            }
        }
    };

    if device_list.is_empty() {
        anyhow::bail!("No audio devices available on server");
    }

    let device = &device_list[0];
    let channels = settings.audio.channels.min(device.input_channels);
    let sample_rate = settings.audio.sample_rate;

    control::send_control(
        write_half,
        &ControlMessage::SelectDevice {
            device_id: device.index,
            channels,
            sample_rate,
        },
    )
    .await?;

    loop {
        match control::recv_control(reader).await? {
            ControlMessage::DeviceSelected { stream_id, .. } => {
                log::info!("Device confirmed (stream #{})", stream_id);
                break;
            }
            ControlMessage::Error { message } => {
                anyhow::bail!("Server error: {}", message);
            }
            _ => {}
        }
    }

    let audio_format = AudioFormat::with_format(
        channels,
        sample_rate,
        settings.audio.buffer_frames,
        settings.audio.sample_format,
    );

    let output_device =
        if settings.client.use_virtual_device && !settings.client.virtual_device_name.is_empty() {
            match crate::audio::find_device(&settings.client.virtual_device_name) {
                Ok(d) => {
                    log::info!("Using virtual audio device: {}", d.name()?);
                    d
                }
                Err(_) => {
                    log::warn!(
                        "Virtual device '{}' not found, trying auto-detect...",
                        settings.client.virtual_device_name
                    );
                    if let Some(name) = crate::audio::virtual_device::detect_virtual_device() {
                        match crate::audio::find_device(&name) {
                            Ok(d) => {
                                log::info!("Auto-detected virtual device: {}", d.name()?);
                                d
                            }
                            Err(_) => {
                                log::warn!("Auto-detected device not available, using default");
                                crate::audio::default_output_device()?
                            }
                        }
                    } else {
                        log::warn!("No virtual device found, using default output");
                        crate::audio::default_output_device()?
                    }
                }
            }
        } else {
            crate::audio::default_output_device()?
        };

    let playback = AudioPlayback::new(&output_device, &audio_format)?;
    playback.start()?;
    let playback_tx = playback.sender();

    let input_device = crate::audio::default_input_device()?;
    let capture = AudioCapture::new(&input_device, &audio_format)?;
    capture.start()?;
    let capture_rx = capture.receiver();

    control::send_control(
        write_half,
        &ControlMessage::StartStream {
            direction: "both".to_string(),
        },
    )
    .await?;

    let audio_port = discovered_audio_port.unwrap_or(settings.network.audio_port);
    let server_audio_addr: std::net::SocketAddr =
        format!("{}:{}", server_ip, audio_port).parse()?;

    // Jitter-buffered receive path
    let jitter_buffer = Arc::new(Mutex::new(JitterBuffer::new(
        settings.client.jitter_buffer_ms,
        sample_rate,
    )));
    let rx_sock = UdpSocket::bind(format!("0.0.0.0:{}", settings.network.audio_port)).await?;
    let rx_tx = playback_tx.clone();
    let jb_clone = jitter_buffer.clone();
    let crypto_rx = crypto.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; audio_stream::MAX_PACKET_SIZE];
        loop {
            match rx_sock.recv_from(&mut buf).await {
                Ok((len, _addr)) => {
                    if let Some((header, samples)) =
                        audio_stream::parse_audio_packet_decrypt(&buf[..len], crypto_rx.as_ref())
                    {
                        let mut jb = jb_clone.lock().await;
                        jb.push(header.sequence, samples);
                        for packet in jb.drain_available() {
                            if let Err(e) = rx_tx.try_send(packet) {
                                if !e.is_disconnected() {
                                    log::warn!("Playback buffer full");
                                } else {
                                    log::error!("Playback channel disconnected");
                                    return;
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::error!("Audio receive error: {}", e);
                    break;
                }
            }
        }
    });

    // Send path
    let tx_sock = UdpSocket::bind("0.0.0.0:0").await?;
    let ch = audio_format.channels;
    let sr = audio_format.sample_rate;
    let bf = audio_format.buffer_frames as u32;
    let fmt = audio_format.sample_format;
    let crypto_tx = crypto.clone();
    tokio::spawn(async move {
        let mut sequence: u64 = 0;
        loop {
            match capture_rx.recv() {
                Ok(samples) => {
                    let header = AudioPacketHeader::new(0, sequence, ch, sr, fmt, bf);
                    sequence = sequence.wrapping_add(1);
                    if let Err(e) = audio_stream::send_audio_packet(
                        &tx_sock,
                        &server_audio_addr,
                        &header,
                        &samples,
                        crypto_tx.as_ref(),
                    )
                    .await
                    {
                        log::error!("Failed to send capture: {}", e);
                        break;
                    }
                }
                Err(channel::RecvError) => {
                    log::error!("Capture channel closed");
                    break;
                }
            }
        }
    });

    // Keep-alive ping loop
    loop {
        if stop_flag.load(Ordering::SeqCst) {
            log::info!("Session stopped cleanly");
            return Ok(());
        }
        sleep(Duration::from_secs(10)).await;
        if let Err(e) = control::send_control(write_half, &ControlMessage::Ping).await {
            log::info!("Keep-alive lost: {}", e);
            break;
        }
    }

    Ok(())
}
