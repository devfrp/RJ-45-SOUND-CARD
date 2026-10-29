pub mod capture;
pub mod format;
pub mod jitter;
pub mod playback;
pub mod playout;
pub mod virtual_device;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::audio::format::AudioFormat;
use crate::net::control::DeviceInfo;

/// How long a stream with a fixed buffer size is tried before trusting it.
const BUFFER_PROBE: Duration = Duration::from_millis(200);

/// Shared between a stream's error callback and `build_with_buffer_size`.
#[derive(Default)]
struct StreamHealth {
    failed: AtomicBool,
    /// Errors while trying a fixed buffer size are expected, not worth an ERROR.
    probing: AtomicBool,
}

/// Error callback for a stream: logs the first error only (a broken stream can
/// report thousands per second) and records that the stream failed.
fn stream_error_logger(
    what: &'static str,
    health: Arc<StreamHealth>,
) -> impl FnMut(cpal::StreamError) {
    move |err| {
        let repeated = health.failed.swap(true, Ordering::SeqCst);
        if repeated || health.probing.load(Ordering::SeqCst) {
            log::debug!("Audio {} stream error: {}", what, err);
        } else {
            log::error!("Audio {} stream error: {}", what, err);
        }
    }
}

/// Builds a stream whose device buffer is `fmt.buffer_frames` (the latency the
/// user chose), falling back to the device's default size if it is refused.
/// Some backends accept a fixed size and then fail while running, so the
/// stream is also played briefly and dropped if it reports an error.
/// `build` gets the config and the health its error callback must report to.
fn build_with_buffer_size(
    fmt: &AudioFormat,
    mut build: impl FnMut(
        &cpal::StreamConfig,
        Arc<StreamHealth>,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>,
) -> Result<cpal::Stream> {
    let mut config = cpal::StreamConfig {
        channels: fmt.channels.max(1),
        sample_rate: cpal::SampleRate(fmt.sample_rate),
        buffer_size: cpal::BufferSize::Fixed(fmt.buffer_frames as u32),
    };
    let health = Arc::new(StreamHealth {
        probing: AtomicBool::new(true),
        ..Default::default()
    });
    match build(&config, health.clone()) {
        Ok(stream) if runs_cleanly(&stream, &health) => return Ok(stream),
        Ok(_) => log::info!(
            "A {}-frame buffer fails on this device, using its default size",
            fmt.buffer_frames
        ),
        Err(e) => log::info!(
            "Device refused a {}-frame buffer ({}), using its default size",
            fmt.buffer_frames,
            e
        ),
    }
    config.buffer_size = cpal::BufferSize::Default;
    Ok(build(&config, Arc::default())?)
}

fn runs_cleanly(stream: &cpal::Stream, health: &StreamHealth) -> bool {
    if stream.play().is_err() {
        return false;
    }
    std::thread::sleep(BUFFER_PROBE);
    let _ = stream.pause();
    health.probing.store(false, Ordering::SeqCst);
    !health.failed.load(Ordering::SeqCst)
}

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

/// Lists this machine's audio devices with their channel counts.
pub fn device_infos() -> Vec<DeviceInfo> {
    let host = cpal::default_host();
    let mut devices = Vec::new();
    if let Ok(dev_iter) = host.devices() {
        for (i, device) in dev_iter.enumerate() {
            let name = device.name().unwrap_or_else(|_| format!("Device {}", i));
            let input_configs = device
                .supported_input_configs()
                .ok()
                .and_then(|mut c| c.next())
                .map(|c| c.channels())
                .unwrap_or(0);
            let output_configs = device
                .supported_output_configs()
                .ok()
                .and_then(|mut c| c.next())
                .map(|c| c.channels())
                .unwrap_or(0);

            let mut sample_rates = Vec::new();
            if let Ok(confs) = device.supported_input_configs() {
                for c in confs {
                    for sr in [c.min_sample_rate().0, c.max_sample_rate().0] {
                        if !sample_rates.contains(&sr) {
                            sample_rates.push(sr);
                        }
                    }
                }
            }
            sample_rates.sort();
            sample_rates.dedup();

            devices.push(DeviceInfo {
                index: i,
                name,
                input_channels: input_configs,
                output_channels: output_configs,
                sample_rates,
            });
        }
    }
    devices
}
