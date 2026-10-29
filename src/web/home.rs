//! Local page (http://localhost:42080) for `rjsc start` when this PC is not a server:
//! the one-time setup (choose server or laptop), then the laptop's status page,
//! which keeps looking for the server and connects as soon as the cable is plugged in.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use crossbeam::channel;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::client::{self, ClientEvent, ClientHooks};
use crate::config::{self, Role, Settings};
use crate::driver;
use crate::logging::{LogBuffer, LogEntry};

use super::{now_ms, wait_for};

/// Pause between two searches for the server.
const SEARCH_PAUSE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Setup,
    Client,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Phase {
    Searching,
    Connecting,
    Connected,
    Streaming,
}

#[derive(Debug, Clone, Serialize)]
struct ClientStatus {
    phase: Phase,
    server_name: Option<String>,
    server_address: Option<String>,
    panel_url: Option<String>,
    /// Last connection error once a server was found (not "no server yet").
    error: Option<String>,
    streaming_since_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum InstallState {
    Idle,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
struct DriverStatus {
    /// Detected virtual device; `None` while unknown or absent.
    device: Option<String>,
    checked: bool,
    install: InstallState,
    message: Option<String>,
    manual_url: &'static str,
}

struct Home {
    mode: Mode,
    started_ms: u64,
    config_path: String,
    hostname: String,
    exe_path: String,
    logs: &'static LogBuffer,
    client: Mutex<ClientStatus>,
    driver: Mutex<DriverStatus>,
    chosen_role: Mutex<Option<Role>>,
    role_chosen: Notify,
    /// Restarts the client session (after a driver install).
    restart: Notify,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Home {
    fn new(mode: Mode, config_path: String, logs: &'static LogBuffer) -> Self {
        Self {
            mode,
            started_ms: now_ms(),
            config_path,
            hostname: crate::server::hostname(),
            exe_path: std::env::current_exe()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
            logs,
            client: Mutex::new(ClientStatus {
                phase: Phase::Searching,
                server_name: None,
                server_address: None,
                panel_url: None,
                error: None,
                streaming_since_ms: None,
            }),
            driver: Mutex::new(DriverStatus {
                device: None,
                checked: false,
                install: InstallState::Idle,
                message: None,
                manual_url: driver::MANUAL_URL,
            }),
            chosen_role: Mutex::new(None),
            role_chosen: Notify::new(),
            restart: Notify::new(),
        }
    }

    fn on_client_event(&self, event: ClientEvent) {
        let mut status = lock(&self.client);
        match event {
            ClientEvent::ServerFound {
                hostname,
                panel_url,
            } => {
                status.phase = Phase::Connecting;
                status.server_name = Some(hostname);
                status.panel_url = panel_url;
            }
            ClientEvent::Connected { server } => {
                status.phase = Phase::Connected;
                status.server_address = Some(server);
                status.error = None;
            }
            ClientEvent::Streaming => {
                status.phase = Phase::Streaming;
                status.streaming_since_ms = Some(now_ms());
            }
            ClientEvent::Disconnected => {
                status.phase = Phase::Searching;
                status.streaming_since_ms = None;
            }
        }
    }

    async fn check_driver(&self) {
        let device = tokio::task::spawn_blocking(driver::detect)
            .await
            .unwrap_or(None);
        let mut status = lock(&self.driver);
        status.device = device;
        status.checked = true;
    }
}

async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    // Local only: this page changes how this PC is set up.
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    log::info!("Local page: http://localhost:{}", port);
    Ok(listener)
}

/// Serves the setup page until a role is chosen and saved. Returns `None` if
/// stopped first. The port is released when this returns.
pub async fn setup(
    settings: &Settings,
    config_path: String,
    stop_flag: Arc<AtomicBool>,
    logs: &'static LogBuffer,
) -> Result<Option<Role>> {
    let home = Arc::new(Home::new(Mode::Setup, config_path, logs));
    let listener = bind(settings.web.port).await?;
    let chosen = home.clone();
    let serve = axum::serve(listener, router(home.clone()))
        .with_graceful_shutdown(async move { chosen.role_chosen.notified().await });
    tokio::select! {
        result = serve => result?,
        _ = wait_for(&stop_flag) => return Ok(None),
    }
    let role = *lock(&home.chosen_role);
    Ok(role)
}

/// Runs the laptop side: keeps looking for the server and streams once connected.
pub async fn client(
    settings: Settings,
    config_path: String,
    stop_flag: Arc<AtomicBool>,
    logs: &'static LogBuffer,
) -> Result<()> {
    let home = Arc::new(Home::new(Mode::Client, config_path, logs));
    match bind(settings.web.port).await {
        Ok(listener) => {
            let app = router(home.clone());
            tokio::spawn(async move {
                if let Err(e) = axum::serve(listener, app).await {
                    log::error!("Local page stopped: {}", e);
                }
            });
        }
        Err(e) => log::error!("Local page unavailable: {}", e),
    }
    home.check_driver().await;

    // Each pass discovers the server again, so a new address (another cable,
    // another server) is picked up.
    let mut session_settings = settings;
    session_settings.client.auto_reconnect = false;
    loop {
        let session_stop = Arc::new(AtomicBool::new(false));
        let events = home.clone();
        let (_commands, commands_rx) = channel::unbounded();
        let hooks = ClientHooks {
            commands: commands_rx,
            on_event: Arc::new(move |event| events.on_client_event(event)),
        };
        lock(&home.client).phase = Phase::Searching;
        let run = client::run_with_hooks(
            session_settings.clone(),
            None,
            session_stop.clone(),
            Some(hooks),
        );
        tokio::pin!(run);

        // The client future isn't `Send` (cpal streams), so it runs inline.
        let (result, interrupted) = tokio::select! {
            result = &mut run => (result, false),
            _ = home.restart.notified() => {
                session_stop.store(true, Ordering::SeqCst);
                (run.await, true)
            }
            _ = wait_for(&stop_flag) => {
                session_stop.store(true, Ordering::SeqCst);
                (run.await, true)
            }
        };
        if stop_flag.load(Ordering::SeqCst) {
            return Ok(());
        }
        if interrupted {
            continue;
        }
        {
            let mut status = lock(&home.client);
            status.phase = Phase::Searching;
            match result {
                // Nothing answered discovery: cable unplugged or server off, not an error.
                Err(_) if status.server_name.is_none() => {}
                Err(e) => status.error = Some(format!("{:#}", e)),
                Ok(()) => {}
            }
            status.server_name = None;
        }
        tokio::select! {
            _ = tokio::time::sleep(SEARCH_PAUSE) => {}
            _ = home.restart.notified() => {}
            _ = wait_for(&stop_flag) => return Ok(()),
        }
    }
}

fn router(home: Arc<Home>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(super::stylesheet))
        .route("/api/state", get(get_state))
        .route("/api/setup", post(post_setup))
        .route("/api/driver/install", post(post_driver_install))
        .route("/api/driver/check", post(post_driver_check))
        .with_state(home)
}

async fn index() -> Html<&'static str> {
    Html(include_str!("home.html"))
}

