use std::sync::Arc;

use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam::channel::{self, Sender};

use crate::audio::format::AudioFormat;
use crate::audio::playout::{Playout, PlayoutStats};

pub struct AudioPlayback {
    stream: cpal::Stream,
    tx: Sender<Vec<f32>>,
    format: AudioFormat,
    stats: Arc<PlayoutStats>,
}

impl AudioPlayback {
    /// `latency_ms` is the playout buffer target (raised automatically when the
    /// packet and callback sizes need more); see `audio::playout`.
    pub fn new(device: &cpal::Device, fmt: &AudioFormat, latency_ms: u32) -> Result<Self> {
        let channels = fmt.channels.max(1);
        // Packets are only queued here; the callback moves them into the playout buffer.
        let (tx, rx) = channel::bounded::<Vec<f32>>(256);
        let stats = Arc::new(PlayoutStats::default());

        let stream = super::build_with_buffer_size(fmt, |config, health| {
            let rx = rx.clone();
            let mut playout =
                Playout::with_stats(channels, fmt.sample_rate, latency_ms, stats.clone());
            device.build_output_stream::<f32, _, _>(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    while let Ok(buf) = rx.try_recv() {
                        playout.push(&buf);
                    }
                    playout.fill(data);
                },
                super::stream_error_logger("playback", health),
                None,
            )
        })?;

        Ok(Self {
            stream,
            tx,
            format: AudioFormat::new(channels, fmt.sample_rate, fmt.buffer_frames),
            stats,
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

    /// Buffer level, clock drift and underruns of the incoming audio.
    pub fn stats(&self) -> Arc<PlayoutStats> {
        self.stats.clone()
    }
}
