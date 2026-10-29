use std::collections::BTreeMap;
use std::time::Duration;

/// Puts packets back in order. A missing packet is waited for until the audio
/// queued behind it exceeds `max_latency`, then skipped.
pub struct JitterBuffer {
    packets: BTreeMap<u64, Vec<f32>>,
    max_latency: Duration,
    next_sequence: u64,
    sample_rate: u32,
    channels: u16,
}

impl JitterBuffer {
    pub fn new(max_latency_ms: u64, sample_rate: u32, channels: u16) -> Self {
        Self {
            packets: BTreeMap::new(),
            max_latency: Duration::from_millis(max_latency_ms),
            next_sequence: 0,
            sample_rate,
            channels: channels.max(1),
        }
    }

    /// Duration of the audio waiting behind a missing packet.
    fn queued_duration(&self) -> Duration {
        let samples: usize = self.packets.values().map(Vec::len).sum();
        let frames = samples / self.channels as usize;
        Duration::from_secs_f64(frames as f64 / self.sample_rate as f64)
    }

    pub fn push(&mut self, sequence: u64, samples: Vec<f32>) {
        if sequence < self.next_sequence {
            return;
        }
        self.packets.insert(sequence, samples);
    }

    pub fn pop(&mut self) -> Option<Vec<f32>> {
        if let Some((&seq, _)) = self.packets.first_key_value() {
            if seq == self.next_sequence {
                let samples = self.packets.remove(&seq).unwrap();
                self.next_sequence = seq.wrapping_add(1);
                return Some(samples);
            }
            if seq < self.next_sequence {
                self.packets.pop_first();
                return None;
            }
        }

        if let Some(first_seq) = self.packets.first_key_value().map(|(k, _)| *k) {
            let gap = first_seq.wrapping_sub(self.next_sequence);
            if self.queued_duration() > self.max_latency {
                log::debug!(
                    "Jitter buffer: skipping {} missing packets (seq {} -> {})",
                    gap,
                    self.next_sequence,
                    first_seq
                );
                let samples = self.packets.remove(&first_seq).unwrap();
                self.next_sequence = first_seq.wrapping_add(1);
                return Some(samples);
            }
        }

        None
    }

    pub fn drain_available(&mut self) -> Vec<Vec<f32>> {
        let mut result = Vec::new();
        while let Some(samples) = self.pop() {
            result.push(samples);
        }
        result
    }

    pub fn len(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_in_order_delivery() {
        let mut jb = JitterBuffer::new(50, 48000, 2);
        let samples = vec![0.5f32; 512];
        jb.push(0, samples.clone());
        assert_eq!(jb.pop(), Some(samples));
    }

    #[test]
    fn test_reordering() {
        let mut jb = JitterBuffer::new(50, 48000, 2);
        jb.push(1, vec![1.0; 256]);
        jb.push(0, vec![0.5; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));
        assert_eq!(jb.pop(), Some(vec![1.0; 256]));
    }

    #[test]
    fn test_skip_late_packets() {
        // 10 ms window, stereo: each 256-sample packet is 128 frames = 2.7 ms.
        let mut jb = JitterBuffer::new(10, 48000, 2);
        jb.push(0, vec![0.5; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));

        // Packet 1 is lost: wait while less than 10 ms is queued behind it...
        for seq in 2..5 {
            jb.push(seq, vec![seq as f32; 256]);
        }
        assert_eq!(jb.pop(), None);
        // ...then skip it once more than 10 ms is waiting.
        jb.push(5, vec![5.0; 256]);
        assert_eq!(jb.pop(), Some(vec![2.0; 256]));
        assert_eq!(jb.pop(), Some(vec![3.0; 256]));
    }

    #[test]
    fn test_discard_old_duplicates() {
        let mut jb = JitterBuffer::new(50, 48000, 2);
        jb.push(0, vec![0.5; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));
        jb.push(0, vec![1.0; 256]);
        assert!(jb.is_empty());
    }
}
