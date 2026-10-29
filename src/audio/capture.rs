use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam::channel::{self, Receiver, TrySendError};

use crate::audio::format::AudioFormat;

pub struct AudioCapture {
    stream: cpal::Stream,
    rx: Receiver<Vec<f32>>,
    format: AudioFormat,
}

impl AudioCapture {
    pub fn new(device: &cpal::Device, fmt: &AudioFormat) -> Result<Self> {
        let actual_channels = fmt.channels.max(1);
        let (tx, rx) = channel::bounded::<Vec<f32>>(32);
        let warned = Arc::new(AtomicBool::new(false));

        let stream = super::build_with_buffer_size(fmt, |config, health| {
            let tx = tx.clone();
            let warned = warned.clone();
            device.build_input_stream::<f32, _, _>(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if data.len() < actual_channels as usize {
                        return;
                    }
                    let buf = data.to_vec();
                    if let Err(TrySendError::Full(_)) = tx.try_send(buf) {
                        // Once: this runs in the real-time audio thread.
                        if !warned.swap(true, Ordering::Relaxed) {
                            log::warn!("Capture buffer full, dropping audio (network too slow?)");
                        }
                    }
                },
                super::stream_error_logger("capture", health),
                None,
            )
        })?;

        Ok(Self {
            stream,
            rx,
            format: AudioFormat::new(actual_channels, fmt.sample_rate, fmt.buffer_frames),
        })
    }

    pub fn start(&self) -> Result<()> {
        self.stream.play()?;
        log::info!(
            "Audio capture started ({} ch @ {} Hz)",
            self.format.channels,
            self.format.sample_rate
        );
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.stream.pause()?;
        log::info!("Audio capture stopped");
        Ok(())
    }

    pub fn receiver(&self) -> Receiver<Vec<f32>> {
        self.rx.clone()
    }

    pub fn format(&self) -> &AudioFormat {
        &self.format
    }
}
