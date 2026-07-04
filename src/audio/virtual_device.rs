use cpal::traits::{DeviceTrait, HostTrait};

pub fn detect_virtual_device() -> Option<String> {
    let host = cpal::default_host();
    let host_name = host.id().name();

    let devices = match host.devices() {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Failed to enumerate audio devices: {}", e);
            return None;
        }
    };

    for device in devices {
        let name = match device.name() {
            Ok(n) => n,
            Err(_) => continue,
        };

        if is_virtual_device(&name) {
            log::info!("Detected virtual audio device on {}: {}", host_name, name);
            return Some(name);
        }
    }

    log::info!("No virtual audio device detected on {}", host_name);
    None
}

fn is_virtual_device(name: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        if name.contains("Loopback") {
            return true;
        }
        let virtual_keywords = ["RJ45", "Virtual", "Null", "Dummy", "Monitor"];
        for kw in &virtual_keywords {
            if name.contains(kw) {
                return true;
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if name.contains("BlackHole") {
            return true;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if name.contains("CABLE") || name.contains("VB-Audio") || name.contains("Virtual Cable") {
            return true;
        }
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let virtual_keywords = [
            "Loopback",
            "BlackHole",
            "CABLE",
            "VB-Audio",
            "Virtual Cable",
            "Virtual",
            "Null",
            "Dummy",
        ];
        for kw in &virtual_keywords {
            if name.contains(kw) {
                return true;
            }
        }
    }

    false
}
