//! Audio file loading and playback for standalone mode.
//!
//! Supports WAV, MP3, FLAC formats via symphonia.
//! Handles sample rate conversion via rubato.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decoded and resampled audio data, ready for playback
#[derive(Clone)]
pub struct DecodedAudio {
    /// Per-channel sample data (f32), resampled to target SR
    pub channels: Arc<Vec<Vec<f32>>>,
    /// Sample rate (matches plugin SR after resampling)
    pub sample_rate: u32,
    /// Total number of frames
    pub total_frames: u64,
    /// Original file path (for display)
    pub file_path: PathBuf,
    /// Original file sample rate (for display)
    pub original_sample_rate: u32,
    /// Number of channels
    pub num_channels: usize,
}

/// Playback state shared between GUI and audio thread.
/// Uses atomics for lock-free audio thread access.
#[derive(Default)]
pub struct PlaybackState {
    /// Current playback position in frames
    pub position: AtomicU64,
    /// Is playback active
    pub playing: AtomicBool,
    /// Loop mode enabled
    pub looping: AtomicBool,
}

impl PlaybackState {
    pub fn new() -> Self {
        Self {
            position: AtomicU64::new(0),
            // Start with playing=true so instruments show visualization immediately
            // and can be paused to freeze the display
            playing: AtomicBool::new(true),
            looping: AtomicBool::new(false),
        }
    }

    pub fn reset(&self) {
        self.position.store(0, Ordering::Relaxed);
        self.playing.store(false, Ordering::Relaxed);
    }
}

/// Complete audio file player state
pub struct AudioFilePlayer {
    /// Decoded audio data (None if no file loaded) - wrapped in Arc for lock-free access
    audio_data: RwLock<Option<Arc<DecodedAudio>>>,
    /// Cached Arc for audio thread (updated when new audio is loaded)
    pub cached_audio: RwLock<Option<Arc<DecodedAudio>>>,
    /// Playback state (lock-free atomics)
    pub state: Arc<PlaybackState>,
}

impl AudioFilePlayer {
    pub fn new(state: Arc<PlaybackState>) -> Self {
        Self {
            audio_data: RwLock::new(None),
            cached_audio: RwLock::new(None),
            state,
        }
    }

    /// Set new audio data (called from background thread)
    pub fn set_audio(&self, audio: DecodedAudio) {
        let arc = Arc::new(audio);
        *self.audio_data.write().unwrap() = Some(Arc::clone(&arc));
        *self.cached_audio.write().unwrap() = Some(arc);
    }

    /// Get audio data for display (GUI thread)
    pub fn get_audio_info(&self) -> Option<Arc<DecodedAudio>> {
        self.audio_data.read().unwrap().clone()
    }

    /// Get audio data for playback (audio thread) - uses try_read to avoid blocking
    pub fn get_audio_for_playback(&self) -> Option<Arc<DecodedAudio>> {
        // First try the cached version (always available after first load)
        if let Ok(guard) = self.cached_audio.try_read() {
            return guard.clone();
        }
        // Fallback to main data
        self.audio_data.try_read().ok().and_then(|g| g.clone())
    }
}