#[derive(Serialize)]
struct StateResponse {
    mode: Mode,
    started_ms: u64,
    version: &'static str,
    os: &'static str,
    hostname: String,
    config_path: String,
    exe_path: String,
    client: ClientStatus,
    driver: DriverStatus,
    logs: Vec<LogEntry>,
}

#[derive(Deserialize)]
struct StateQuery {
    #[serde(default)]
    logs_after: u64,
}

async fn get_state(
    State(home): State<Arc<Home>>,
    axum::extract::Query(query): axum::extract::Query<StateQuery>,
) -> Json<StateResponse> {
    Json(StateResponse {
        mode: home.mode,
        started_ms: home.started_ms,
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        hostname: home.hostname.clone(),
        config_path: home.config_path.clone(),
        exe_path: home.exe_path.clone(),
        client: lock(&home.client).clone(),
        driver: lock(&home.driver).clone(),
        logs: home.logs.since(query.logs_after),
    })
}

fn reply(status: StatusCode, error: Option<String>) -> Response {
    let body = serde_json::json!({ "ok": error.is_none(), "error": error });
    (status, Json(body)).into_response()
}

#[derive(Deserialize)]
struct SetupRequest {
    role: Role,
}

/// Saves `role` into the configuration file, keeping any other settings in it.
fn save_role(config_path: &str, role: Role) -> Result<()> {
    if let Some(dir) = Path::new(config_path).parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut settings = config::load(config_path)?;
    settings.role = Some(role);
    config::save(config_path, &settings)
}

async fn post_setup(State(home): State<Arc<Home>>, Json(request): Json<SetupRequest>) -> Response {
    if home.mode != Mode::Setup {
        return reply(StatusCode::CONFLICT, Some("already set up".into()));
    }
    if let Err(e) = save_role(&home.config_path, request.role) {
        log::error!("Could not save '{}': {:#}", home.config_path, e);
        return reply(StatusCode::INTERNAL_SERVER_ERROR, Some(format!("{:#}", e)));
    }
    log::info!("This PC is set up as: {:?}", request.role);
    *lock(&home.chosen_role) = Some(request.role);
    // Graceful shutdown lets this response finish before the port is released.
    home.role_chosen.notify_one();
    reply(StatusCode::OK, None)
}

async fn post_driver_install(
    State(home): State<Arc<Home>>,
    Json(_): Json<serde::de::IgnoredAny>,
) -> Response {
    {
        let mut status = lock(&home.driver);
        if status.install == InstallState::Running {
            return reply(StatusCode::CONFLICT, Some("already running".into()));
        }
        status.install = InstallState::Running;
        status.message = None;
    }
    log::info!("Installing the virtual audio driver...");
    let worker = home.clone();
    tokio::spawn(async move {
        let result = tokio::task::spawn_blocking(driver::install).await;
        worker.check_driver().await;
        let mut status = lock(&worker.driver);
        match result {
            Ok(Ok(())) if status.device.is_some() => {
                status.install = InstallState::Done;
                log::info!("Virtual audio driver installed");
            }
            Ok(Ok(())) => {
                // Some drivers only appear after a reboot.
                status.install = InstallState::Done;
                status.message = Some("not_detected".into());
                log::warn!("Driver installer finished, but no virtual device is visible yet");
            }
            Ok(Err(e)) => {
                status.install = InstallState::Failed;
                status.message = Some(format!("{:#}", e));
                log::error!("Driver installation failed: {:#}", e);
            }
            Err(e) => {
                status.install = InstallState::Failed;
                status.message = Some(e.to_string());
            }
        }
        drop(status);
        // Reopen the audio devices with the new driver.
        worker.restart.notify_one();
    });
    reply(StatusCode::OK, None)
}

async fn post_driver_check(
    State(home): State<Arc<Home>>,
    Json(_): Json<serde::de::IgnoredAny>,
) -> Response {
    home.check_driver().await;
    home.restart.notify_one();
    reply(StatusCode::OK, None)
}
