use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Result;
use cpal::traits::DeviceTrait;
use crossbeam::channel;
use tokio::io::BufReader;
use tokio::sync::RwLock;

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::config::Settings;
use crate::net::audio_stream::{self, AudioPacketHeader};
use crate::net::control::{self, ControlMessage, DeviceInfo};
use crate::net::discovery;

struct ClientStream {
    stream_id: u32,
    sequence: u64,
}

pub async fn run(settings: Settings, stop_flag: Arc<AtomicBool>) -> Result<()> {
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string());

    log::info!("RJ45 Sound Card - Server Mode");
    log::info!("Hostname: {}", hostname);

    let input_device = if settings.audio.input_device == "@default" {
        crate::audio::default_input_device()?
    } else {
        crate::audio::find_device(&settings.audio.input_device)?
    };
    let output_device = if settings.audio.output_device == "@default" {
        crate::audio::default_output_device()?
    } else {
        crate::audio::find_device(&settings.audio.output_device)?
    };

    log::info!("Input device: {}", input_device.name()?);
    log::info!("Output device: {}", output_device.name()?);

    let audio_format = settings.audio_format();
    log::info!(
        "Audio format: {} channels @ {} Hz, buffer: {} frames, {:?}",
        audio_format.channels,
        audio_format.sample_rate,
        audio_format.buffer_frames,
        audio_format.sample_format,
    );
    log::info!(
        "Estimated bandwidth: {:.1} Mbps",
        audio_format.bitrate_mbps()
    );

    let crypto = settings.encryption_key();
    if crypto.is_some() {
        log::info!("Encryption enabled (pre-shared key)");
    }

    let capture = AudioCapture::new(&input_device, &audio_format)?;
    capture.start()?;
    let capture_rx = capture.receiver();
    let actual_format = *capture.format();

    let playback = AudioPlayback::new(&output_device, &actual_format)?;
    playback.start()?;
    let playback_tx = playback.sender();

    let audio_socket = Arc::new(
        tokio::net::UdpSocket::bind(format!(
            "{}:{}",
            settings.network.bind_address, settings.network.audio_port
        ))
        .await?,
    );
    let audio_port = audio_socket.local_addr()?.port();

    let (control_listener, control_port) = control::run_control_server(&format!(
        "{}:{}",
        settings.network.bind_address, settings.network.control_port
    ))
    .await?;

    let device_name = input_device
        .name()
        .unwrap_or_else(|_| "Unknown".to_string());
    let dn = device_name.clone();
    let hn = hostname.clone();
    let ch = audio_format.channels;
    tokio::spawn(async move {
        if let Err(e) = discovery::announce_server(&hn, &dn, ch, audio_port, control_port).await {
            log::error!("Discovery broadcast failed: {}", e);
        }
    });

    let connected_clients = Arc::new(AtomicU32::new(0));
    let next_stream_id = Arc::new(AtomicU32::new(1));
    let current_client_addrs: Arc<RwLock<Vec<SocketAddr>>> = Arc::new(RwLock::new(Vec::new()));
    let client_streams: Arc<RwLock<HashMap<SocketAddr, ClientStream>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let volumes: Arc<RwLock<Vec<f32>>> =
        Arc::new(RwLock::new(vec![1.0; audio_format.channels as usize]));

    let audio_socket_clone = audio_socket.clone();
    let client_addrs = current_client_addrs.clone();
    let streams = client_streams.clone();
    let fmt_ch = audio_format.channels;
    let fmt_sr = audio_format.sample_rate;
    let fmt_bf = audio_format.buffer_frames as u32;
    let fmt_sfmt = audio_format.sample_format;
    let vols = volumes.clone();
    let crypto_cap = crypto.clone();
    tokio::spawn(async move {
        loop {
            match capture_rx.recv() {
                Ok(mut samples) => {
                    let addr_guard = client_addrs.read().await;
                    if addr_guard.is_empty() {
                        continue;
                    }

                    let vol_guard = vols.read().await;
                    for (i, sample) in samples.iter_mut().enumerate() {
                        let ch_idx = i % fmt_ch as usize;
                        *sample *= vol_guard[ch_idx];
                    }
                    drop(vol_guard);

                    let mut stream_guard = streams.write().await;
                    for addr in addr_guard.iter() {
                        let entry = stream_guard.entry(*addr).or_insert_with(|| ClientStream {
                            stream_id: 0,
                            sequence: 0,
                        });
                        let seq = entry.sequence;
                        entry.sequence = entry.sequence.wrapping_add(1);

                        let header = AudioPacketHeader::new(
                            entry.stream_id,
                            seq,
                            fmt_ch,
                            fmt_sr,
                            fmt_sfmt,
                            fmt_bf,
                        );

                        let audio_addr = std::net::SocketAddr::new(addr.ip(), audio_port);

                        if let Err(e) = audio_stream::send_audio_packet(
                            &audio_socket_clone,
                            &audio_addr,
                            &header,
                            &samples,
                            crypto_cap.as_ref(),
                        )
                        .await
                        {
                            log::warn!("Failed to send audio to {}: {}", addr, e);
                        }
                    }
                    drop(stream_guard);
                }
                Err(channel::RecvError) => {
                    log::error!("Capture channel closed");
                    break;
                }
            }
        }
    });

    let audio_socket_clone2 = audio_socket.clone();
    let client_addr_filter = current_client_addrs.clone();
    let crypto_playback = crypto.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; audio_stream::MAX_PACKET_SIZE];
        loop {
            match audio_socket_clone2.recv_from(&mut buf).await {
                Ok((len, src_addr)) => {
                    let addr_guard = client_addr_filter.read().await;
                    let allowed = addr_guard.iter().any(|a| a.ip() == src_addr.ip());
                    drop(addr_guard);
                    if !allowed {
                        continue;
                    }
                    if let Some((_header, samples)) = audio_stream::parse_audio_packet_decrypt(
                        &buf[..len],
                        crypto_playback.as_ref(),
                    ) {
                        if let Err(e) = playback_tx.try_send(samples) {
                            if !e.is_disconnected() {
                                log::warn!("Playback buffer full, dropping frame");
                            } else {
                                log::error!("Playback channel disconnected");
                                break;
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

    log::info!("Waiting for client connections on port {}...", control_port);

    loop {
        if stop_flag.load(Ordering::SeqCst) {
            log::info!("Server shutting down...");
            break Ok(());
        }
        let (tcp_stream, client_addr) = control_listener.accept().await?;
        log::info!("Client connected from {}", client_addr);

        let connected = connected_clients.clone();
        let max_clients = settings.server.max_clients;
        let current_addr = current_client_addrs.clone();
        let audio_fmt = actual_format;
        let device_name_str = device_name.clone();
        let sid_counter = next_stream_id.clone();
        let streams = client_streams.clone();
        let vols = volumes.clone();
        let enc_enabled = settings.encryption.enabled;
        let psk = settings.encryption.pre_shared_key.clone();

        tokio::spawn(async move {
            if connected.load(Ordering::SeqCst) >= max_clients {
                log::warn!("Rejected client {} (max clients reached)", client_addr);
                return;
            }
            connected.fetch_add(1, Ordering::SeqCst);

            let (mut read_half, mut write_half) = tokio::io::split(tcp_stream);
            let mut reader = BufReader::new(&mut read_half);

            if enc_enabled && !psk.is_empty() {
                let challenge = format!("{:016x}", fastrand::u64(..));
                if let Err(e) = control::send_control(
                    &mut write_half,
                    &ControlMessage::Auth {
                        challenge: challenge.clone(),
                    },
                )
                .await
                {
                    log::error!("Failed to send auth challenge: {}", e);
                    connected.fetch_sub(1, Ordering::SeqCst);
                    return;
                }

                match control::recv_control(&mut reader).await {
                    Ok(ControlMessage::AuthResponse { hash }) => {
                        let expected = control::compute_auth_response(&challenge, &psk);
                        if hash != expected {
                            log::warn!("Auth failed for {}", client_addr);
                            let _ = control::send_control(
                                &mut write_half,
                                &ControlMessage::Error {
                                    message: "Authentication failed".to_string(),
                                },
                            )
                            .await;
                            connected.fetch_sub(1, Ordering::SeqCst);
                            return;
                        }
                        let _ =
                            control::send_control(&mut write_half, &ControlMessage::AuthOk).await;
                        log::info!("Client {} authenticated", client_addr);
                    }
                    Ok(_) | Err(_) => {
                        log::warn!("Auth failed for {} (invalid response)", client_addr);
                        connected.fetch_sub(1, Ordering::SeqCst);
                        return;
                    }
                }
            }

            loop {
                match control::recv_control(&mut reader).await {
                    Ok(msg) => match msg {
                        ControlMessage::ListDevices => {
                            let devices = vec![DeviceInfo {
                                index: 0,
                                name: device_name_str.clone(),
                                input_channels: audio_fmt.channels,
                                output_channels: audio_fmt.channels,
                                sample_rates: vec![44100, 48000, 96000, 192000],
                            }];
                            if let Err(e) = control::send_control(
                                &mut write_half,
                                &ControlMessage::DeviceList { devices },
                            )
                            .await
                            {
                                log::error!("Failed to send device list: {}", e);
                                break;
                            }
                        }
                        ControlMessage::SelectDevice {
                            channels,
                            sample_rate,
                            ..
                        } => {
                            let sid = sid_counter.fetch_add(1, Ordering::SeqCst);
                            let new_addr = std::net::SocketAddr::new(client_addr.ip(), 0);
                            {
                                let mut stream_map = streams.write().await;
                                stream_map.insert(
                                    new_addr,
                                    ClientStream {
                                        stream_id: sid,
                                        sequence: 0,
                                    },
                                );
                            }
                            let mut addrs = current_addr.write().await;
                            let audio_addr = std::net::SocketAddr::new(client_addr.ip(), 0);
                            if !addrs.iter().any(|a| a.ip() == client_addr.ip()) {
                                addrs.push(audio_addr);
                            }
                            if let Err(e) = control::send_control(
                                &mut write_half,
                                &ControlMessage::DeviceSelected {
                                    device_id: 0,
                                    channels,
                                    sample_rate,
                                    stream_id: sid,
                                },
                            )
                            .await
                            {
                                log::error!("Failed to confirm device: {}", e);
                                break;
                            }
                            log::info!(
                                "Client {} selected device: {} ch @ {} Hz (stream #{})",
                                client_addr,
                                channels,
                                sample_rate,
                                sid
                            );
                        }
                        ControlMessage::StartStream { direction } => {
                            log::info!("Client {} started {} stream", client_addr, direction);
                            if let Err(e) = control::send_control(
                                &mut write_half,
                                &ControlMessage::Status {
                                    running: true,
                                    uptime_secs: 0,
                                    clients: connected.load(Ordering::SeqCst),
                                    stream_id: None,
                                    audio_rx_kbps: 0.0,
                                    audio_tx_kbps: 0.0,
                                },
                            )
                            .await
                            {
                                log::error!("Failed to send status: {}", e);
                                break;
                            }
                        }
                        ControlMessage::StopStream => {
                            log::info!("Client {} stopped stream", client_addr);
                            current_addr
                                .write()
                                .await
                                .retain(|a| a.ip() != client_addr.ip());
                        }
                        ControlMessage::SetVolume { channel, volume } => {
                            let mut vol_guard = vols.write().await;
                            let ch_idx = channel as usize;
                            if ch_idx < vol_guard.len() {
                                vol_guard[ch_idx] = volume.clamp(0.0, 1.0);
                                log::info!(
                                    "Client {} set channel {} volume to {:.2}",
                                    client_addr,
                                    channel,
                                    volume
                                );
                            }
                        }
                        ControlMessage::Ping => {
                            if let Err(e) =
                                control::send_control(&mut write_half, &ControlMessage::Pong).await
                            {
                                log::error!("Failed to send pong: {}", e);
                                break;
                            }
                        }
                        _ => {
                            log::warn!("Unexpected message from {}: {:?}", client_addr, msg);
                        }
                    },
                    Err(e) => {
                        log::info!("Client {} disconnected: {}", client_addr, e);
                        break;
                    }
                }
            }

            connected.fetch_sub(1, Ordering::SeqCst);
            current_addr
                .write()
                .await
                .retain(|a| a.ip() != client_addr.ip());
            let cleanup_addr = std::net::SocketAddr::new(client_addr.ip(), 0);
            streams.write().await.remove(&cleanup_addr);
            log::info!("Client {} removed", client_addr);
        });
    }
}
