//! Oversampling module using halfband filters for 2x/4x/8x oversampling.
//!
//! Uses FIR halfband filters with 31 taps for good balance of quality and performance.
//!
//! ## How it works
//!
//! Oversampling reduces aliasing artifacts from nonlinear DSP operations (distortion,
//! saturation, waveshaping) by:
//! 1. Upsampling the input signal (1 sample → N samples with interpolation filtering)
//! 2. Processing at the higher sample rate
//! 3. Downsampling back (N samples → 1 sample with anti-aliasing filtering)
//!
//! ## DSP sample rate
//!
//! When oversampling is enabled, the Faust DSP is automatically reloaded at the
//! oversampled sample rate (host_rate × oversampling_factor). This ensures that
//! time-based effects (delays, filters, LFOs) operate correctly at the higher rate.
//!
//! Changing the oversampling factor triggers an async DSP reload. There may be a
//! brief transition period while the new DSP loads.

use halfband::fir::presets::{Downsampler31, Upsampler31};
use nih_plug::prelude::Enum;

/// Oversampling factor selection
/// Note: 8x is not supported because Faust's SVF filters lose precision at very high sample rates
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Enum, strum_macros::EnumIter)]
pub enum OversamplingFactor {
    #[default]
    #[name = "1x (Off)"]
    X1 = 0,
    #[name = "2x"]
    X2 = 1,
    #[name = "4x"]
    X4 = 2,
}

impl OversamplingFactor {
    /// Get the numeric multiplier
    pub fn factor(&self) -> usize {
        match self {
            OversamplingFactor::X1 => 1,
            OversamplingFactor::X2 => 2,
            OversamplingFactor::X4 => 4,
        }
    }

    /// Create from a numeric factor (1, 2, or 4)
    pub fn from_factor(factor: u8) -> Self {
        match factor {
            2 => OversamplingFactor::X2,
            4 => OversamplingFactor::X4,
            _ => OversamplingFactor::X1,
        }
    }
}

/// Oversampler for a single channel, supporting 1x/2x/4x
pub struct ChannelOversampler {
    // Cascade of 2x stages for upsampling (up to 2 stages for 4x)
    up1: Upsampler31,
    up2: Upsampler31,
    // Cascade of 2x stages for downsampling
    down1: Downsampler31,
    down2: Downsampler31,
}

impl Default for ChannelOversampler {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelOversampler {
    pub fn new() -> Self {
        Self {
            up1: Upsampler31::default(),
            up2: Upsampler31::default(),
            down1: Downsampler31::default(),
            down2: Downsampler31::default(),
        }
    }

    /// Reset filter states (call when audio stream restarts)
    pub fn reset(&mut self) {
        self.up1 = Upsampler31::default();
        self.up2 = Upsampler31::default();
        self.down1 = Downsampler31::default();
        self.down2 = Downsampler31::default();
    }

    /// Upsample input samples into output buffer
    /// Returns the number of output samples written
    pub fn upsample(&mut self, factor: OversamplingFactor, input: &[f32], output: &mut [f32]) -> usize {
        let n = factor.factor();
        debug_assert!(output.len() >= input.len() * n);

        match factor {
            OversamplingFactor::X1 => {
                output[..input.len()].copy_from_slice(input);
                input.len()
            }
            OversamplingFactor::X2 => {
                for (i, &s) in input.iter().enumerate() {
                    let [s0, s1] = self.up1.process_sample(s);
                    output[i * 2] = s0;
                    output[i * 2 + 1] = s1;
                }
                input.len() * 2
            }
            OversamplingFactor::X4 => {
                for (i, &s) in input.iter().enumerate() {
                    let [a0, a1] = self.up1.process_sample(s);
                    let [b0, b1] = self.up2.process_sample(a0);
                    let [b2, b3] = self.up2.process_sample(a1);
                    output[i * 4] = b0;
                    output[i * 4 + 1] = b1;
                    output[i * 4 + 2] = b2;
                    output[i * 4 + 3] = b3;
                }
                input.len() * 4
            }
        }
    }

    /// Downsample input samples into output buffer
    /// Returns the number of output samples written
    pub fn downsample(&mut self, factor: OversamplingFactor, input: &[f32], output: &mut [f32]) -> usize {
        let n = factor.factor();
        let out_len = input.len() / n;
        debug_assert!(output.len() >= out_len);

        match factor {
            OversamplingFactor::X1 => {
                output[..input.len()].copy_from_slice(input);
                input.len()
            }
            OversamplingFactor::X2 => {
                for i in 0..out_len {
                    output[i] = self.down1.process_sample(input[i * 2], input[i * 2 + 1]);
                }
                out_len
            }
            OversamplingFactor::X4 => {
                for i in 0..out_len {
                    let a0 = self.down2.process_sample(input[i * 4], input[i * 4 + 1]);
                    let a1 = self.down2.process_sample(input[i * 4 + 2], input[i * 4 + 3]);
                    output[i] = self.down1.process_sample(a0, a1);
                }
                out_len
            }
        }
    }
}

/// Stereo oversampler with pre-allocated buffers
pub struct StereoOversampler {
    pub left: ChannelOversampler,
    pub right: ChannelOversampler,
    /// Oversampled buffer for left channel
    pub buffer_left: Vec<f32>,
    /// Oversampled buffer for right channel
    pub buffer_right: Vec<f32>,
    /// Maximum supported buffer size at 1x
    max_block_size: usize,
}

impl StereoOversampler {
    /// Create a new stereo oversampler
    /// `max_block_size` is the maximum number of samples per channel at 1x rate
    pub fn new(max_block_size: usize) -> Self {
        // Allocate for maximum oversampling (4x)
        let max_oversampled = max_block_size * 4;
        Self {
            left: ChannelOversampler::new(),
            right: ChannelOversampler::new(),
            buffer_left: vec![0.0; max_oversampled],
            buffer_right: vec![0.0; max_oversampled],
            max_block_size,
        }
    }

    /// Reset filter states
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Resize internal buffers if needed
    pub fn ensure_buffer_size(&mut self, block_size: usize) {
        if block_size > self.max_block_size {
            self.max_block_size = block_size;
            let max_oversampled = block_size * 4;
            self.buffer_left.resize(max_oversampled, 0.0);
            self.buffer_right.resize(max_oversampled, 0.0);
        }
    }

    /// Get mutable slices to the oversampled buffers for the given factor and input size
    pub fn get_oversampled_buffers(&mut self, factor: OversamplingFactor, input_size: usize) -> (&mut [f32], &mut [f32]) {
        let oversampled_size = input_size * factor.factor();
        (&mut self.buffer_left[..oversampled_size], &mut self.buffer_right[..oversampled_size])
    }
}
