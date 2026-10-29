use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SampleFormat {
    #[default]
    F32,
    I16,
    I24,
    I32,
}

impl SampleFormat {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::F32),
            1 => Some(Self::I16),
            2 => Some(Self::I24),
            3 => Some(Self::I32),
            _ => None,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            Self::F32 => 0,
            Self::I16 => 1,
            Self::I24 => 2,
            Self::I32 => 3,
        }
    }

    pub fn bytes_per_sample(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::I16 => 2,
            Self::I24 => 3,
            Self::I32 => 4,
        }
    }
}

/// Converts interleaved audio between channel counts, channel N to channel N:
/// extra channels are dropped, missing ones are silent.
pub fn remap_channels(samples: Vec<f32>, from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 || to == 0 {
        return samples;
    }
    let frames = samples.len() / from;
    let mut out = vec![0.0; frames * to];
    let shared = from.min(to);
    for (src, dst) in samples.chunks_exact(from).zip(out.chunks_exact_mut(to)) {
        dst[..shared].copy_from_slice(&src[..shared]);
    }
    out
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AudioFormat {
    pub channels: u16,
    pub sample_rate: u32,
    pub buffer_frames: usize,
    pub sample_format: SampleFormat,
}

impl AudioFormat {
    pub fn new(channels: u16, sample_rate: u32, buffer_frames: usize) -> Self {
        Self {
            channels,
            sample_rate,
            buffer_frames,
            sample_format: SampleFormat::F32,
        }
    }

    pub fn with_format(
        channels: u16,
        sample_rate: u32,
        buffer_frames: usize,
        sample_format: SampleFormat,
    ) -> Self {
        Self {
            channels,
            sample_rate,
            buffer_frames,
            sample_format,
        }
    }

    pub fn bytes_per_sample(&self) -> usize {
        self.sample_format.bytes_per_sample()
    }

    pub fn bytes_per_frame(&self) -> usize {
        self.channels as usize * self.bytes_per_sample()
    }

    pub fn buffer_bytes(&self) -> usize {
        self.buffer_frames * self.bytes_per_frame()
    }

    pub fn bitrate_mbps(&self) -> f64 {
        self.channels as f64 * self.sample_rate as f64 * self.bytes_per_sample() as f64 * 8.0
            / 1_000_000.0
    }
}

impl Default for AudioFormat {
    fn default() -> Self {
        Self {
            channels: 2,
            sample_rate: 48000,
            buffer_frames: 256,
            sample_format: SampleFormat::F32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remap_channels() {
        // 3 channels -> 2: third dropped.
        assert_eq!(
            remap_channels(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3, 2),
            vec![1.0, 2.0, 4.0, 5.0]
        );
        // 2 channels -> 4: extra ones silent.
        assert_eq!(
            remap_channels(vec![1.0, 2.0, 3.0, 4.0], 2, 4),
            vec![1.0, 2.0, 0.0, 0.0, 3.0, 4.0, 0.0, 0.0]
        );
        assert_eq!(remap_channels(vec![1.0, 2.0], 2, 2), vec![1.0, 2.0]);
    }
}
