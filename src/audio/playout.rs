//! Playout buffer for audio arriving from the network, read by the playback
//! callback. Two sound cards never run at exactly the same rate (crystals
//! differ by tens of ppm), so a plain FIFO slowly empties (dropouts) or fills
//! (growing latency). This one reads through a resampler whose ratio a PI
//! controller nudges to keep the buffered audio at a target latency.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::Arc;

/// Controller gains, in units of "ratio change per second of buffer error".
/// Kp = 0.1 corrects a fill error with a ~10 s time constant (1 ms off → 100 ppm);
/// Ki = 0.004 removes a steady drift with ζ ≈ 0.8, so it settles without oscillating.
const KP: f64 = 0.1;
const KI: f64 = 0.004;
/// Largest speed correction (±0.5 %), far above real crystal drift.
const MAX_CORRECTION: f64 = 0.005;
/// Time constant of the averaged fill level, so packet jitter doesn't drive the ratio.
const FILL_SMOOTHING_SECS: f64 = 0.5;
/// Beyond this many times the target, drop the excess at once (e.g. after a stall).
const OVERFLOW_FACTOR: usize = 4;

/// Live figures, readable from other threads (for logs and the panel).
#[derive(Debug, Default)]
pub struct PlayoutStats {
    buffered_frames: AtomicU32,
    target_frames: AtomicU32,
    drift_ppm: AtomicI32,
    underruns: AtomicU32,
    playing: AtomicU32,
}

impl PlayoutStats {
    pub fn buffered_ms(&self, sample_rate: u32) -> f64 {
        self.buffered_frames.load(Ordering::Relaxed) as f64 * 1000.0 / sample_rate as f64
    }

    pub fn target_ms(&self, sample_rate: u32) -> f64 {
        self.target_frames.load(Ordering::Relaxed) as f64 * 1000.0 / sample_rate as f64
    }

    /// Positive when the sender's clock runs faster than this sound card's.
    pub fn drift_ppm(&self) -> i32 {
        self.drift_ppm.load(Ordering::Relaxed)
    }

    pub fn underruns(&self) -> u32 {
        self.underruns.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed) != 0
    }

    /// One-line summary for the logs.
    pub fn summary(&self, sample_rate: u32) -> String {
        format!(
            "{:.1} ms buffered (target {:.1} ms), clock drift {:+} ppm, {} underrun(s)",
            self.buffered_ms(sample_rate),
            self.target_ms(sample_rate),
            self.drift_ppm(),
            self.underruns()
        )
    }
}

pub struct Playout {
    channels: usize,
    sample_rate: f64,
    /// Interleaved samples; the first frame is history for the interpolator.
    fifo: VecDeque<f32>,
    /// Read position in frames from the front of `fifo`.
    pos: f64,
    integral: f64,
    fill_avg: f64,
    min_target: usize,
    target: usize,
    largest_packet: usize,
    largest_callback: usize,
    playing: bool,
    stats: Arc<PlayoutStats>,
}

impl Playout {
    pub fn new(channels: u16, sample_rate: u32, target_latency_ms: u32) -> Self {
        Self::with_stats(channels, sample_rate, target_latency_ms, Arc::default())
    }

    /// Like `new`, reporting into an existing `stats`.
    pub fn with_stats(
        channels: u16,
        sample_rate: u32,
        target_latency_ms: u32,
        stats: Arc<PlayoutStats>,
    ) -> Self {
        let channels = channels.max(1) as usize;
        let min_target = (sample_rate as u64 * target_latency_ms as u64 / 1000).max(1) as usize;
        Self {
            channels,
            sample_rate: sample_rate as f64,
            // One second up front, so the real-time callback rarely allocates.
            fifo: VecDeque::with_capacity(sample_rate as usize * channels),
            pos: 0.0,
            integral: 0.0,
            fill_avg: 0.0,
            min_target,
            target: min_target,
            largest_packet: 0,
            largest_callback: 0,
            playing: false,
            stats,
        }
    }

    pub fn stats(&self) -> Arc<PlayoutStats> {
        self.stats.clone()
    }

    fn frames(&self) -> usize {
        self.fifo.len() / self.channels
    }

    /// Frames not yet played.
    fn pending(&self) -> f64 {
        self.frames() as f64 - self.pos
    }

