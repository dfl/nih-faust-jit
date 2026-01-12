use egui::Color32;

/// Meter color state for VU meter visualization
#[derive(Clone, Copy, Default, PartialEq)]
pub enum MeterColorState {
    #[default]
    Normal, // Green (0-80%)
    Warn,   // Orange (80-100%)
    Clip,   // Red (>100%)
}

/// State machine for meter display with peak hold and clip indication
#[derive(Clone)]
pub struct MeterState {
    clip_frames_remaining: u8,
    current_state: MeterColorState,
    peak_hold: f32,
}

impl Default for MeterState {
    fn default() -> Self {
        Self {
            clip_frames_remaining: 0,
            current_state: MeterColorState::Normal,
            peak_hold: 0.0,
        }
    }
}

impl MeterState {
    const WARN_THRESHOLD: f32 = 0.8;
    const CLIP_HOLD_FRAMES: u8 = 10;
    const DECAY_RATE: f32 = 0.95;

    pub fn update(&mut self, normalized_value: f32) -> Color32 {
        // Peak hold with decay
        if normalized_value > self.peak_hold {
            self.peak_hold = normalized_value;
        } else {
            self.peak_hold *= Self::DECAY_RATE;
        }

        // Check for clipping based on peak hold
        if self.peak_hold >= 1.0 {
            self.current_state = MeterColorState::Clip;
            self.clip_frames_remaining = Self::CLIP_HOLD_FRAMES;
        } else if self.peak_hold >= Self::WARN_THRESHOLD {
            if self.current_state != MeterColorState::Clip {
                self.current_state = MeterColorState::Warn;
            }
        }

        // Decay clipping indicator
        if self.clip_frames_remaining > 0 {
            self.clip_frames_remaining -= 1;
        } else if self.current_state == MeterColorState::Clip {
            self.current_state = if self.peak_hold >= Self::WARN_THRESHOLD {
                MeterColorState::Warn
            } else {
                MeterColorState::Normal
            };
        } else if self.current_state == MeterColorState::Warn
            && self.peak_hold < Self::WARN_THRESHOLD
        {
            self.current_state = MeterColorState::Normal;
        }

        match self.current_state {
            MeterColorState::Normal => Color32::from_rgb(0, 180, 0),
            MeterColorState::Warn => Color32::from_rgb(255, 165, 0),
            MeterColorState::Clip => Color32::RED,
        }
    }

    pub fn get_display_value(&self) -> f32 {
        self.peak_hold
    }

    pub fn is_clipping(&self) -> bool {
        self.clip_frames_remaining > 0
    }
}

/// VU meter color thresholds
pub const GREEN_END: f32 = 0.7;
pub const ORANGE_END: f32 = 0.9;

/// Standard meter colors
pub fn meter_colors() -> (Color32, Color32, Color32) {
    (
        Color32::from_rgb(80, 200, 120),  // green
        Color32::from_rgb(255, 180, 0),   // orange
        Color32::from_rgb(255, 80, 80),   // red
    )
}

#[allow(dead_code)]
pub fn lerp_colors(min: Color32, max: Color32, t: f32) -> Color32 {
    let lerp_comps = |x, y| ((1.0 - t) * x as f32 + t * y as f32) as u8;
    Color32::from_rgb(
        lerp_comps(min.r(), max.r()),
        lerp_comps(min.g(), max.g()),
        lerp_comps(min.b(), max.b()),
    )
}
