//! Testbench module for audio visualization and benchmarking.
//!
//! Provides:
//! - Oscilloscope (time-domain waveform display)
//! - Spectrum analyzer (FFT frequency display)
//! - Pre/post DSP comparison for effects
//! - Performance benchmarking metrics
//!
//! Architecture:
//! - AudioProducer: owned by audio thread (writes samples)
//! - AudioConsumer: owned by GUI thread (reads samples)
//! - BenchmarkMetrics: shared via atomics (lock-free)

pub mod benchmarking;
pub mod ring_buffer;
pub mod visualizations;

pub use benchmarking::BenchmarkMetrics;
pub use ring_buffer::{create_stereo_ring_buffer, AudioConsumer, AudioProducer};
pub use visualizations::{ErbState, OscilloscopeState, SpectrumState, TestbenchState};

use std::sync::Arc;

/// Audio thread side of the testbench.
/// Contains producers for ring buffers (owned by audio thread).
pub struct TestbenchAudio {
    /// Ring buffer producer for post-DSP visualization
    pub producer: AudioProducer,
    /// Ring buffer producer for pre-DSP visualization (effects only)
    pub pre_producer: AudioProducer,
    /// Whether visualization is enabled
    pub enabled: std::sync::atomic::AtomicBool,
}

impl TestbenchAudio {
    /// Push post-DSP samples to visualization ring buffer.
    pub fn push_from_buffer(&mut self, buffer: &[&mut [f32]]) {
        if self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            self.producer.push_from_buffer(buffer);
        }
    }

    /// Push pre-DSP samples to visualization ring buffer (for effects comparison).
    pub fn push_pre_dsp(&mut self, buffer: &[&mut [f32]]) {
        if self.enabled.load(std::sync::atomic::Ordering::Relaxed) {
            self.pre_producer.push_from_buffer(buffer);
        }
    }
}

/// GUI thread side of the testbench.
/// Contains consumers for ring buffers and visualization state.
pub struct TestbenchGui {
    /// Ring buffer consumer for post-DSP (GUI thread reads from here)
    pub consumer: AudioConsumer,
    /// Ring buffer consumer for pre-DSP
    pub pre_consumer: AudioConsumer,
    /// Visualization state (oscilloscope, spectrum, etc.)
    pub state: TestbenchState,
    /// Temporary buffers for pre-DSP samples
    temp_pre_left: Vec<f32>,
    temp_pre_right: Vec<f32>,
}

impl TestbenchGui {
    /// Update visualization state from ring buffer data.
    pub fn update(&mut self) {
        // Sync waterfall settings from main spectrum to pre-spectrum
        self.state.pre_spectrum.show_waterfall = self.state.spectrum.show_waterfall;
        self.state.pre_erb.show_waterfall = self.state.erb.show_waterfall;

        // Read post-DSP samples from ring buffer
        self.consumer.read_available(
            &mut self.state.temp_left,
            &mut self.state.temp_right,
        );

        // Read pre-DSP samples
        self.pre_consumer.read_available(
            &mut self.temp_pre_left,
            &mut self.temp_pre_right,
        );

        // Update post-DSP visualizations
        if !self.state.temp_left.is_empty() {
            self.state.oscilloscope.update_from_samples(
                &self.state.temp_left,
                &self.state.temp_right,
            );
            self.state.spectrum.process(&self.state.temp_left);
            self.state.erb.process(&self.state.temp_left, &self.state.temp_right);
        }

        // Update pre-DSP visualizations (for comparison)
        if !self.temp_pre_left.is_empty() {
            self.state.pre_spectrum.process(&self.temp_pre_left);
            self.state.pre_erb.process(&self.temp_pre_left, &self.temp_pre_right);
        }
    }
}

/// Shared benchmark metrics (uses atomics, can be accessed from both threads).
pub type SharedMetrics = Arc<BenchmarkMetrics>;

/// Create the testbench components.
/// Returns:
/// - TestbenchAudio: for the audio thread
/// - TestbenchGui: for the GUI thread
/// - SharedMetrics: for both threads
pub fn create_testbench() -> (TestbenchAudio, TestbenchGui, SharedMetrics) {
    let (producer, consumer) = create_stereo_ring_buffer(8192);
    let (pre_producer, pre_consumer) = create_stereo_ring_buffer(8192);
    let metrics = Arc::new(BenchmarkMetrics::new());

    let audio = TestbenchAudio {
        producer,
        pre_producer,
        enabled: std::sync::atomic::AtomicBool::new(true),
    };

    let gui = TestbenchGui {
        consumer,
        pre_consumer,
        state: TestbenchState::default(),
        temp_pre_left: Vec::with_capacity(8192),
        temp_pre_right: Vec::with_capacity(8192),
    };

    (audio, gui, metrics)
}
