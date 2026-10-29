//! `rjsc autostart`: run `rjsc start` (the role chosen at setup) automatically, using the platform's
//! per-user mechanism (systemd user unit, launchd agent, Windows logon task).
//! Per-user rather than system-wide because audio devices live in the user session.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
const SYSTEMD_UNIT: &str = "rjsc.service";
#[cfg(target_os = "macos")]
const LAUNCHD_LABEL: &str = "com.rj45soundcard.rjsc";
#[cfg(windows)]
const TASK_NAME: &str = "RJ45 Sound Card";

/// Per-user configuration file used when `--config` is not given.
pub fn default_config_path() -> Result<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var("APPDATA").context("APPDATA is not set")?)
    } else if cfg!(target_os = "macos") {
        home()?.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .map_or_else(|| home().map(|h| h.join(".config")), Ok)?
    };
    Ok(base.join("rjsc").join("rjsc.toml"))
}

fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

/// `role`: overrides the saved role; without one, the PC becomes a server.
pub fn enable(config: Option<&str>, role: Option<crate::config::Role>) -> Result<()> {
    let config = match config {
        Some(path) => PathBuf::from(path),
        None => default_config_path()?,
    };
    if !config.exists() {
        if let Some(dir) = config.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let config_str = config.to_string_lossy();
    let mut settings = crate::config::load(&config_str)?;
    if role.is_some() || settings.role.is_none() || !config.exists() {
        // Enabling from the command line is how servers were set up before roles existed.
        settings.role = role.or(settings.role).or(Some(crate::config::Role::Server));
        crate::config::save(&config_str, &settings)?;
    }

    let (exe, config) = install_for(&config)?;

    println!("Autostart enabled.");
    println!("  Program: {}", exe.display());
    println!("  Config:  {}", config.display());
    println!("The program must stay at this location; run this command again if you move it.");
    println!(
        "  Role:    {:?}",
        settings.role.unwrap_or(crate::config::Role::Server)
    );
    println!("Page: http://localhost:{}", settings.web.port);
    Ok(())
}

/// Registers `rjsc start --config <config>` to run at every startup and starts it now.
/// Returns the absolute program and config paths used.
pub fn install_for(config: &Path) -> Result<(PathBuf, PathBuf)> {
    // Not `canonicalize`: on Windows it yields `\\?\` paths that schtasks mishandles.
    let config = std::path::absolute(config)?;
    let exe = std::env::current_exe()?;
    install(&exe, &config)?;
    Ok((exe, config))
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("Failed to run {:?}", cmd.get_program()))?;
    if !status.success() {
        anyhow::bail!("{:?} failed ({})", cmd.get_program(), status);
    }
    Ok(())
}

// ─── Linux: systemd user unit ─────────────────────────────

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn systemd_unit(exe: &Path, config: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=RJ45 Sound Card server\n\
         \n\
         [Service]\n\
         ExecStart=\"{}\" start --config \"{}\"\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         Environment=RUST_LOG=info\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe.display(),
        config.display()
    )
}

#[cfg(target_os = "linux")]
fn systemd_unit_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .map_or_else(|| home().map(|h| h.join(".config")), Ok)?;
    Ok(base.join("systemd/user").join(SYSTEMD_UNIT))
}

