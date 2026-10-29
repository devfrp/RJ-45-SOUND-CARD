//! Web control panel served by `rjsc serve`, and the supervisor that
//! (re)starts the audio server when settings change or the device is missing.
//! `home` serves the laptop/setup page used by `rjsc start`.

pub mod home;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, RwLock};

use crate::config::{self, AudioSettings, Settings};
use crate::logging::{LogBuffer, LogEntry};
use crate::net::control::DeviceInfo;
use crate::net::crypto::PacketCrypto;
use crate::server::{self, ServerEvent, ServerHooks};

/// Delay before retrying when the audio server fails (e.g. sound card not plugged in yet).
const RETRY_DELAY: Duration = Duration::from_secs(10);
const STOP_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone, Serialize)]
struct Status {
    phase: Phase,
    error: Option<String>,
    control_port: u16,
    clients: u32,
    /// Unix time (ms) when the server entered `Running`.
    running_since_ms: Option<u64>,
}

struct Panel {
    /// Identifies this process, so the page notices a server restart.
    started_ms: u64,
    config_path: String,
    hostname: String,
    settings: RwLock<Settings>,
    status: Mutex<Status>,
    volumes: Arc<RwLock<Vec<f32>>>,
    logs: &'static LogBuffer,
    restart: Notify,
}

impl Panel {
    fn status(&self) -> std::sync::MutexGuard<'_, Status> {
        self.status.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_phase(&self, phase: Phase, error: Option<String>) {
        let mut status = self.status();
        status.phase = phase;
        status.error = error;
        status.clients = 0;
        status.running_since_ms = None;
    }

    fn on_server_event(&self, event: ServerEvent) {
        let mut status = self.status();
        match event {
            ServerEvent::Listening { control_port } => {
                status.phase = Phase::Running;
                status.error = None;
                status.control_port = control_port;
                status.running_since_ms = Some(now_ms());
            }
            ServerEvent::Clients(count) => status.clients = count,
        }
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) async fn wait_for(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(STOP_POLL).await;
    }
}

/// Runs the audio server with the web control panel until `stop_flag` is set.
pub async fn serve(
    settings: Settings,
    config_path: String,
    stop_flag: Arc<AtomicBool>,
    logs: &'static LogBuffer,
) -> Result<()> {
    let web_addr = format!("{}:{}", settings.network.bind_address, settings.web.port);
    let web_port = settings.web.port;
    let panel = Arc::new(Panel {
        started_ms: now_ms(),
        config_path,
        hostname: server::hostname(),
        settings: RwLock::new(settings),
        status: Mutex::new(Status {
            phase: Phase::Starting,
            error: None,
            control_port: 0,
            clients: 0,
            running_since_ms: None,
        }),
        volumes: Arc::new(RwLock::new(Vec::new())),
        logs,
        restart: Notify::new(),
    });

    // The audio server keeps running even if the panel can't bind its port.
    match tokio::net::TcpListener::bind(&web_addr).await {
        Ok(listener) => {
            log::info!(
                "Control panel: http://<server-ip>:{} (http://localhost:{} on this machine)",
                web_port,
                web_port
            );
            let app = router(panel.clone());
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, app).await {
                    log::error!("Control panel stopped: {}", e);
                }
            });
        }
        Err(e) => log::error!("Control panel unavailable on {}: {}", web_addr, e),
    }

    supervise(panel, stop_flag).await
}