    /// Appends interleaved samples received from the network.
    pub fn push(&mut self, samples: &[f32]) {
        let whole = samples.len() - samples.len() % self.channels;
        self.largest_packet = self.largest_packet.max(whole / self.channels);
        self.fifo.extend(&samples[..whole]);
    }

    /// Fills one playback buffer, with silence while (re)buffering.
    pub fn fill(&mut self, out: &mut [f32]) {
        let ch = self.channels;
        let out_frames = out.len() / ch;
        self.largest_callback = self.largest_callback.max(out_frames);
        // Never aim below what one callback plus 1.5 packets need. Packets arrive whole, so
        // with drift the level measured at each callback holds still, then drops by a full
        // packet at once; the extra half packet absorbs that step while the controller reacts.
        self.target = self
            .min_target
            .max(self.largest_callback + self.largest_packet * 3 / 2 + 8);
        self.stats
            .target_frames
            .store(self.target as u32, Ordering::Relaxed);

        if !self.playing {
            if self.frames() < self.target + 1 {
                out.fill(0.0);
                self.publish();
                return;
            }
            // Start with one frame of history and exactly the target ahead.
            let excess = self.frames() - self.target - 1;
            self.fifo.drain(..excess * ch);
            self.pos = 1.0;
            self.fill_avg = self.target as f64;
            self.playing = true;
        }

        let ratio = self.update_ratio(out_frames);

        if self.pending() > (self.target * OVERFLOW_FACTOR) as f64 {
            let excess = (self.pending() - self.target as f64) as usize;
            log::debug!("Playout overflow, dropping {} frames", excess);
            self.fifo.drain(..excess * ch);
            self.fill_avg = self.target as f64;
        }

        for i in 0..out_frames {
            let idx = self.pos as usize;
            // Cubic interpolation reads frames idx-1 ..= idx+2.
            if idx + 2 >= self.frames() {
                out[i * ch..].fill(0.0);
                self.playing = false;
                self.stats.underruns.fetch_add(1, Ordering::Relaxed);
                log::debug!("Playout underrun, rebuffering");
                break;
            }
            let t = (self.pos - idx as f64) as f32;
            for c in 0..ch {
                let at = |k: usize| self.fifo[k * ch + c];
                out[i * ch + c] = hermite(at(idx - 1), at(idx), at(idx + 1), at(idx + 2), t);
            }
            self.pos += ratio;
        }

        // Keep one frame of history before the read position.
        let consumed = (self.pos as usize).saturating_sub(1);
        self.fifo.drain(..consumed * ch);
        self.pos -= consumed as f64;
        self.publish();
    }

    /// PI controller on the averaged fill level; returns the read speed.
    fn update_ratio(&mut self, out_frames: usize) -> f64 {
        let dt = out_frames as f64 / self.sample_rate;
        let alpha = (dt / FILL_SMOOTHING_SECS).min(1.0);
        self.fill_avg += alpha * (self.pending() - self.fill_avg);
        let error_secs = (self.fill_avg - self.target as f64) / self.sample_rate;
        self.integral =
            (self.integral + KI * error_secs * dt).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        let correction = (KP * error_secs + self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION);
        self.stats
            .drift_ppm
            .store((correction * 1e6).round() as i32, Ordering::Relaxed);
        1.0 + correction
    }

    fn publish(&self) {
        // The smoothed level the controller regulates (measured before each callback).
        let buffered = if self.playing {
            self.fill_avg
        } else {
            self.pending()
        };
        self.stats
            .buffered_frames
            .store(buffered.max(0.0) as u32, Ordering::Relaxed);
        self.stats
            .playing
            .store(self.playing as u32, Ordering::Relaxed);
    }
}

