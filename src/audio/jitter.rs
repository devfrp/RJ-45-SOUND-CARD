use std::collections::BTreeMap;
use std::time::Duration;

pub struct JitterBuffer {
    packets: BTreeMap<u64, Vec<f32>>,
    max_latency: Duration,
    next_sequence: u64,
    sample_rate: u32,
}

impl JitterBuffer {
    pub fn new(max_latency_ms: u64, sample_rate: u32) -> Self {
        Self {
            packets: BTreeMap::new(),
            max_latency: Duration::from_millis(max_latency_ms),
            next_sequence: 0,
            sample_rate,
        }
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
            let gap_duration = Duration::from_secs_f64(gap as f64 / self.sample_rate as f64);
            if gap_duration > self.max_latency {
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
        let mut jb = JitterBuffer::new(50, 48000);
        let samples = vec![0.5f32; 512];
        jb.push(0, samples.clone());
        assert_eq!(jb.pop(), Some(samples));
    }

    #[test]
    fn test_reordering() {
        let mut jb = JitterBuffer::new(50, 48000);
        jb.push(1, vec![1.0; 256]);
        jb.push(0, vec![0.5; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));
        assert_eq!(jb.pop(), Some(vec![1.0; 256]));
    }

    #[test]
    fn test_skip_late_packets() {
        let mut jb = JitterBuffer::new(1, 48000);
        jb.push(0, vec![0.5; 256]);
        jb.push(60, vec![0.8; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(jb.pop(), Some(vec![0.8; 256]));
    }

    #[test]
    fn test_discard_old_duplicates() {
        let mut jb = JitterBuffer::new(50, 48000);
        jb.push(0, vec![0.5; 256]);
        assert_eq!(jb.pop(), Some(vec![0.5; 256]));
        jb.push(0, vec![1.0; 256]);
        assert!(jb.is_empty());
    }
}
