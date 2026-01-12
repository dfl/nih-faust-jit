use egui::{Color32, Pos2, Stroke};
use std::time::Instant;

use faust_jit::WidgetScale;

/// Full range in pixels for drag (from faustVST: 160px = full range)
pub const DRAG_FULL_RANGE_PX: f32 = 160.0;

/// Full range in scroll units (about 10 scroll notches for full range)
pub const SCROLL_FULL_RANGE: f32 = 1200.0;

/// Returns drag sensitivity multiplier (0.1 when shift held for fine control)
pub fn drag_sensitivity(ui: &egui::Ui) -> f32 {
    if ui.input(|i| i.modifiers.shift) {
        0.1
    } else {
        1.0
    }
}

/// Draw an arc for knob widgets
pub fn draw_arc(
    painter: &egui::Painter,
    center: Pos2,
    radius: f32,
    start_angle: f32,
    end_angle: f32,
    stroke: Stroke,
) {
    let n_points = 32;
    let angle_range = end_angle - start_angle;
    let points: Vec<Pos2> = (0..=n_points)
        .map(|i| {
            let t = i as f32 / n_points as f32;
            let angle = start_angle + t * angle_range;
            center + egui::vec2(angle.sin() * radius, -angle.cos() * radius)
        })
        .collect();

    for window in points.windows(2) {
        painter.line_segment([window[0], window[1]], stroke);
    }
}

/// Apply scale transformation for display (bargraphs)
pub fn apply_display_scale(normalized: f32, scale: &WidgetScale, _unit: Option<&str>) -> f32 {
    match scale {
        WidgetScale::Log => {
            // Log scale: spread out low values, compress high values
            (normalized.max(0.001) * 999.0 + 1.0).log10() / 3.0
        }
        WidgetScale::Exp => {
            // Exponential: compress low values, spread out high values
            normalized * normalized
        }
        WidgetScale::Lin => normalized,
    }
}

// ============================================================================
// Animation / Tooltip
// ============================================================================

/// State for fade-out animations (tooltips, indicators, etc.)
#[derive(Clone, Default)]
pub struct FadeState {
    last_trigger: Option<Instant>,
}

impl FadeState {
    pub fn trigger(&mut self) {
        self.last_trigger = Some(Instant::now());
    }

    /// Returns alpha value 0.0-1.0, or None if animation complete
    pub fn alpha(&self, hold_secs: f32, fade_secs: f32) -> Option<f32> {
        let elapsed = self.last_trigger?.elapsed().as_secs_f32();
        if elapsed < hold_secs {
            Some(1.0)
        } else if elapsed < hold_secs + fade_secs {
            Some(1.0 - (elapsed - hold_secs) / fade_secs)
        } else {
            None
        }
    }
}

/// State for value tooltip with fade animation
#[derive(Clone, Default)]
pub struct TooltipState {
    pub fade: FadeState,
    pub last_value: f32,
}

pub fn show_value_tooltip(
    ui: &mut egui::Ui,
    widget_id: egui::Id,
    value: f32,
    unit: Option<&str>,
    is_interacting: bool,
    widget_rect: egui::Rect,
) {
    const HOLD_SECS: f32 = 0.25;
    const FADE_SECS: f32 = 0.25;

    let tooltip_id = widget_id.with("value_tooltip");

    let mut state = ui
        .ctx()
        .memory_mut(|mem| mem.data.get_temp::<TooltipState>(tooltip_id).unwrap_or_default());

    if is_interacting {
        state.fade.trigger();
        state.last_value = value;
    }

    let alpha = state.fade.alpha(HOLD_SECS, FADE_SECS);

    ui.ctx()
        .memory_mut(|mem| mem.data.insert_temp(tooltip_id, state.clone()));

    if let Some(a) = alpha {
        ui.ctx().request_repaint();

        let text = match unit {
            Some(u) => format!("{:.2} {}", state.last_value, u),
            None => format!("{:.2}", state.last_value),
        };

        let tooltip_pos = widget_rect.center_top() - egui::vec2(0.0, 20.0);
        let text_color = Color32::WHITE.linear_multiply(a);
        let bg_color = Color32::from_rgba_unmultiplied(40, 40, 40, (200.0 * a) as u8);

        let galley = ui
            .painter()
            .layout_no_wrap(text, egui::FontId::default(), text_color);
        let rect = egui::Rect::from_center_size(tooltip_pos, galley.size() + egui::vec2(8.0, 4.0));
        ui.painter().rect_filled(rect, 4.0, bg_color);
        ui.painter()
            .galley(rect.min + egui::vec2(4.0, 2.0), galley, text_color);
    }
}