/// Catmull-Rom interpolation between `x1` and `x2` (t in 0..1).
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// Feeds `packet` frames per step at `in_rate`, plays `callback` frames per step at RATE.
    fn simulate(playout: &mut Playout, in_rate: f64, packet: usize, callback: usize, secs: f64) {
        simulate_with_jitter(playout, in_rate, packet, callback, secs, 0);
    }

    /// Like `simulate`, each packet arriving up to `jitter` frames late (random, seeded).
    fn simulate_with_jitter(
        playout: &mut Playout,
        in_rate: f64,
        packet: usize,
        callback: usize,
        secs: f64,
        jitter: usize,
    ) {
        let mut rng = fastrand::Rng::with_seed(7);
        let ch = playout.channels;
        let mut delay = 0.0;
        let mut sent = 0.0;
        let mut played = 0usize;
        let mut out = vec![0.0f32; callback * ch];
        let mut phase = 0.0f32;
        while (played as f64) < secs * RATE as f64 {
            // Deliver every whole packet the sender has finished by now.
            let due = played as f64 * in_rate / RATE as f64;
            while sent + packet as f64 + delay <= due {
                delay = rng.usize(..=jitter) as f64;
                let samples: Vec<f32> = (0..packet * ch)
                    .map(|_| {
                        phase += 0.01;
                        phase.sin()
                    })
                    .collect();
                playout.push(&samples);
                sent += packet as f64;
            }
            playout.fill(&mut out);
            played += callback;
        }
    }

    #[test]
    fn test_steady_state_keeps_target_latency() {
        let mut playout = Playout::new(1, RATE, 10);
        simulate(&mut playout, RATE as f64, 256, 256, 60.0);
        let stats = playout.stats();
        assert!(stats.is_playing());
        assert_eq!(stats.underruns(), 0);
        let target = stats.target_ms(RATE);
        assert!((stats.buffered_ms(RATE) - target).abs() < 3.0);
    }

    #[test]
    fn test_compensates_clock_drift() {
        // Sender 200 ppm fast: without correction the buffer would grow by 24 ms in 120 s.
        let mut playout = Playout::new(1, RATE, 10);
        simulate(&mut playout, RATE as f64 * 1.0002, 256, 256, 120.0);
        let stats = playout.stats();
        assert_eq!(stats.underruns(), 0);
        assert!(
            (stats.drift_ppm() - 200).abs() < 40,
            "{}",
            stats.drift_ppm()
        );
        assert!((stats.buffered_ms(RATE) - stats.target_ms(RATE)).abs() < 3.0);
    }

    #[test]
    fn test_compensates_slow_sender() {
        let mut playout = Playout::new(1, RATE, 10);
        simulate(&mut playout, RATE as f64 * 0.9998, 256, 256, 120.0);
        let stats = playout.stats();
        assert_eq!(stats.underruns(), 0);
        assert!(
            (stats.drift_ppm() + 200).abs() < 40,
            "{}",
            stats.drift_ppm()
        );
    }

    #[test]
    fn test_mismatched_packet_and_callback_sizes() {
        // Sender packets of 480 frames, playback callbacks of 128: no chopping, no underruns.
        let mut playout = Playout::new(2, RATE, 5);
        simulate(&mut playout, RATE as f64, 480, 128, 20.0);
        assert_eq!(playout.stats().underruns(), 0);
        // The target grew to fit 1.5 packets plus one callback.
        assert!(playout.stats().target_ms(RATE) >= (720.0 + 128.0) * 1000.0 / RATE as f64);
    }

    #[test]
    fn test_drift_with_jitter_and_odd_sizes() {
        // 300-frame packets, 2 ms of network jitter, 128-frame callbacks, sender 150 ppm slow.
        let mut playout = Playout::new(2, RATE, 10);
        simulate_with_jitter(&mut playout, RATE as f64 * 0.99985, 300, 128, 120.0, 96);
        let stats = playout.stats();
        assert_eq!(stats.underruns(), 0);
        assert!(
            (stats.drift_ppm() + 150).abs() < 60,
            "{}",
            stats.drift_ppm()
        );
    }

    #[test]
    fn test_silence_until_buffered_then_continuous() {
        let mut playout = Playout::new(1, RATE, 10);
        let mut out = vec![1.0f32; 64];
        playout.fill(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));

        // A constant signal must come out unchanged once playing.
        for _ in 0..8 {
            playout.push(&[0.25; 256]);
        }
        playout.fill(&mut out);
        assert!(out.iter().all(|&s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn test_hermite_passes_through_points() {
        assert_eq!(hermite(0.0, 1.0, 2.0, 3.0, 0.0), 1.0);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 1.0) - 2.0).abs() < 1e-6);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.5) - 1.5).abs() < 1e-6);
    }
}
