//! Configuration persistence for standalone mode.
//!
//! Saves and loads device selections to platform-specific config directory.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Standalone configuration (device selections)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StandaloneConfig {
    /// Selected audio output device name (None = system default)
    pub output_device: Option<String>,
    /// Selected audio input device name (None = no input)
    pub input_device: Option<String>,
    /// Selected MIDI input device name (None = no MIDI)
    pub midi_input: Option<String>,
    /// Sample rate in Hz (None = let nih_plug choose, usually 48000)
    /// Use 44100 for Bluetooth devices which often don't support 48000
    pub sample_rate: Option<u32>,
}

impl StandaloneConfig {
    /// Get the config file path for this platform
    pub fn config_path() -> Option<PathBuf> {
        directories::ProjectDirs::from("com", "ypares", "nih-faust-jit")
            .map(|dirs| dirs.config_dir().join("standalone.json"))
    }

    /// Load config from file, returning default if not found or invalid
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(&path).ok())
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    /// Save config to file
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("Could not determine config directory")?;

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        let content = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        std::fs::write(&path, content)
            .map_err(|e| format!("Failed to write config: {}", e))?;

        Ok(())
    }
}

/// Enumerate available audio devices using CPAL
pub fn enumerate_audio_devices() -> (Vec<String>, Vec<String>) {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();

    let inputs: Vec<String> = host
        .input_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();

    let outputs: Vec<String> = host
        .output_devices()
        .map(|devs| devs.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();

    (inputs, outputs)
}

/// Enumerate available MIDI input devices using midir
pub fn enumerate_midi_inputs() -> Vec<String> {
    let midi_in = match midir::MidiInput::new("nih-faust-jit-probe") {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };

    midi_in
        .ports()
        .iter()
        .filter_map(|port| midi_in.port_name(port).ok())
        .collect()
}

/// Get the system default output device name
pub fn default_output_device() -> Option<String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
}
