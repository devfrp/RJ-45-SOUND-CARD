//! `rjsc start` (and `rjsc` with no arguments, i.e. a double-click): runs the
//! role chosen for this PC, or the one-time setup page when none is chosen yet.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use crate::config::{self, Role};
use crate::logging::LogBuffer;
use crate::{autostart, web};

/// `open_page`: open the local page in the browser (when started by the user,
/// not by autostart).
pub async fn run(config: Option<String>, open_page: bool, logs: &'static LogBuffer) -> Result<()> {
    let config_path = match config {
        Some(path) => path,
        None => autostart::default_config_path()?
            .to_string_lossy()
            .into_owned(),
    };
    let mut settings = config::load(&config_path)?;
    let port = settings.web.port;
    let url = format!("http://localhost:{}", port);

    // Already running (e.g. started at boot): just show its page.
    if answers(port).await {
        log::info!("rjsc is already running: {}", url);
        if open_page {
            open_browser(&url);
        }
        return Ok(());
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let sf = stop_flag.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        sf.store(true, Ordering::SeqCst);
    });
    if open_page {
        tokio::spawn(open_when_ready(port, url));
    }

    if settings.role.is_none() {
        let Some(role) =
            web::home::setup(&settings, config_path.clone(), stop_flag.clone(), logs).await?
        else {
            return Ok(());
        };
        // Hand over to the autostarted instance, which picks up the saved role
        // and serves the page on the port this process just released.
        match autostart::install_for(Path::new(&config_path)) {
            Ok(_) => {
                log::info!("Autostart enabled, running in the background from now on");
                return Ok(());
            }
            Err(e) => {
                log::warn!(
                    "Could not enable autostart ({:#}); running in this window instead",
                    e
                );
                settings.role = Some(role);
            }
        }
    }

    match settings.role {
        Some(Role::Server) if settings.web.enabled => {
            web::serve(settings, config_path, stop_flag, logs).await
        }
        Some(Role::Server) => crate::server::run(settings, stop_flag).await,
        Some(Role::Client) => web::home::client(settings, config_path, stop_flag, logs).await,
        None => unreachable!("role chosen above"),
    }
}

async fn answers(port: u16) -> bool {
    let connect = tokio::net::TcpStream::connect(("127.0.0.1", port));
    matches!(
        tokio::time::timeout(Duration::from_millis(500), connect).await,
        Ok(Ok(_))
    )
}

async fn open_when_ready(port: u16, url: String) {
    for _ in 0..40 {
        if answers(port).await {
            open_browser(&url);
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    log::warn!("Open {} in a browser", url);
}

pub fn open_browser(url: &str) {
    let result = if cfg!(windows) {
        // `start` is a cmd builtin; the empty string is the window title.
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(e) = result {
        log::warn!("Could not open the browser ({}); open {} manually", e, url);
    }
}
