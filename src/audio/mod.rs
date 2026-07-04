pub mod capture;
pub mod format;
pub mod jitter;
pub mod playback;
pub mod virtual_device;

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};

pub fn list_devices() -> Result<()> {
    let host = cpal::default_host();
    println!("=== Available Audio Devices ===");
    println!("Host: {}", host.id().name());
    println!();

    for device in host.devices()? {
        let name = device.name()?;
        let input_confs = device.supported_input_configs()?.count();
        let output_confs = device.supported_output_configs()?.count();

        let default_in = host.default_input_device().and_then(|d| d.name().ok());
        let default_out = host.default_output_device().and_then(|d| d.name().ok());

        let is_default_in = default_in.as_deref() == Some(&name);
        let is_default_out = default_out.as_deref() == Some(&name);

        print!("  {} ", name);
        if is_default_in && is_default_out {
            print!("[default input/output]");
        } else if is_default_in {
            print!("[default input]");
        } else if is_default_out {
            print!("[default output]");
        }
        println!();
        println!(
            "       Input configs: {}, Output configs: {}",
            input_confs, output_confs
        );

        if let Ok(confs) = device.supported_input_configs() {
            for c in confs.take(2) {
                println!(
                    "       IN:  {} ch, {} Hz, {:?}",
                    c.channels(),
                    c.min_sample_rate().0,
                    c.sample_format()
                );
            }
        }
        if let Ok(confs) = device.supported_output_configs() {
            for c in confs.take(2) {
                println!(
                    "       OUT: {} ch, {} Hz, {:?}",
                    c.channels(),
                    c.min_sample_rate().0,
                    c.sample_format()
                );
            }
        }
        println!();
    }
    Ok(())
}

pub fn find_device(name: &str) -> Result<cpal::Device> {
    let host = cpal::default_host();
    host.devices()?
        .find(|d| {
            d.name()
                .map(|n| n.eq_ignore_ascii_case(name) || n.contains(name) || name.contains(&n))
                .unwrap_or(false)
        })
        .ok_or_else(|| anyhow::anyhow!("Audio device '{}' not found", name))
}

pub fn find_device_by_index(index: usize) -> Result<cpal::Device> {
    let host = cpal::default_host();
    host.devices()?
        .enumerate()
        .find(|(i, _)| *i == index)
        .map(|(_, d)| d)
        .ok_or_else(|| anyhow::anyhow!("Device at index {} not found", index))
}

pub fn default_output_device() -> Result<cpal::Device> {
    cpal::default_host()
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("No default output device found"))
}

pub fn default_input_device() -> Result<cpal::Device> {
    cpal::default_host()
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("No default input device found"))
}
