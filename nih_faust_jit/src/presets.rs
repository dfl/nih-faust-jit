use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Preset data containing parameter values
#[derive(Debug, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub values: HashMap<String, f32>,
}

/// Manages preset storage and retrieval
pub struct PresetManager {
    presets_dir: Option<PathBuf>,
}

impl PresetManager {
    pub fn new() -> Self {
        let presets_dir = directories::ProjectDirs::from("com", "ypares", "nih-faust-jit")
            .map(|dirs| dirs.data_dir().join("presets"));

        // Create the presets directory if it doesn't exist
        if let Some(ref dir) = presets_dir {
            let _ = std::fs::create_dir_all(dir);
        }

        Self { presets_dir }
    }

    /// Get the directory path for a specific DSP script's presets
    fn dsp_presets_dir(&self, dsp_name: &str) -> Option<PathBuf> {
        self.presets_dir.as_ref().map(|dir| dir.join(dsp_name))
    }

    /// List all preset names for a given DSP script
    pub fn list_presets(&self, dsp_name: &str) -> Vec<String> {
        let Some(dir) = self.dsp_presets_dir(dsp_name) else {
            return Vec::new();
        };

        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };

        entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|ext| ext == "json").unwrap_or(false))
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            })
            .collect()
    }

    /// Save a preset for a given DSP script
    pub fn save_preset(
        &self,
        dsp_name: &str,
        preset_name: &str,
        values: HashMap<String, f32>,
    ) -> Result<(), String> {
        let dir = self
            .dsp_presets_dir(dsp_name)
            .ok_or("Presets directory not available")?;

        std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create directory: {}", e))?;

        let preset = Preset {
            name: preset_name.to_string(),
            values,
        };

        let path = dir.join(format!("{}.json", sanitize_filename(preset_name)));
        let json = serde_json::to_string_pretty(&preset)
            .map_err(|e| format!("Failed to serialize preset: {}", e))?;

        std::fs::write(&path, json).map_err(|e| format!("Failed to write preset: {}", e))?;

        Ok(())
    }

    /// Load a preset for a given DSP script
    pub fn load_preset(&self, dsp_name: &str, preset_name: &str) -> Result<Preset, String> {
        let dir = self
            .dsp_presets_dir(dsp_name)
            .ok_or("Presets directory not available")?;

        let path = dir.join(format!("{}.json", sanitize_filename(preset_name)));
        let json =
            std::fs::read_to_string(&path).map_err(|e| format!("Failed to read preset: {}", e))?;

        serde_json::from_str(&json).map_err(|e| format!("Failed to parse preset: {}", e))
    }

    /// Delete a preset
    pub fn delete_preset(&self, dsp_name: &str, preset_name: &str) -> Result<(), String> {
        let dir = self
            .dsp_presets_dir(dsp_name)
            .ok_or("Presets directory not available")?;

        let path = dir.join(format!("{}.json", sanitize_filename(preset_name)));
        std::fs::remove_file(&path).map_err(|e| format!("Failed to delete preset: {}", e))
    }
}

impl Default for PresetManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Sanitize filename to remove invalid characters
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}
