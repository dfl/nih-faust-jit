//! Performance metrics collection for DSP benchmarking.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

/// Atomic performance metrics that can be written from the audio thread
/// and read from the GUI thread without locks.
pub struct BenchmarkMetrics {
    /// Last process() duration in nanoseconds
    pub last_process_ns: AtomicU64,
    /// Maximum process() duration in nanoseconds
    pub max_process_ns: AtomicU64,
    /// Sum of process times for averaging
    process_sum_ns: AtomicU64,
    /// Count of process calls
    process_count: AtomicU64,
    /// Current buffer size
    pub buffer_size: AtomicU32,
    /// Current sample rate
    pub sample_rate: AtomicU32,
    /// Number of potential buffer underruns detected
    pub xrun_count: AtomicU32,
}

impl Default for BenchmarkMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl BenchmarkMetrics {
    pub fn new() -> Self {
        Self {
            last_process_ns: AtomicU64::new(0),
            max_process_ns: AtomicU64::new(0),
            process_sum_ns: AtomicU64::new(0),
            process_count: AtomicU64::new(0),
            buffer_size: AtomicU32::new(512),
            sample_rate: AtomicU32::new(44100),
            xrun_count: AtomicU32::new(0),
        }
    }

    /// Record a process duration (call from audio thread).
    pub fn record_process_time(&self, duration_ns: u64) {
        self.last_process_ns.store(duration_ns, Ordering::Relaxed);

        // Update max
        let mut current_max = self.max_process_ns.load(Ordering::Relaxed);
        while duration_ns > current_max {
            match self.max_process_ns.compare_exchange_weak(
                current_max,
                duration_ns,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current_max = actual,
            }
        }

        // Update sum and count for averaging
        self.process_sum_ns.fetch_add(duration_ns, Ordering::Relaxed);
        self.process_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Get average process time in nanoseconds.
    pub fn average_process_ns(&self) -> u64 {
        let sum = self.process_sum_ns.load(Ordering::Relaxed);
        let count = self.process_count.load(Ordering::Relaxed);
        if count > 0 {
            sum / count
        } else {
            0
        }
    }

    /// Calculate CPU usage percentage based on process time vs available time.
    pub fn cpu_usage_percent(&self) -> f32 {
        let process_ns = self.last_process_ns.load(Ordering::Relaxed) as f32;
        let buffer_size = self.buffer_size.load(Ordering::Relaxed) as f32;
        let sample_rate = self.sample_rate.load(Ordering::Relaxed) as f32;

        if sample_rate > 0.0 && buffer_size > 0.0 {
            // Available time for this buffer in nanoseconds
            let available_ns = (buffer_size / sample_rate) * 1_000_000_000.0;
            (process_ns / available_ns) * 100.0
        } else {
            0.0
        }
    }

    /// Reset all metrics.
    pub fn reset(&self) {
        self.last_process_ns.store(0, Ordering::Relaxed);
        self.max_process_ns.store(0, Ordering::Relaxed);
        self.process_sum_ns.store(0, Ordering::Relaxed);
        self.process_count.store(0, Ordering::Relaxed);
        self.xrun_count.store(0, Ordering::Relaxed);
    }

    /// Increment xrun counter.
    pub fn record_xrun(&self) {
        self.xrun_count.fetch_add(1, Ordering::Relaxed);
    }
}

/// RAII timer that records duration when dropped.
pub struct ProcessTimer<'a> {
    metrics: &'a BenchmarkMetrics,
    start: Instant,
}

impl<'a> ProcessTimer<'a> {
    pub fn new(metrics: &'a BenchmarkMetrics) -> Self {
        Self {
            metrics,
            start: Instant::now(),
        }
    }
}

impl<'a> Drop for ProcessTimer<'a> {
    fn drop(&mut self) {
        let duration = self.start.elapsed();
        self.metrics.record_process_time(duration.as_nanos() as u64);
    }
}