#[cfg(target_os = "linux")]
fn install(exe: &Path, config: &Path) -> Result<()> {
    let unit = systemd_unit_path()?;
    std::fs::create_dir_all(unit.parent().unwrap())?;
    std::fs::write(&unit, systemd_unit(exe, config))?;
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", "--now", SYSTEMD_UNIT]))?;

    // Lingering starts the user's services at boot, before anyone logs in.
    let user = std::env::var("USER").unwrap_or_default();
    if run(Command::new("loginctl").args(["enable-linger", &user])).is_err() {
        println!("Note: could not enable lingering, so the server starts when you log in.");
        println!(
            "To start it at boot instead, run: sudo loginctl enable-linger {}",
            user
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn disable() -> Result<()> {
    let _ = run(Command::new("systemctl").args(["--user", "disable", "--now", SYSTEMD_UNIT]));
    let unit = systemd_unit_path()?;
    if unit.exists() {
        std::fs::remove_file(&unit)?;
    }
    let _ = run(Command::new("systemctl").args(["--user", "daemon-reload"]));
    println!("Autostart disabled.");
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn status() -> Result<()> {
    let unit = systemd_unit_path()?;
    if !unit.exists() {
        println!("Autostart is disabled.");
        return Ok(());
    }
    println!("Autostart is enabled ({}).", unit.display());
    let _ = Command::new("systemctl")
        .args(["--user", "status", "--no-pager", SYSTEMD_UNIT])
        .status();
    Ok(())
}

// ─── macOS: launchd agent ─────────────────────────────────

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn launchd_plist(label: &str, exe: &Path, config: &Path, log: &Path) -> String {
    let arg = |p: &Path| format!("<string>{}</string>", xml_escape(&p.to_string_lossy()));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{}</string>
  <key>ProgramArguments</key>
  <array>{}<string>start</string><string>--config</string>{}</array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
  <key>StandardErrorPath</key>{}
</dict>
</plist>
"#,
        label,
        arg(exe),
        arg(config),
        arg(log)
    )
}

#[cfg(target_os = "macos")]
fn launchd_plist_path() -> Result<PathBuf> {
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", LAUNCHD_LABEL)))
}

#[cfg(target_os = "macos")]
fn install(exe: &Path, config: &Path) -> Result<()> {
    let plist = launchd_plist_path()?;
    let log = home()?.join("Library/Logs/rjsc.log");
    std::fs::create_dir_all(plist.parent().unwrap())?;
    // Reloading picks up a changed plist; unloading fails harmlessly the first time.
    let _ = Command::new("launchctl").arg("unload").arg(&plist).output();
    std::fs::write(&plist, launchd_plist(LAUNCHD_LABEL, exe, config, &log))?;
    run(Command::new("launchctl").args(["load", "-w"]).arg(&plist))?;
    println!("The server starts when you log in (enable automatic login to start it at boot).");
    println!("Logs: {}", log.display());
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn disable() -> Result<()> {
    let plist = launchd_plist_path()?;
    if plist.exists() {
        let _ = Command::new("launchctl").arg("unload").arg(&plist).output();
        std::fs::remove_file(&plist)?;
    }
    println!("Autostart disabled.");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn status() -> Result<()> {
    let plist = launchd_plist_path()?;
    if plist.exists() {
        println!("Autostart is enabled ({}).", plist.display());
    } else {
        println!("Autostart is disabled.");
    }
    Ok(())
}

// ─── Windows: scheduled task at logon ─────────────────────

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_task_command(exe: &Path, config: &Path) -> String {
    format!(
        "\"{}\" start --daemon --config \"{}\"",
        exe.display(),
        config.display()
    )
}

#[cfg(windows)]
fn install(exe: &Path, config: &Path) -> Result<()> {
    run(Command::new("schtasks").args([
        "/Create",
        "/F",
        "/TN",
        TASK_NAME,
        "/SC",
        "ONLOGON",
        "/RL",
        "LIMITED",
        "/TR",
        &windows_task_command(exe, config),
    ]))?;
    // Also start it now rather than at the next logon.
    let _ = Command::new("schtasks")
        .args(["/Run", "/TN", TASK_NAME])
        .output();
    println!("The server starts when you log in (enable automatic login to start it at boot).");
    Ok(())
}

#[cfg(windows)]
pub fn disable() -> Result<()> {
    let _ = Command::new("schtasks")
        .args(["/End", "/TN", TASK_NAME])
        .output();
    let _ = run(Command::new("schtasks").args(["/Delete", "/F", "/TN", TASK_NAME]));
    println!("Autostart disabled.");
    Ok(())
}

#[cfg(windows)]
pub fn status() -> Result<()> {
    let query = Command::new("schtasks")
        .args(["/Query", "/TN", TASK_NAME])
        .output()?;
    if query.status.success() {
        println!("Autostart is enabled (scheduled task \"{}\").", TASK_NAME);
    } else {
        println!("Autostart is disabled.");
    }
    Ok(())
}

// ─── Other platforms ──────────────────────────────────────

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn install(_exe: &Path, _config: &Path) -> Result<()> {
    anyhow::bail!("Autostart is not supported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn disable() -> Result<()> {
    anyhow::bail!("Autostart is not supported on this platform")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn status() -> Result<()> {
    anyhow::bail!("Autostart is not supported on this platform")
}

/// True when the console window belongs to this process alone, i.e. it was
/// double-clicked rather than run from a terminal.
#[cfg(windows)]
pub fn owns_console() -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetConsoleProcessList(list: *mut u32, count: u32) -> u32;
    }
    let mut list = [0u32; 2];
    // SAFETY: the buffer holds `list.len()` entries, as passed.
    unsafe { GetConsoleProcessList(list.as_mut_ptr(), list.len() as u32) == 1 }
}

/// Closes the console window when started by the logon task or a double-click.
#[cfg(windows)]
pub fn detach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn FreeConsole() -> i32;
    }
    // SAFETY: FreeConsole takes no arguments and only detaches this process from its console.
    unsafe {
        FreeConsole();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_systemd_unit_quotes_paths() {
        let unit = systemd_unit(
            Path::new("/opt/my apps/rjsc"),
            Path::new("/home/a/rjsc.toml"),
        );
        assert!(
            unit.contains("ExecStart=\"/opt/my apps/rjsc\" start --config \"/home/a/rjsc.toml\"")
        );
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn test_launchd_plist_escapes_paths() {
        let plist = launchd_plist(
            "com.example",
            Path::new("/Apps/R&D/rjsc"),
            Path::new("/c.toml"),
            Path::new("/l.log"),
        );
        assert!(plist.contains("<string>/Apps/R&amp;D/rjsc</string>"));
        assert!(plist
            .contains("<string>start</string><string>--config</string><string>/c.toml</string>"));
    }

    #[test]
    fn test_windows_task_command() {
        let cmd = windows_task_command(
            Path::new(r"C:\Program Files\rjsc\rjsc.exe"),
            Path::new(r"C:\Users\a\rjsc.toml"),
        );
        assert_eq!(
            cmd,
            r#""C:\Program Files\rjsc\rjsc.exe" start --daemon --config "C:\Users\a\rjsc.toml""#
        );
    }
}
