use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use anyhow::Result;
use cpal::traits::DeviceTrait;
use crossbeam::channel;
use tokio::io::BufReader;
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio::time::Duration;

use crate::audio::capture::AudioCapture;
use crate::audio::playback::AudioPlayback;
use crate::config::Settings;
use crate::net::audio_stream::{self, AudioPacketHeader};
use crate::net::control::{self, ControlMessage, DeviceInfo};
use crate::net::discovery;

/// How often the accept loop checks `stop_flag`.
const STOP_POLL: Duration = Duration::from_millis(250);
/// How often to log the incoming audio buffer and clock drift.
const STATS_INTERVAL: Duration = Duration::from_secs(30);

struct ClientStream {
    stream_id: u32,
    sequence: u64,
}

/// Notifications for front-ends (the web panel) that embed the server.
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// Sockets are bound and clients can connect.
    Listening { control_port: u16 },
    /// The number of connected clients changed.
    Clients(u32),
}

pub type EventCallback = Arc<dyn Fn(ServerEvent) + Send + Sync>;

/// Lets a front-end observe and drive a running server.
pub struct ServerHooks {
    pub on_event: EventCallback,
    /// Per-channel volumes, resized to the channel count on start.
    pub volumes: Arc<RwLock<Vec<f32>>>,
}

fn notify(on_event: &Option<EventCallback>, event: ServerEvent) {
    if let Some(callback) = on_event {
        callback(event);
    }
}

pub fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .ok()
        .or_else(|| {
            let out = std::process::Command::new("hostname").output().ok()?;
            let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!name.is_empty()).then_some(name)
        })
        .unwrap_or_else(|| "unknown".to_string())
}

pub async fn run(settings: Settings, stop_flag: Arc<AtomicBool>) -> Result<()> {
    run_with_hooks(settings, stop_flag, None).await
}

/// Runs the server until `stop_flag` is set. All background tasks are
/// aborted on return, so the server can be restarted in the same process.
pub async fn run_with_hooks(
    settings: Settings,
    stop_flag: Arc<AtomicBool>,
    hooks: Option<ServerHooks>,
) -> Result<()> {
    let hostname = hostname();

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
    let capture_rx = capture.receiver();
    let actual_format = *capture.format();

    let playback = AudioPlayback::new(&output_device, &actual_format, settings.audio.latency_ms)?;
    // Started only now: opening the playback can take a moment, and captured
    // audio would pile up in the meantime.
    capture.start()?;
    playback.start()?;
    let playback_tx = playback.sender();
    let playout_stats = playback.stats();

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
    let web_port = if settings.web.enabled {
        settings.web.port
    } else {
        0
    };
    // Dropped (aborting every task) when this function returns.
    let mut tasks = JoinSet::new();
    let sample_rate = actual_format.sample_rate;
    tasks.spawn(async move {
        let mut ticker = tokio::time::interval(STATS_INTERVAL);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if playout_stats.is_playing() {
                log::info!("Incoming audio: {}", playout_stats.summary(sample_rate));
            }
        }
    });
    tasks.spawn(async move {
        if let Err(e) =
            discovery::announce_server(&hn, &dn, ch, audio_port, control_port, web_port).await
        {
            log::error!("Discovery broadcast failed: {}", e);
        }
    });

    let connected_clients = Arc::new(AtomicU32::new(0));
    let next_stream_id = Arc::new(AtomicU32::new(1));
    let current_client_addrs: Arc<RwLock<Vec<SocketAddr>>> = Arc::new(RwLock::new(Vec::new()));
    let client_streams: Arc<RwLock<HashMap<SocketAddr, ClientStream>>> =
        Arc::new(RwLock::new(HashMap::new()));
    let (on_event, volumes) = match hooks {
        Some(hooks) => (Some(hooks.on_event), hooks.volumes),
        None => (None, Arc::new(RwLock::new(Vec::new()))),
    };
    volumes
        .write()
        .await
        .resize(audio_format.channels as usize, 1.0);

    let audio_socket_clone = audio_socket.clone();
    let client_addrs = current_client_addrs.clone();
    let streams = client_streams.clone();
    let fmt_ch = audio_format.channels;
    let fmt_sr = audio_format.sample_rate;
    let fmt_bf = audio_format.buffer_frames as u32;
    let fmt_sfmt = audio_format.sample_format;
    let vols = volumes.clone();
    let crypto_cap = crypto.clone();
    tasks.spawn(async move {
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
                    // Also the normal path on shutdown, when the capture stream is dropped.
                    log::debug!("Capture channel closed");
                    break;
                }
            }
        }
    });

    let audio_socket_clone2 = audio_socket.clone();
    let client_addr_filter = current_client_addrs.clone();
    let crypto_playback = crypto.clone();
    tasks.spawn(async move {
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
    notify(&on_event, ServerEvent::Listening { control_port });

    let mut stop_check = tokio::time::interval(STOP_POLL);
    let result = loop {
        while tasks.try_join_next().is_some() {}
        let (tcp_stream, client_addr) = tokio::select! {
            accepted = control_listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => break Err(e.into()),
            },
            _ = stop_check.tick() => {
                if stop_flag.load(Ordering::SeqCst) {
                    log::info!("Server shutting down...");
                    break Ok(());
                }
                continue;
            }
        };
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
        let on_event = on_event.clone();

        tasks.spawn(async move {
            if connected.load(Ordering::SeqCst) >= max_clients {
                log::warn!("Rejected client {} (max clients reached)", client_addr);
                return;
            }
            let count = connected.fetch_add(1, Ordering::SeqCst) + 1;
            notify(&on_event, ServerEvent::Clients(count));
            let leave = || {
                let count = connected.fetch_sub(1, Ordering::SeqCst) - 1;
                notify(&on_event, ServerEvent::Clients(count));
            };

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
                    leave();
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
                            leave();
                            return;
                        }
                        let _ =
                            control::send_control(&mut write_half, &ControlMessage::AuthOk).await;
                        log::info!("Client {} authenticated", client_addr);
                    }
                    Ok(_) | Err(_) => {
                        log::warn!("Auth failed for {} (invalid response)", client_addr);
                        leave();
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

            leave();
            current_addr
                .write()
                .await
                .retain(|a| a.ip() != client_addr.ip());
            let cleanup_addr = std::net::SocketAddr::new(client_addr.ip(), 0);
            streams.write().await.remove(&cleanup_addr);
            log::info!("Client {} removed", client_addr);
        });
    };

    // Stopping the audio streams closes the channels the capture task blocks on,
    // so every task can finish and release its socket before we return.
    drop(capture);
    drop(playback);
    drop(control_listener);
    tasks.shutdown().await;
    result
}
