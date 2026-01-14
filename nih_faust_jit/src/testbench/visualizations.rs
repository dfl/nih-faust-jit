//! Oscilloscope, Spectrum Analyzer, and ERB band visualizations.

use cortix::{Analyser, Scale};
use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// Oscilloscope state for time-domain visualization.
pub struct OscilloscopeState {
    /// Display buffer for left channel
    pub left_buffer: Vec<f32>,
    /// Display buffer for right channel
    pub right_buffer: Vec<f32>,
    /// Accumulation buffer for left channel (keeps history)
    accum_left: Vec<f32>,
    /// Accumulation buffer for right channel
    accum_right: Vec<f32>,
    /// Number of samples to display
    pub display_samples: usize,
    /// Vertical scale (amplitude zoom)
    pub vertical_scale: f32,
    /// Trigger level for stable display (-1.0 to 1.0)
    pub trigger_level: f32,
    /// Whether triggering is enabled
    pub trigger_enabled: bool,
    /// Sample rate for time axis display
    pub sample_rate: f32,
}

impl Default for OscilloscopeState {
    fn default() -> Self {
        Self {
            left_buffer: Vec::with_capacity(4096),
            right_buffer: Vec::with_capacity(4096),
            accum_left: Vec::with_capacity(8192),
            accum_right: Vec::with_capacity(8192),
            display_samples: 2048,
            vertical_scale: 1.0,
            trigger_level: 0.0,
            trigger_enabled: true,
            sample_rate: 44100.0,
        }
    }
}

impl OscilloscopeState {
    /// Update display buffers from ring buffer data.
    /// Accumulates samples and applies trigger for stable display.
    pub fn update_from_samples(&mut self, left: &[f32], right: &[f32]) {
        // Accumulate new samples
        self.accum_left.extend_from_slice(left);
        self.accum_right.extend_from_slice(right);

        // Keep only the most recent samples (limit accumulation buffer size)
        let max_accum = self.display_samples * 4;
        if self.accum_left.len() > max_accum {
            let drain_count = self.accum_left.len() - max_accum;
            self.accum_left.drain(0..drain_count);
        }
        if self.accum_right.len() > max_accum {
            let drain_count = self.accum_right.len() - max_accum;
            self.accum_right.drain(0..drain_count);
        }

        // Only update display if we have enough samples
        if self.accum_left.len() >= self.display_samples {
            // Find trigger point (zero crossing with positive slope)
            let trigger_idx = if self.trigger_enabled {
                find_trigger_point(&self.accum_left, self.trigger_level)
            } else {
                0
            };

            // Copy samples starting from trigger point
            let end_idx = (trigger_idx + self.display_samples).min(self.accum_left.len());
            self.left_buffer.clear();
            self.right_buffer.clear();

            if trigger_idx < self.accum_left.len() {
                self.left_buffer.extend_from_slice(&self.accum_left[trigger_idx..end_idx]);
            }
            if trigger_idx < self.accum_right.len() {
                let right_end = (trigger_idx + self.display_samples).min(self.accum_right.len());
                self.right_buffer.extend_from_slice(&self.accum_right[trigger_idx..right_end]);
            }
        }
    }
}

/// Find zero-crossing trigger point with positive slope.
fn find_trigger_point(samples: &[f32], level: f32) -> usize {
    for i in 1..samples.len().saturating_sub(1) {
        if samples[i - 1] <= level && samples[i] > level {
            return i;
        }
    }
    0
}

/// Spectrum analyzer state for frequency-domain visualization.
pub struct SpectrumState {
    /// FFT planner
    fft: Arc<dyn RealToComplex<f32>>,
    /// FFT size
    pub fft_size: usize,
    /// Input buffer for FFT
    input_buffer: Vec<f32>,
    /// Accumulation buffer for samples
    accum_buffer: Vec<f32>,
    /// Output buffer for FFT (complex)
    output_buffer: Vec<realfft::num_complex::Complex<f32>>,
    /// Magnitude in dB for display
    pub magnitude_db: Vec<f32>,
    /// Smoothed magnitude for visual display
    pub smoothed_db: Vec<f32>,
    /// Hann window coefficients
    window: Vec<f32>,
    /// Smoothing factor (0.0 = no smoothing, 1.0 = full smoothing)
    pub smoothing: f32,
    /// Minimum dB for display
    pub min_db: f32,
    /// Maximum dB for display
    pub max_db: f32,
    /// Sample rate for frequency axis
    pub sample_rate: f32,
    /// Peak hold values
    pub peak_db: Vec<f32>,
    /// Whether peak hold is enabled
    pub peak_hold: bool,
    /// Waterfall history (each row is one FFT frame, oldest first)
    pub waterfall: Vec<Vec<f32>>,
    /// Maximum waterfall history length
    pub waterfall_history: usize,
    /// Show waterfall instead of line spectrum
    pub show_waterfall: bool,
    /// Maximum frequency to display in spectrogram (Hz)
    pub max_display_freq: f32,
}

