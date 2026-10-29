use anyhow::Result;
use cpal::traits::DeviceTrait;
use crossbeam::channel;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};

use crate::audio::capture::AudioCapture;
use crate::audio::format::AudioFormat;
use crate::audio::jitter::JitterBuffer;
use crate::audio::playback::AudioPlayback;
use crate::config::Settings;
use crate::net::audio_stream::{self, AudioPacketHeader};
use crate::net::control::{self, ControlMessage};
use crate::net::crypto::PacketCrypto;
use crate::net::discovery;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_DELAY: Duration = Duration::from_secs(5);
/// How often the session loop checks `stop_flag` and pending commands.
const STOP_POLL: Duration = Duration::from_millis(250);
/// Keep-alive ping every 10 s, counted in `STOP_POLL` ticks.
const PING_EVERY_TICKS: u32 = 40;
/// Log the incoming audio buffer and clock drift every 30 s.
const STATS_EVERY_TICKS: u32 = 120;

/// Notifications for front-ends (the laptop's local page) that embed the client.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Found by discovery; `panel_url` is the server's web panel, if enabled.
    ServerFound {
        hostname: String,
        panel_url: Option<String>,
    },
    Connected {
        server: String,
    },
    Streaming,
    Disconnected,
}

/// Lets a front-end drive a running client: messages received on `commands`
/// (e.g. `SetVolume`) are forwarded to the server, session changes go to `on_event`.
pub struct ClientHooks {
    pub commands: channel::Receiver<ControlMessage>,
    pub on_event: Arc<dyn Fn(ClientEvent) + Send + Sync>,
}

impl ClientHooks {
    fn notify(&self, event: ClientEvent) {
        (self.on_event)(event);
    }
}

/// Aborts the session's background tasks when the session ends, so their
/// sockets are released before reconnecting.
struct AbortOnDrop(Vec<JoinHandle<()>>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

pub async fn run(
    settings: Settings,
    server_addr: Option<String>,
    stop_flag: Arc<AtomicBool>,
) -> Result<()> {
    run_with_hooks(settings, server_addr, stop_flag, None).await
}

pub async fn run_with_hooks(
    settings: Settings,
    server_addr: Option<String>,
    stop_flag: Arc<AtomicBool>,
    hooks: Option<ClientHooks>,
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
        let panel_url =
            (msg.web_port != 0).then(|| format!("http://{}:{}", addr.ip(), msg.web_port));
        if let Some(url) = &panel_url {
            log::info!("Control panel: {}", url);
        }
        if let Some(hooks) = &hooks {
            hooks.notify(ClientEvent::ServerFound {
                hostname: msg.hostname.clone(),
                panel_url,
            });
        }
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
        let result = run_session(
            &settings,
            &server_addr,
            &server_ip,
            discovered_audio_port,
            &stop_flag,
            hooks.as_ref(),
        )
        .await;
        if let Some(hooks) = &hooks {
            hooks.notify(ClientEvent::Disconnected);
        }
        if stop_flag.load(Ordering::SeqCst) {
            return Ok(());
        }
        match result {
            Ok(()) if settings.client.auto_reconnect => {
                log::info!("Session ended, reconnecting in 5 seconds...");
            }
            Err(e) if settings.client.auto_reconnect => {
                log::error!("Session error: {}. Reconnecting in 5 seconds...", e);
            }
            other => return other,
        }
        if !sleep_unless_stopped(RECONNECT_DELAY, &stop_flag).await {
            return Ok(());
        }
    }
}

/// Sleeps for `duration`, waking early if `stop_flag` is set.
/// Returns `false` when stopped.
async fn sleep_unless_stopped(duration: Duration, stop_flag: &AtomicBool) -> bool {
    let ticks = (duration.as_millis() / STOP_POLL.as_millis()).max(1);
    for _ in 0..ticks {
        if stop_flag.load(Ordering::SeqCst) {
            return false;
        }
        sleep(STOP_POLL).await;
    }
    !stop_flag.load(Ordering::SeqCst)
}

async fn recv(reader: &mut (impl AsyncRead + Unpin)) -> Result<ControlMessage> {
    timeout(REPLY_TIMEOUT, control::recv_control(reader))
        .await
        .map_err(|_| anyhow::anyhow!("Server did not reply within {} s", REPLY_TIMEOUT.as_secs()))?
}

