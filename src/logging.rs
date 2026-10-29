//! Logger setup. `env_logger` writes to stderr as usual; records from this
//! crate can also be mirrored into a [`LogBuffer`] shown by the web panel.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

const CAPACITY: usize = 500;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub seq: u64,
    /// Unix time in milliseconds.
    pub time_ms: u64,
    pub level: String,
    pub message: String,
}

#[derive(Default)]
pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
}

impl LogBuffer {
    fn push(&self, level: log::Level, message: String) {
        let time_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let seq = entries.back().map_or(1, |last| last.seq + 1);
        if entries.len() == CAPACITY {
            entries.pop_front();
        }
        entries.push_back(LogEntry {
            seq,
            time_ms,
            level: level.to_string(),
            message,
        });
    }

    /// Entries with a sequence number greater than `after`.
    pub fn since(&self, after: u64) -> Vec<LogEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.iter().filter(|e| e.seq > after).cloned().collect()
    }
}

struct MirrorLogger {
    inner: env_logger::Logger,
    buffer: &'static LogBuffer,
}

impl log::Log for MirrorLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &log::Record) {
        if !self.inner.matches(record) {
            return;
        }
        self.inner.log(record);
        if record.level() <= log::Level::Info && record.target().starts_with("rj45_sound_card") {
            self.buffer.push(record.level(), record.args().to_string());
        }
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

fn builder() -> env_logger::Builder {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.format_timestamp_millis();
    builder
}

/// Installs the stderr logger.
pub fn init() {
    builder().init();
}

/// Installs the stderr logger and mirrors this crate's records into the returned buffer.
pub fn init_with_buffer() -> &'static LogBuffer {
    let buffer: &'static LogBuffer = Box::leak(Box::default());
    let inner = builder().build();
    let max_level = inner.filter();
    if log::set_boxed_logger(Box::new(MirrorLogger { inner, buffer })).is_ok() {
        log::set_max_level(max_level);
    }
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_buffer_since_and_capacity() {
        let buffer = LogBuffer::default();
        for i in 0..CAPACITY + 10 {
            buffer.push(log::Level::Info, format!("line {}", i));
        }
        let all = buffer.since(0);
        assert_eq!(all.len(), CAPACITY);
        assert_eq!(
            all.last().unwrap().message,
            format!("line {}", CAPACITY + 9)
        );

        let last_seq = all.last().unwrap().seq;
        assert!(buffer.since(last_seq).is_empty());
        assert_eq!(buffer.since(last_seq - 2).len(), 2);
    }
}
