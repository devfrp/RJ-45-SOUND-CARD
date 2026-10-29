//! Installs the client's virtual audio driver from the laptop's local page:
//! runs the setup script embedded in the binary through the OS's own
//! administrator prompt (UAC, macOS password dialog, polkit).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
const SCRIPT: (&str, &str) = ("linux_setup.sh", include_str!("../scripts/linux_setup.sh"));
#[cfg(target_os = "macos")]
const SCRIPT: (&str, &str) = ("mac_setup.sh", include_str!("../scripts/mac_setup.sh"));
#[cfg(windows)]
const SCRIPT: (&str, &str) = (
    "windows_setup.ps1",
    include_str!("../scripts/windows_setup.ps1"),
);

/// Where to get the driver by hand if the automatic install fails.
pub const MANUAL_URL: &str = if cfg!(windows) {
    "https://vb-audio.com/Cable/"
} else if cfg!(target_os = "macos") {
    "https://existential.audio/blackhole/"
} else {
    "https://www.alsa-project.org/wiki/Matrix:Module-aloop"
};

/// Name of the virtual audio device currently present, if any.
pub fn detect() -> Option<String> {
    crate::audio::virtual_device::detect_virtual_device()
}

/// Blocks until the install script finishes (including the admin prompt).
pub fn install() -> Result<()> {
    let dir = std::env::temp_dir().join(format!("rjsc-driver-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let script = write_script(&dir)?;
    let result = run_elevated(&script);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn write_script(dir: &Path) -> Result<PathBuf> {
    let (name, content) = SCRIPT;
    let path = dir.join(name);
    std::fs::write(&path, content)?;
    Ok(path)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn write_script(_dir: &Path) -> Result<PathBuf> {
    anyhow::bail!("No virtual driver installer for this platform")
}

fn check(cmd: &mut Command) -> Result<()> {
    let output = cmd
        .output()
        .with_context(|| format!("Failed to run {:?}", cmd.get_program()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Installer failed ({}): {}", output.status, stderr.trim());
    }
    Ok(())
}

#[cfg(unix)]
fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn run_elevated(script: &Path) -> Result<()> {
    let mut cmd = if is_root() {
        Command::new("/bin/bash")
    } else {
        let mut cmd = Command::new("pkexec");
        cmd.arg("/bin/bash");
        cmd
    };
    check(cmd.arg(script).arg("install"))
}

#[cfg(target_os = "macos")]
fn run_elevated(script: &Path) -> Result<()> {
    if is_root() {
        return check(Command::new("/bin/bash").arg(script).arg("install"));
    }
    let apple_script = format!(
        "do shell script \"/bin/bash \" & quoted form of \"{}\" & \" install\" with administrator privileges",
        script.display().to_string().replace('\\', "\\\\").replace('"', "\\\"")
    );
    check(Command::new("osascript").arg("-e").arg(apple_script))
}

#[cfg(windows)]
fn run_elevated(script: &Path) -> Result<()> {
    // Start-Process -Verb RunAs shows the UAC prompt; -PassThru gives back the exit code.
    let script = script.display().to_string().replace('\'', "''");
    let command = format!(
        "$p = Start-Process -FilePath powershell -Verb RunAs -Wait -PassThru \
         -ArgumentList '-NoProfile -ExecutionPolicy Bypass -File \"{}\" install'; exit $p.ExitCode",
        script
    );
    check(Command::new("powershell").args(["-NoProfile", "-Command", &command]))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn run_elevated(_script: &Path) -> Result<()> {
    anyhow::bail!("No virtual driver installer for this platform")
}