impl Default for SpectrumState {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl SpectrumState {
    /// Create a new spectrum analyzer with the given FFT size.
    pub fn new(fft_size: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(fft_size);

        let output_len = fft_size / 2 + 1;

        // Compute Hann window
        let window: Vec<f32> = (0..fft_size)
            .map(|i| {
                let x = std::f32::consts::PI * 2.0 * i as f32 / (fft_size - 1) as f32;
                0.5 * (1.0 - x.cos())
            })
            .collect();

        Self {
            fft,
            fft_size,
            input_buffer: vec![0.0; fft_size],
            accum_buffer: Vec::with_capacity(fft_size * 2),
            output_buffer: vec![realfft::num_complex::Complex::new(0.0, 0.0); output_len],
            magnitude_db: vec![-96.0; output_len],
            smoothed_db: vec![-96.0; output_len],
            window,
            smoothing: 0.8,
            min_db: -96.0,
            max_db: 0.0,
            sample_rate: 44100.0,
            peak_db: vec![-96.0; output_len],
            peak_hold: false,
            waterfall: Vec::with_capacity(200),
            waterfall_history: 200,
            show_waterfall: false,
            max_display_freq: 20000.0,
        }
    }

    /// Process samples and compute spectrum.
    /// Accumulates samples until we have enough for FFT.
    pub fn process(&mut self, samples: &[f32]) {
        // Accumulate samples
        self.accum_buffer.extend_from_slice(samples);

        // Keep buffer from growing too large
        let max_accum = self.fft_size * 2;
        if self.accum_buffer.len() > max_accum {
            let drain_count = self.accum_buffer.len() - max_accum;
            self.accum_buffer.drain(0..drain_count);
        }

        // Only process if we have enough samples
        if self.accum_buffer.len() < self.fft_size {
            return;
        }

        // Copy and window the input (use most recent samples)
        let start = self.accum_buffer.len().saturating_sub(self.fft_size);
        for (i, sample) in self.accum_buffer[start..].iter().enumerate().take(self.fft_size) {
            self.input_buffer[i] = sample * self.window[i];
        }

        // Perform FFT
        if self.fft.process(&mut self.input_buffer, &mut self.output_buffer).is_ok() {
            // Convert to magnitude (dB)
            let normalization = 2.0 / self.fft_size as f32;
            for (i, complex) in self.output_buffer.iter().enumerate() {
                let magnitude = (complex.re * complex.re + complex.im * complex.im).sqrt() * normalization;
                let db = if magnitude > 1e-10 {
                    20.0 * magnitude.log10()
                } else {
                    self.min_db
                };
                let db = db.clamp(self.min_db, self.max_db);
                self.magnitude_db[i] = db;

                // Apply smoothing
                self.smoothed_db[i] = self.smoothed_db[i] * self.smoothing + db * (1.0 - self.smoothing);

                // Update peak hold
                if self.peak_hold && db > self.peak_db[i] {
                    self.peak_db[i] = db;
                }
            }

            // Add to waterfall history
            if self.show_waterfall {
                self.waterfall.push(self.smoothed_db.clone());
                // Remove oldest entries if we exceed history limit
                while self.waterfall.len() > self.waterfall_history {
                    self.waterfall.remove(0);
                }
            }
        }
    }

    /// Get frequency for a given bin index.
    pub fn bin_to_freq(&self, bin: usize) -> f32 {
        bin as f32 * self.sample_rate / self.fft_size as f32
    }