/// Load and decode an audio file, resampling to target sample rate if needed.
pub fn load_audio_file(path: &Path, target_sample_rate: u32) -> Result<DecodedAudio, String> {
    // Open the file
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open file: {}", e))?;

    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    // Probe the format
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Failed to probe format: {}", e))?;

    let mut format = probed.format;

    // Find the first audio track
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("No audio track found")?;

    let track_id = track.id;

    let original_sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("Unknown sample rate")?;

    let num_channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .unwrap_or(2);

    // Create decoder
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {}", e))?;

    // Decode all samples
    let mut decoded_channels: Vec<Vec<f32>> = vec![Vec::new(); num_channels];

    loop {
        match format.next_packet() {
            Ok(packet) => {
                if packet.track_id() != track_id {
                    continue;
                }

                match decoder.decode(&packet) {
                    Ok(decoded) => {
                        let spec = *decoded.spec();
                        let duration = decoded.capacity() as u64;

                        let mut sample_buf = SampleBuffer::<f32>::new(duration, spec);
                        sample_buf.copy_interleaved_ref(decoded);

                        // De-interleave into channel vectors
                        let samples = sample_buf.samples();
                        let ch_count = spec.channels.count();
                        for (i, &sample) in samples.iter().enumerate() {
                            let ch = i % ch_count;
                            if ch < decoded_channels.len() {
                                decoded_channels[ch].push(sample);
                            }
                        }
                    }
                    Err(SymphoniaError::DecodeError(e)) => {
                        // Recoverable decode error, skip this packet
                        eprintln!("Decode warning: {}", e);
                    }
                    Err(e) => {
                        return Err(format!("Decode error: {}", e));
                    }
                }
            }
            Err(SymphoniaError::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                // End of file
                break;
            }
            Err(e) => {
                return Err(format!("Format read error: {}", e));
            }
        }
    }

    if decoded_channels.is_empty() || decoded_channels[0].is_empty() {
        return Err("No audio data decoded".to_string());
    }

    // Resample if needed
    let final_channels = if original_sample_rate != target_sample_rate {
        resample_audio(&decoded_channels, original_sample_rate, target_sample_rate)?
    } else {
        decoded_channels
    };

    let total_frames = final_channels.first().map(|c| c.len()).unwrap_or(0) as u64;

    Ok(DecodedAudio {
        channels: Arc::new(final_channels),
        sample_rate: target_sample_rate,
        total_frames,
        file_path: path.to_path_buf(),
        original_sample_rate,
        num_channels,
    })
}

/// Resample audio using rubato (high-quality sinc resampling)
fn resample_audio(
    input: &[Vec<f32>],
    from_rate: u32,
    to_rate: u32,
) -> Result<Vec<Vec<f32>>, String> {
    use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

    let channels = input.len();
    if channels == 0 {
        return Ok(Vec::new());
    }

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let mut resampler = SincFixedIn::<f32>::new(
        to_rate as f64 / from_rate as f64,
        2.0, // max relative ratio (for variable rate, not used here)
        params,
        input[0].len(),
        channels,
    )
    .map_err(|e| format!("Failed to create resampler: {}", e))?;

    // Convert input to the format rubato expects
    let input_refs: Vec<&[f32]> = input.iter().map(|ch| ch.as_slice()).collect();

    let output = resampler
        .process(&input_refs, None)
        .map_err(|e| format!("Resample error: {}", e))?;

    Ok(output)
}

/// Fill an audio buffer from decoded audio data.
/// Returns true if audio was written, false if playback stopped (end of file, not looping).
pub fn fill_buffer_from_audio(
    audio: &DecodedAudio,
    state: &PlaybackState,
    buffer: &mut [&mut [f32]],
) -> bool {
    if !state.playing.load(Ordering::Relaxed) {
        return false;
    }

    let num_samples = buffer.first().map(|b| b.len()).unwrap_or(0);
    if num_samples == 0 {
        return true;
    }

    let channels = &*audio.channels;
    if channels.is_empty() {
        return false;
    }

    let mut pos = state.position.load(Ordering::Relaxed);
    let total = audio.total_frames;
    let looping = state.looping.load(Ordering::Relaxed);

    for sample_idx in 0..num_samples {
        if pos >= total {
            if looping {
                pos = 0;
            } else {
                // Stop playback, fill remainder with silence
                state.playing.store(false, Ordering::Relaxed);
                for ch in buffer.iter_mut() {
                    for remaining in sample_idx..num_samples {
                        ch[remaining] = 0.0;
                    }
                }
                state.position.store(pos, Ordering::Relaxed);
                return false;
            }
        }

        // Copy audio data to buffer (handle mono -> stereo if needed)
        for (ch_idx, ch_buf) in buffer.iter_mut().enumerate() {
            let src_ch = ch_idx.min(channels.len() - 1);
            ch_buf[sample_idx] = channels[src_ch][pos as usize];
        }
        pos += 1;
    }

    state.position.store(pos, Ordering::Relaxed);
    true
}

/// Format duration in seconds as MM:SS
pub fn format_duration(seconds: f64) -> String {
    let mins = (seconds / 60.0).floor() as u32;
    let secs = (seconds % 60.0).floor() as u32;
    format!("{:02}:{:02}", mins, secs)
}
