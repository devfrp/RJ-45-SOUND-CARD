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
        let config = cpal::StreamConfig {
            channels: fmt.channels.max(1),
            sample_rate: cpal::SampleRate(fmt.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let actual_channels = config.channels;
        let (tx, rx) = channel::bounded::<Vec<f32>>(32);

        let stream = device.build_input_stream::<f32, _, _>(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if data.len() < actual_channels as usize {
                    return;
                }
                let buf = data.to_vec();
                if let Err(TrySendError::Full(_)) = tx.try_send(buf) {
                    log::warn!("Capture buffer full, dropping frame");
                }
            },
            |err| {
                log::error!("Audio capture stream error: {}", err);
            },
            None,
        )?;

        Ok(Self {
            stream,
            rx,
            format: AudioFormat::new(actual_channels, config.sample_rate.0, fmt.buffer_frames),
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