    /// Reset peak hold values.
    pub fn reset_peaks(&mut self) {
        for peak in &mut self.peak_db {
            *peak = self.min_db;
        }
    }
}

/// ERB (Equivalent Rectangular Bandwidth) analyzer state using Gammatone filterbank.
pub struct ErbState {
    /// Cortix analyser with Gammatone filterbank
    analyser: Analyser,
    /// Number of ERB bands
    pub num_bands: usize,
    /// Current envelope in dB
    pub envelope_db: Vec<f32>,
    /// Smoothed envelope for display
    pub smoothed_db: Vec<f32>,
    /// Sample rate
    pub sample_rate: f32,
    /// Minimum dB for display
    pub min_db: f32,
    /// Maximum dB for display
    pub max_db: f32,
    /// Smoothing factor for display
    pub smoothing: f32,
    /// Waterfall history for spectrogram
    pub waterfall: Vec<Vec<f32>>,
    /// Maximum waterfall history length
    pub waterfall_history: usize,
    /// Show waterfall (spectrogram) view
    pub show_waterfall: bool,
}

impl ErbState {
    /// Create a new ERB analyzer with the specified number of bands.
    pub fn new(num_bands: usize, sample_rate: f32) -> Self {
        let analyser = Analyser::builder()
            .bands(num_bands)
            .scale(Scale::ERB)
            .sample_rate(sample_rate)
            .range(20.0, 20000.0)
            .smoothing(5.0) // 5ms smoothing in the filterbank
            .build();

        Self {
            analyser,
            num_bands,
            envelope_db: vec![-100.0; num_bands],
            smoothed_db: vec![-100.0; num_bands],
            sample_rate,
            min_db: -96.0,
            max_db: 0.0,
            smoothing: 0.7,
            waterfall: Vec::with_capacity(200),
            waterfall_history: 200,
            show_waterfall: false,
        }
    }

    /// Update sample rate and rebuild analyser.
    pub fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() > 1.0 {
            self.sample_rate = sample_rate;
            self.analyser = Analyser::builder()
                .bands(self.num_bands)
                .scale(Scale::ERB)
                .sample_rate(sample_rate)
                .range(20.0, 20000.0)
                .smoothing(5.0)
                .build();
        }
    }

    /// Process audio samples and update envelope.
    pub fn process(&mut self, left: &[f32], right: &[f32]) {
        // Process stereo input (averages L+R internally)
        let envelope = self.analyser.process_stereo(left, right);

        // Convert to dB and apply display smoothing
        for (i, &mag) in envelope.iter().enumerate() {
            let db = if mag > 1e-10 {
                20.0 * mag.log10()
            } else {
                self.min_db
            };
            let db = db.clamp(self.min_db, self.max_db);
            self.envelope_db[i] = db;

            // Apply display smoothing
            self.smoothed_db[i] = self.smoothed_db[i] * self.smoothing + db * (1.0 - self.smoothing);
        }

        // Add to waterfall history
        if self.show_waterfall {
            self.waterfall.push(self.smoothed_db.clone());
            while self.waterfall.len() > self.waterfall_history {
                self.waterfall.remove(0);
            }
        }
    }

    /// Get the center frequency for a band.
    pub fn center_hz(&self, band: usize) -> f32 {
        self.analyser.center_hz(band)
    }

    /// Get band info for all bands.
    pub fn bands(&self) -> &[cortix::BandInfo] {
        self.analyser.bands()
    }

    /// Reset the analyser state.
    pub fn reset(&mut self) {
        self.analyser.reset();
        for db in &mut self.envelope_db {
            *db = self.min_db;
        }
        for db in &mut self.smoothed_db {
            *db = self.min_db;
        }
        self.waterfall.clear();
    }
}

impl Default for ErbState {
    fn default() -> Self {
        Self::new(48, 44100.0)
    }
}

/// Combined testbench visualization state.
pub struct TestbenchState {
    pub oscilloscope: OscilloscopeState,
    /// Post-DSP spectrum (output)
    pub spectrum: SpectrumState,
    /// Pre-DSP spectrum (input, for comparison)
    pub pre_spectrum: SpectrumState,
    /// Post-DSP ERB bands (output)
    pub erb: ErbState,
    /// Pre-DSP ERB bands (input, for comparison)
    pub pre_erb: ErbState,
    /// Temporary buffer for reading from ring buffer
    pub temp_left: Vec<f32>,
    pub temp_right: Vec<f32>,
    /// Whether testbench is expanded/visible
    pub visible: bool,
    /// Currently selected tab (0=oscilloscope, 1=spectrum, 2=erb, 3=benchmark)
    pub selected_tab: usize,
    /// Show pre/post comparison overlay (for effects)
    pub show_comparison: bool,
    /// Normalize spectra to peak/max (for harmonic comparison)
    pub normalize_to_max: bool,
    /// Show spectrogram (waterfall) instead of line/bar view (shared between Spectrum and ERB)
    pub show_spectrogram: bool,
}

impl Default for TestbenchState {
    fn default() -> Self {
        Self {
            oscilloscope: OscilloscopeState::default(),
            spectrum: SpectrumState::default(),
            pre_spectrum: SpectrumState::default(),
            erb: ErbState::default(),
            pre_erb: ErbState::default(),
            temp_left: Vec::with_capacity(8192),
            temp_right: Vec::with_capacity(8192),
            visible: true,
            selected_tab: 0,
            show_comparison: false,
            normalize_to_max: false,
            show_spectrogram: false,
        }
    }
}