async fn supervise(panel: Arc<Panel>, stop_flag: Arc<AtomicBool>) -> Result<()> {
    loop {
        let settings = panel.settings.read().await.clone();
        panel.set_phase(Phase::Starting, None);

        let server_stop = Arc::new(AtomicBool::new(false));
        let events = panel.clone();
        let hooks = ServerHooks {
            on_event: Arc::new(move |event| events.on_server_event(event)),
            volumes: panel.volumes.clone(),
        };
        let run = server::run_with_hooks(settings, server_stop.clone(), Some(hooks));
        tokio::pin!(run);

        // The server is the one future here that isn't `Send`, so it runs inline.
        let (result, interrupted) = tokio::select! {
            result = &mut run => (result, false),
            _ = panel.restart.notified() => {
                server_stop.store(true, Ordering::SeqCst);
                (run.await, true)
            }
            _ = wait_for(&stop_flag) => {
                server_stop.store(true, Ordering::SeqCst);
                (run.await, true)
            }
        };

        if stop_flag.load(Ordering::SeqCst) {
            return Ok(());
        }
        if interrupted {
            log::info!("Restarting audio server with the new settings...");
            continue;
        }
        if let Err(e) = result {
            log::error!(
                "Audio server error: {:#}. Retrying in {} s...",
                e,
                RETRY_DELAY.as_secs()
            );
            panel.set_phase(Phase::Error, Some(format!("{:#}", e)));
            tokio::select! {
                _ = tokio::time::sleep(RETRY_DELAY) => {}
                _ = panel.restart.notified() => {}
                _ = wait_for(&stop_flag) => return Ok(()),
            }
        }
    }
}

fn router(panel: Arc<Panel>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(stylesheet))
        .route("/api/state", get(get_state))
        .route("/api/devices", get(get_devices))
        .route("/api/settings", post(post_settings))
        .route("/api/volume", post(post_volume))
        .route("/api/restart", post(post_restart))
        .with_state(panel)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("panel.html"))
}

pub(crate) async fn stylesheet() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("style.css"),
    )
}

#[derive(Deserialize)]
struct StateQuery {
    #[serde(default)]
    logs_after: u64,
}

/// Settings as shown in the panel; the pre-shared key itself is never sent out.
#[derive(Serialize)]
struct PanelSettings {
    audio: AudioSettings,
    max_clients: u32,
    encryption_enabled: bool,
    has_key: bool,
}

#[derive(Serialize)]
struct StateResponse {
    /// Lets a page notice that this port now serves another page (after setup).
    mode: &'static str,
    version: &'static str,
    started_ms: u64,
    hostname: String,
    config_path: String,
    status: Status,
    settings: PanelSettings,
    volumes: Vec<f32>,
    logs: Vec<LogEntry>,
}

async fn get_state(
    State(panel): State<Arc<Panel>>,
    Query(query): Query<StateQuery>,
) -> Json<StateResponse> {
    let settings = {
        let s = panel.settings.read().await;
        PanelSettings {
            audio: s.audio.clone(),
            max_clients: s.server.max_clients,
            encryption_enabled: s.encryption.enabled,
            has_key: !s.encryption.pre_shared_key.is_empty(),
        }
    };
    let volumes = panel.volumes.read().await.clone();
    Json(StateResponse {
        mode: "server",
        version: env!("CARGO_PKG_VERSION"),
        started_ms: panel.started_ms,
        hostname: panel.hostname.clone(),
        config_path: panel.config_path.clone(),
        status: panel.status().clone(),
        settings,
        volumes,
        logs: panel.logs.since(query.logs_after),
    })
}

async fn get_devices() -> Json<Vec<DeviceInfo>> {
    let devices = tokio::task::spawn_blocking(crate::audio::device_infos)
        .await
        .unwrap_or_default();
    Json(devices)
}

/// `error` is a stable code the panel translates; `detail` is free text.
#[derive(Serialize)]
struct ApiResult {
    ok: bool,
    error: Option<&'static str>,
    detail: Option<String>,
}

impl ApiResult {
    fn ok() -> Response {
        Json(Self {
            ok: true,
            error: None,
            detail: None,
        })
        .into_response()
    }

    fn error(status: StatusCode, code: &'static str, detail: Option<String>) -> Response {
        let body = Self {
            ok: false,
            error: Some(code),
            detail,
        };
        (status, Json(body)).into_response()
    }
}

#[derive(Debug, Deserialize)]
struct SettingsUpdate {
    audio: AudioSettings,
    max_clients: u32,
    encryption_enabled: bool,
    /// Empty keeps the current key.
    #[serde(default)]
    pre_shared_key: String,
}