async fn run_session(
    settings: &Settings,
    server_addr: &str,
    server_ip: &str,
    discovered_audio_port: Option<u16>,
    stop_flag: &AtomicBool,
    hooks: Option<&ClientHooks>,
) -> Result<()> {
    log::info!("Connecting to server at {}...", server_addr);
    let tcp_stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(server_addr))
        .await
        .map_err(|_| anyhow::anyhow!("Connection to {} timed out", server_addr))??;
    let (read_half, mut write_half) = tcp_stream.into_split();
    let mut reader = BufReader::new(read_half);
    log::info!("Connected to server");
    if let Some(hooks) = hooks {
        hooks.notify(ClientEvent::Connected {
            server: server_addr.to_string(),
        });
    }

    let crypto = settings.encryption_key();

    // The server only speaks first when it requires authentication,
    // which it does under the same condition as below.
    let psk = &settings.encryption.pre_shared_key;
    if settings.encryption.enabled && !psk.is_empty() {
        let challenge = match recv(&mut reader).await {
            Ok(ControlMessage::Auth { challenge }) => challenge,
            Ok(other) => anyhow::bail!("Expected an auth challenge, got {:?}", other),
            Err(e) => anyhow::bail!("{} (is encryption enabled on the server too?)", e),
        };
        let hash = control::compute_auth_response(&challenge, psk);
        control::send_control(&mut write_half, &ControlMessage::AuthResponse { hash }).await?;

        match recv(&mut reader).await? {
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
    }

    control::send_control(&mut write_half, &ControlMessage::ListDevices).await?;
    log::info!("Requested device list...");
    read_msg_loop(
        settings,
        reader,
        &mut write_half,
        server_ip,
        discovered_audio_port,
        stop_flag,
        crypto,
        hooks,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn read_msg_loop(
    settings: &Settings,
    mut reader: BufReader<OwnedReadHalf>,
    write_half: &mut OwnedWriteHalf,
    server_ip: &str,
    discovered_audio_port: Option<u16>,
    stop_flag: &AtomicBool,
    crypto: Option<PacketCrypto>,
    hooks: Option<&ClientHooks>,
) -> Result<()> {
    let device_list = loop {
        match recv(&mut reader).await? {
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
            ControlMessage::Auth { .. } => {
                anyhow::bail!(
                    "Server requires authentication: enable [encryption] with the server's pre_shared_key"
                );
            }
            ControlMessage::Error { message } => {
                anyhow::bail!("Server error: {}", message);
            }
            other => {
                log::warn!("Unexpected message: {:?}", other);
            }
        }
    };

    if device_list.is_empty() {
        anyhow::bail!("No audio devices available on server");
    }

    // Follow the server: all its channels, at its sample rate (older servers
    // list several rates instead of their own; the local setting is used then).
    let device = &device_list[0];
    let channels = device.input_channels.max(1);
    let sample_rate = match device.sample_rates.as_slice() {
        [rate] => *rate,
        _ => settings.audio.sample_rate,
    };

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
        match recv(&mut reader).await? {
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

    let output_device = if settings.client.use_virtual_device {
        virtual_output_device(&settings.client.virtual_device_name)?
    } else {
        crate::audio::default_output_device()?
    };
    let input_device = crate::audio::default_input_device()?;

    // Local devices may have fewer channels than the server: open what they
    // have; audio is remapped channel N to channel N.
    let local_format = |device: &cpal::Device, output: bool| {
        let available = crate::audio::max_channels(device, output).unwrap_or(channels);
        AudioFormat::with_format(
            channels.min(available).max(1),
            sample_rate,
            settings.audio.buffer_frames,
            settings.audio.sample_format,
        )
    };
    let playback_format = local_format(&output_device, true);
    let capture_format = local_format(&input_device, false);
    if playback_format.channels < channels {
        log::warn!(
            "Output device has {} channels, server sends {}: extra channels are dropped",
            playback_format.channels,
            channels
        );
    }
    log::info!(
        "Streaming {} channels @ {} Hz from the server",
        channels,
        sample_rate
    );

    let playback = AudioPlayback::new(&output_device, &playback_format, settings.audio.latency_ms)?;
    let playout_stats = playback.stats();
    playback.start()?;
    let playback_tx = playback.sender();

    let capture = AudioCapture::new(&input_device, &capture_format)?;
    capture.start()?;
    let capture_rx = capture.receiver();

    control::send_control(
        write_half,
        &ControlMessage::StartStream {
            direction: "both".to_string(),
        },
    )
    .await?;
    if let Some(hooks) = hooks {
        hooks.notify(ClientEvent::Streaming);
    }

    let audio_port = discovered_audio_port.unwrap_or(settings.network.audio_port);
    let server_audio_addr: std::net::SocketAddr =
        format!("{}:{}", server_ip, audio_port).parse()?;

    // Jitter-buffered receive path
    // Packets are remapped to the playback channel count before reordering.
    let playback_channels = playback_format.channels as usize;
    let jitter_buffer = Arc::new(Mutex::new(JitterBuffer::new(
        settings.client.jitter_buffer_ms,
        sample_rate,
        playback_format.channels,
    )));
    let rx_sock = UdpSocket::bind(format!("0.0.0.0:{}", settings.network.audio_port)).await?;
    let rx_tx = playback_tx.clone();
    let jb_clone = jitter_buffer.clone();
    let crypto_rx = crypto.clone();
    let rx_task = tokio::spawn(async move {
        let mut buf = vec![0u8; audio_stream::MAX_PACKET_SIZE];
        loop {
            match rx_sock.recv_from(&mut buf).await {
                Ok((len, _addr)) => {
                    if let Some((header, samples)) =
                        audio_stream::parse_audio_packet_decrypt(&buf[..len], crypto_rx.as_ref())
                    {
                        let samples = crate::audio::format::remap_channels(
                            samples,
                            header.channels as usize,
                            playback_channels,
                        );
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
    let template = AudioPacketHeader::new(
        0,
        0,
        capture_format.channels,
        capture_format.sample_rate,
        capture_format.sample_format,
        0,
    );
    let crypto_tx = crypto.clone();
    let tx_task = tokio::spawn(async move {
        let mut sequence: u64 = 0;
        loop {
            match capture_rx.recv() {
                Ok(samples) => {
                    if let Err(e) = audio_stream::send_audio_block(
                        &tx_sock,
                        &server_audio_addr,
                        &template,
                        &mut sequence,
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

    // Drain server replies (status, pong) so the control socket never fills up;
    // the task ending means the server closed the connection.
    let inbox_task = tokio::spawn(async move {
        loop {
            match control::recv_control(&mut reader).await {
                Ok(ControlMessage::Error { message }) => log::warn!("Server error: {}", message),
                Ok(msg) => log::debug!("Server message: {:?}", msg),
                Err(_) => break,
            }
        }
    });
    let tasks = AbortOnDrop(vec![rx_task, tx_task, inbox_task]);

    // Keep-alive ping loop, also forwarding front-end commands
    let mut tick = tokio::time::interval(STOP_POLL);
    let mut ticks_since_ping = 0;
    let mut ticks_since_stats = 0;
    loop {
        tick.tick().await;
        if stop_flag.load(Ordering::SeqCst) {
            log::info!("Session stopped cleanly");
            return Ok(());
        }
        if tasks.0[2].is_finished() {
            log::info!("Server closed the control connection");
            return Ok(());
        }
        if let Some(hooks) = hooks {
            while let Ok(msg) = hooks.commands.try_recv() {
                control::send_control(write_half, &msg).await?;
            }
        }
        ticks_since_stats += 1;
        if ticks_since_stats >= STATS_EVERY_TICKS {
            ticks_since_stats = 0;
            log::info!("Incoming audio: {}", playout_stats.summary(sample_rate));
        }
        ticks_since_ping += 1;
        if ticks_since_ping >= PING_EVERY_TICKS {
            ticks_since_ping = 0;
            if let Err(e) = control::send_control(write_half, &ControlMessage::Ping).await {
                log::info!("Keep-alive lost: {}", e);
                return Ok(());
            }
        }
    }
}

/// Picks the configured virtual device, falling back to auto-detection
/// (also used when no name is configured), then to the default output.
fn virtual_output_device(name: &str) -> Result<cpal::Device> {
    if !name.is_empty() {
        match crate::audio::find_device(name) {
            Ok(d) => {
                log::info!("Using virtual audio device: {}", d.name()?);
                return Ok(d);
            }
            Err(_) => log::warn!("Virtual device '{}' not found, trying auto-detect...", name),
        }
    }
    if let Some(detected) = crate::audio::virtual_device::detect_virtual_device() {
        match crate::audio::find_device(&detected) {
            Ok(d) => {
                log::info!("Auto-detected virtual device: {}", d.name()?);
                return Ok(d);
            }
            Err(_) => log::warn!("Auto-detected device not available, using default"),
        }
    } else {
        log::warn!("No virtual device found, using default output");
    }
    crate::audio::default_output_device()
}
