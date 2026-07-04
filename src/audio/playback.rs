use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam::channel::{self, Sender};

use crate::audio::format::AudioFormat;

pub struct AudioPlayback {
    stream: cpal::Stream,
    tx: Sender<Vec<f32>>,
    format: AudioFormat,
}

impl AudioPlayback {
    pub fn new(device: &cpal::Device, fmt: &AudioFormat) -> Result<Self> {
        let config = cpal::StreamConfig {
            channels: fmt.channels.max(1),
            sample_rate: cpal::SampleRate(fmt.sample_rate),
            buffer_size: cpal::BufferSize::Default,
        };

        let actual_channels = config.channels;
        let (tx, rx) = channel::bounded::<Vec<f32>>(32);

        let stream = device.build_output_stream::<f32, _, _>(
            &config,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                if let Ok(buf) = rx.try_recv() {
                    let copy_len = data.len().min(buf.len());
                    data[..copy_len].copy_from_slice(&buf[..copy_len]);
                    if copy_len < data.len() {
                        data[copy_len..].fill(0.0);
                    }
                } else {
                    data.fill(0.0);
                }
            },
            |err| {
                log::error!("Audio playback stream error: {}", err);
            },
            None,
        )?;

        Ok(Self {
            stream,
            tx,
            format: AudioFormat::new(actual_channels, config.sample_rate.0, fmt.buffer_frames),
        })
    }

    pub fn start(&self) -> Result<()> {
        self.stream.play()?;
        log::info!(
            "Audio playback started ({} ch @ {} Hz)",
            self.format.channels,
            self.format.sample_rate
        );
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        self.stream.pause()?;
        log::info!("Audio playback stopped");
        Ok(())
    }

    pub fn sender(&self) -> Sender<Vec<f32>> {
        self.tx.clone()
    }

    pub fn format(&self) -> &AudioFormat {
        &self.format
    }
}