fn apply_update(settings: &mut Settings, update: SettingsUpdate) -> Result<(), &'static str> {
    let audio = update.audio;
    if !(1..=64).contains(&audio.channels) {
        return Err("channels");
    }
    if !(8_000..=384_000).contains(&audio.sample_rate) {
        return Err("sample_rate");
    }
    if !(16..=8192).contains(&audio.buffer_frames) {
        return Err("buffer_frames");
    }
    if !(1..=500).contains(&audio.latency_ms) {
        return Err("latency_ms");
    }
    if update.max_clients == 0 {
        return Err("max_clients");
    }
    let key = update.pre_shared_key.trim();
    if !key.is_empty() && PacketCrypto::from_hex(key).is_err() {
        return Err("key_invalid");
    }
    if update.encryption_enabled && key.is_empty() && settings.encryption.pre_shared_key.is_empty()
    {
        return Err("key_missing");
    }

    settings.audio = audio;
    settings.server.max_clients = update.max_clients;
    settings.encryption.enabled = update.encryption_enabled;
    if !key.is_empty() {
        settings.encryption.pre_shared_key = key.to_string();
    }
    Ok(())
}

async fn post_settings(
    State(panel): State<Arc<Panel>>,
    Json(update): Json<SettingsUpdate>,
) -> Response {
    let settings = {
        let mut settings = panel.settings.write().await;
        if let Err(code) = apply_update(&mut settings, update) {
            return ApiResult::error(StatusCode::BAD_REQUEST, code, None);
        }
        settings.clone()
    };
    log::info!("Settings changed from the control panel");
    panel.restart.notify_one();
    match config::save(&panel.config_path, &settings) {
        Ok(()) => ApiResult::ok(),
        Err(e) => {
            log::error!("Could not save '{}': {:#}", panel.config_path, e);
            ApiResult::error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "save_failed",
                Some(format!("{}: {:#}", panel.config_path, e)),
            )
        }
    }
}

#[derive(Deserialize)]
struct VolumeUpdate {
    channel: usize,
    volume: f32,
}

async fn post_volume(
    State(panel): State<Arc<Panel>>,
    Json(update): Json<VolumeUpdate>,
) -> Response {
    let mut volumes = panel.volumes.write().await;
    match volumes.get_mut(update.channel) {
        Some(volume) => {
            *volume = update.volume.clamp(0.0, 1.0);
            ApiResult::ok()
        }
        None => ApiResult::error(StatusCode::BAD_REQUEST, "channel", None),
    }
}

// Requiring a JSON body makes browsers preflight cross-site requests,
// so other web pages can't trigger a restart.
async fn post_restart(
    State(panel): State<Arc<Panel>>,
    Json(_): Json<serde::de::IgnoredAny>,
) -> Response {
    log::info!("Restart requested from the control panel");
    panel.restart.notify_one();
    ApiResult::ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(channels: u16, encryption_enabled: bool, key: &str) -> SettingsUpdate {
        SettingsUpdate {
            audio: AudioSettings {
                channels,
                ..AudioSettings::default()
            },
            max_clients: 4,
            encryption_enabled,
            pre_shared_key: key.to_string(),
        }
    }

    #[test]
    fn test_apply_update_validates_ranges() {
        let mut settings = Settings::default();
        assert_eq!(
            apply_update(&mut settings, update(0, false, "")),
            Err("channels")
        );
        assert_eq!(
            apply_update(&mut settings, update(65, false, "")),
            Err("channels")
        );
        assert_eq!(apply_update(&mut settings, update(8, false, "")), Ok(()));
        assert_eq!(settings.audio.channels, 8);
    }

    #[test]
    fn test_apply_update_key_handling() {
        let mut settings = Settings::default();
        assert_eq!(
            apply_update(&mut settings, update(2, true, "")),
            Err("key_missing")
        );
        assert_eq!(
            apply_update(&mut settings, update(2, true, "not-hex")),
            Err("key_invalid")
        );
        assert_eq!(
            apply_update(&mut settings, update(2, true, "a1b2c3d4")),
            Ok(())
        );
        assert_eq!(settings.encryption.pre_shared_key, "a1b2c3d4");

        // An empty key keeps the stored one.
        assert_eq!(apply_update(&mut settings, update(2, true, "")), Ok(()));
        assert_eq!(settings.encryption.pre_shared_key, "a1b2c3d4");
    }
}
