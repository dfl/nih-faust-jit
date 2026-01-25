mod controls;
mod meter;

use egui::{Align, Layout, Sense};
use faust_jit::*;

use controls::{
    apply_display_scale, drag_sensitivity, draw_arc, show_value_tooltip,
    DRAG_FULL_RANGE_PX, SCROLL_FULL_RANGE,
};
use meter::{meter_colors, MeterState, GREEN_END, ORANGE_END};

// ============================================================================
// Helper Traits
// ============================================================================

trait DoIf: Sized {
    fn do_if(self, cond: bool, f: impl FnOnce(Self) -> Self) -> Self;
    fn do_if_some<T>(self, cond: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self;
}

impl<T> DoIf for T {
    fn do_if(self, cond: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if cond { f(self) } else { self }
    }

    fn do_if_some<X>(self, option: Option<X>, f: impl FnOnce(Self, X) -> Self) -> Self {
        match option {
            Some(x) => f(self, x),
            None => self,
        }
    }
}

// ============================================================================
// UI Helpers
// ============================================================================

fn hgroup_header_icon(ui: &mut egui::Ui, openness: f32, response: &egui::Response) {
    let stroke = ui.style().interact(&response).fg_stroke;
    let radius = 3.0;
    if openness == 0.0 {
        ui.painter()
            .circle_filled(response.rect.center(), radius, stroke.color);
    } else {
        ui.painter()
            .circle_stroke(response.rect.center(), radius, stroke);
    }
}

// ============================================================================
// Bargraph Rendering
// ============================================================================

fn draw_bargraph_with_state(
    ui: &mut egui::Ui,
    meter_id: egui::Id,
    display_t: f32,
    raw_t: f32,
    cur_val: f32,
    unit: &str,
    layout: &NumDisplayLayout,
) {
    let (clip_hold, smoothed_raw) = {
        let mut state = ui
            .ctx()
            .memory_mut(|mem| mem.data.get_temp::<MeterState>(meter_id).unwrap_or_default());
        state.update(raw_t);
        let hold = state.is_clipping();
        let peak = state.get_display_value();
        ui.ctx()
            .memory_mut(|mem| mem.data.insert_temp(meter_id, state));
        (hold, peak)
    };

    let t = if raw_t > 0.0 {
        (display_t * smoothed_raw / raw_t).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let (green, orange, red) = meter_colors();

    let max_size = match layout {
        NumDisplayLayout::Horizontal => egui::vec2(80.0, 20.0),
        NumDisplayLayout::Vertical => egui::vec2(20.0, 80.0),
    };

    let (rsp, painter) = ui.allocate_painter(max_size, Sense::hover());
    let rounding = egui::CornerRadius::same(3);

    painter.rect_filled(rsp.rect, rounding, ui.style().visuals.extreme_bg_color);

    match layout {
        NumDisplayLayout::Horizontal => {
            let width = rsp.rect.width();
            let min_x = rsp.rect.min.x;

            if t > 0.0 {
                let green_width = (t.min(GREEN_END) * width).max(0.0);
                if green_width > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            rsp.rect.min,
                            egui::pos2(min_x + green_width, rsp.rect.max.y),
                        ),
                        egui::CornerRadius::ZERO,
                        green,
                    );
                }
            }

            if t > GREEN_END {
                let orange_start = GREEN_END * width;
                let orange_width = ((t.min(ORANGE_END) - GREEN_END) * width).max(0.0);
                if orange_width > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(min_x + orange_start, rsp.rect.min.y),
                            egui::pos2(min_x + orange_start + orange_width, rsp.rect.max.y),
                        ),
                        egui::CornerRadius::ZERO,
                        orange,
                    );
                }
            }

            if t > ORANGE_END {
                let red_start = ORANGE_END * width;
                let red_width = ((t - ORANGE_END) * width).max(0.0);
                if red_width > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(min_x + red_start, rsp.rect.min.y),
                            egui::pos2(min_x + red_start + red_width, rsp.rect.max.y),
                        ),
                        egui::CornerRadius::ZERO,
                        red,
                    );
                }
            }
        }
        NumDisplayLayout::Vertical => {
            let height = rsp.rect.height();
            let max_y = rsp.rect.max.y;

            if t > 0.0 {
                let green_height = (t.min(GREEN_END) * height).max(0.0);
                if green_height > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rsp.rect.min.x, max_y - green_height),
                            rsp.rect.max,
                        ),
                        egui::CornerRadius::ZERO,
                        green,
                    );
                }
            }

            if t > GREEN_END {
                let orange_start = GREEN_END * height;
                let orange_height = ((t.min(ORANGE_END) - GREEN_END) * height).max(0.0);
                if orange_height > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rsp.rect.min.x, max_y - orange_start - orange_height),
                            egui::pos2(rsp.rect.max.x, max_y - orange_start),
                        ),
                        egui::CornerRadius::ZERO,
                        orange,
                    );
                }
            }

            if t > ORANGE_END {
                let red_start = ORANGE_END * height;
                let red_height = ((t - ORANGE_END) * height).max(0.0);
                if red_height > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(rsp.rect.min.x, max_y - red_start - red_height),
                            egui::pos2(rsp.rect.max.x, max_y - red_start),
                        ),
                        egui::CornerRadius::ZERO,
                        red,
                    );
                }
            }
        }
    }

    if clip_hold {
        let clip_rect = match layout {
            NumDisplayLayout::Horizontal => egui::Rect::from_min_max(
                egui::pos2(rsp.rect.max.x - 4.0, rsp.rect.min.y),
                egui::pos2(rsp.rect.max.x, rsp.rect.max.y),
            ),
            NumDisplayLayout::Vertical => egui::Rect::from_min_max(
                rsp.rect.min,
                egui::pos2(rsp.rect.max.x, rsp.rect.min.y + 4.0),
            ),
        };
        painter.rect_filled(clip_rect, rounding, red);
    }

    painter.rect_stroke(
        rsp.rect,
        rounding,
        egui::Stroke {
            width: 2.0,
            color: ui.style().interact(&rsp).fg_stroke.color,
        },
        egui::StrokeKind::Outside,
    );

    rsp.on_hover_text(format!("{:.2} {}", cur_val, unit));
}

// ============================================================================
// Main Widget Rendering
// ============================================================================

// Groups to skip entirely (hide from UI)
const SKIP_GROUPS: &[&str] = &["DSP2"];
// Groups to unwrap (render children directly without the group header)
const UNWRAP_GROUPS: &[&str] = &["Sequencer", "Polyphonic", "DSP1", "V1", "FaustDSP"];

/// Check if a label is an individual voice group V2, V3, etc. (skip all except V1)
fn is_voice_group_to_skip(label: &str) -> bool {
    if label.starts_with('V') && label.len() >= 2 {
        let rest = &label[1..];
        if rest.chars().all(|c| c.is_ascii_digit()) {
            // Skip V2, V3, ... but NOT V1
            return rest != "1";
        }
    }
    false
}

/// Check if a group should be skipped
fn should_skip(label: &str) -> bool {
    SKIP_GROUPS.contains(&label) || is_voice_group_to_skip(label)
}

fn faust_widgets_ui_rec(ui: &mut egui::Ui, widgets: &mut [DspWidget<&mut f32>], in_a_tab: bool) {
    for w in widgets {
        match w {
            DspWidget::Box {
                layout: BoxLayout::Tab { selected },
                label,
                inner,
                ..
            } => {
                // Skip groups we want to hide
                if should_skip(label) {
                    continue;
                }
                // Unwrap groups - render children directly without the tab UI
                if UNWRAP_GROUPS.contains(&label.as_str()) {
                    faust_widgets_ui_rec(ui, inner, in_a_tab);
                    continue;
                }
                let id = ui.make_persistent_id(&label);
                egui::collapsing_header::CollapsingState::load_with_default_open(
                    ui.ctx(),
                    id,
                    true,
                )
                .show_header(ui, |ui| {
                    ui.label(&*label);
                    for (idx, w) in inner.iter().enumerate() {
                        // Skip hidden groups in tab buttons too
                        if should_skip(w.label()) {
                            continue;
                        }
                        let btn = egui::Button::new(w.label())
                            .do_if(*selected == idx, |s| s.fill(egui::Color32::DARK_BLUE));
                        if ui.add(btn).clicked() {
                            *selected = idx;
                        }
                    }
                })
                .body(|ui| {
                    faust_widgets_ui_rec(ui, &mut inner[*selected..=*selected], true);
                });
            }
            DspWidget::Box {
                layout,
                label,
                inner,
                ..
            } => {
                // Skip groups we want to hide
                if should_skip(label) {
                    continue;
                }
                // Unwrap groups - render children directly
                if UNWRAP_GROUPS.contains(&label.as_str()) {
                    faust_widgets_ui_rec(ui, inner, in_a_tab);
                    continue;
                }
                let egui_layout = match layout {
                    BoxLayout::Horizontal => Layout::left_to_right(Align::Min),
                    BoxLayout::Vertical => Layout::top_down(Align::Min),
                    _ => panic!("Cannot be Tab here"),
                };
                let mut draw_inner = |ui: &mut egui::Ui| {
                    ui.with_layout(egui_layout, |ui| faust_widgets_ui_rec(ui, inner, false))
                };
                if in_a_tab || label.is_empty() {
                    draw_inner(ui);
                } else {
                    egui::CollapsingHeader::new(&*label)
                        .default_open(true)
                        .do_if(*layout == BoxLayout::Horizontal, |s| {
                            s.icon(hgroup_header_icon)
                        })
                        .show(ui, draw_inner);
                }
            }
            DspWidget::BoolParam {
                layout,
                label,
                zone,
                hidden: false,
                tooltip,
                ..
            } => {
                let resp = match layout {
                    BoolParamLayout::Held => {
                        let button = egui::Button::new(&*label)
                            .sense(Sense::drag().union(Sense::hover()))
                            .do_if(**zone != 0.0, |s| {
                                s.fill(egui::Color32::from_rgb(115, 115, 50))
                            });
                        let resp = ui.add(button);
                        if resp.drag_started() {
                            **zone = 1.0;
                        } else if resp.drag_stopped() {
                            **zone = 0.0;
                        }
                        resp
                    }
                    BoolParamLayout::Checkbox => {
                        let mut selected = **zone != 0.0;
                        let resp = ui.checkbox(&mut selected, &*label).interact(Sense::hover());
                        **zone = selected as i32 as f32;
                        resp
                    }
                };
                if let Some(txt) = tooltip {
                    resp.on_hover_text(txt.to_owned());
                }
            }
            DspWidget::NumParam {
                layout,
                style,
                label,
                zone,
                min,
                max,
                step,
                init,
                metadata:
                    NumMetadata {
                        unit,
                        scale,
                        hidden: false,
                        tooltip,
                        ..
                    },
            } => {
                let rng = std::ops::RangeInclusive::new(*min, *max);
                ui.vertical(|ui| {
                    if !label.is_empty() {
                        let resp = ui
                            .label(&*label)
                            .interact(Sense::click().union(Sense::hover()))
                            .do_if_some(tooltip.as_deref(), |s, tooltip| {
                                s.on_hover_text(tooltip.to_owned())
                            });
                        if resp.double_clicked() {
                            **zone = *init;
                        }
                    }
                    match (layout, style) {
                        (_, NumParamStyle::Knob) => {
                            let knob_radius = 24.0;
                            let (response, painter) = ui.allocate_painter(
                                egui::vec2(knob_radius * 2.0 + 8.0, knob_radius * 2.0 + 8.0),
                                Sense::click_and_drag(),
                            );

                            let center = response.rect.center();
                            let knob_id = ui.make_persistent_id(label);
                            let t = (**zone - *min) / (*max - *min);

                            const MAX_ANGLE_DEG: f32 = 143.0;
                            let start_angle_rad = (-MAX_ANGLE_DEG).to_radians();
                            let end_angle_rad = MAX_ANGLE_DEG.to_radians();
                            let value_angle_rad =
                                start_angle_rad + t * (end_angle_rad - start_angle_rad);

                            let stroke_width = 4.0;
                            let arc_radius = knob_radius - stroke_width / 2.0;

                            draw_arc(
                                &painter,
                                center,
                                arc_radius,
                                start_angle_rad,
                                end_angle_rad,
                                egui::Stroke::new(stroke_width, egui::Color32::DARK_GRAY),
                            );

                            let value_color = egui::Color32::from_rgb(100, 150, 255);
                            draw_arc(
                                &painter,
                                center,
                                arc_radius,
                                start_angle_rad,
                                value_angle_rad,
                                egui::Stroke::new(stroke_width, value_color),
                            );

                            painter.circle_filled(
                                center,
                                arc_radius - stroke_width - 2.0,
                                egui::Color32::from_gray(60),
                            );

                            let pointer_length = arc_radius - 4.0;
                            let pointer_end = center
                                + egui::vec2(
                                    value_angle_rad.sin() * pointer_length,
                                    -value_angle_rad.cos() * pointer_length,
                                );
                            painter.line_segment(
                                [center, pointer_end],
                                egui::Stroke::new(2.0, egui::Color32::WHITE),
                            );

                            if response.dragged() {
                                let delta = -response.drag_delta().y;
                                let sensitivity = drag_sensitivity(ui);
                                let range = *max - *min;
                                let change = (delta * sensitivity * range) / DRAG_FULL_RANGE_PX;
                                **zone = (**zone + change).clamp(*min, *max);
                            }

                            let mut scrolling = false;
                            if response.hovered() {
                                let scroll_delta = ui.input(|i| i.raw_scroll_delta);
                                if scroll_delta.y != 0.0 {
                                    scrolling = true;
                                    let sensitivity = drag_sensitivity(ui);
                                    let range = *max - *min;
                                    let change =
                                        (scroll_delta.y * sensitivity * range) / SCROLL_FULL_RANGE;
                                    **zone = (**zone + change).clamp(*min, *max);
                                }
                            }

                            if response.double_clicked() {
                                **zone = *init;
                            }

                            show_value_tooltip(
                                ui,
                                knob_id,
                                **zone,
                                unit.as_deref(),
                                response.dragged() || scrolling,
                                response.rect,
                            );

                            if let Some(txt) = tooltip {
                                response.on_hover_text(txt.to_owned());
                            }
                        }
                        (_, NumParamStyle::Menu(vals)) => {
                            // Sync UI selection with zone value (e.g., after preset load)
                            let zone_val = **zone;
                            if vals.options.get(vals.selected).map(|(_, v)| *v) != Some(zone_val) {
                                if let Some(pos) = vals.options.iter().position(|(_, v)| (*v - zone_val).abs() < 0.001) {
                                    vals.selected = pos;
                                }
                            }
                            egui::ComboBox::from_id_salt(&*label)
                                .selected_text(vals.options[vals.selected].0.clone())
                                .show_ui(ui, |ui| {
                                    for (i, (k, _)) in vals.options.iter().enumerate() {
                                        ui.selectable_value(&mut vals.selected, i, k);
                                    }
                                });
                            **zone = vals.options[vals.selected].1
                        }
                        (layout, NumParamStyle::Radio(vals)) => {
                            // Sync UI selection with zone value (e.g., after preset load)
                            let zone_val = **zone;
                            if vals.options.get(vals.selected).map(|(_, v)| *v) != Some(zone_val) {
                                if let Some(pos) = vals.options.iter().position(|(_, v)| (*v - zone_val).abs() < 0.001) {
                                    vals.selected = pos;
                                }
                            }
                            let egui_layout = match layout {
                                NumParamLayout::VerticalSlider => Layout::top_down(Align::Min),
                                _ => Layout::left_to_right(Align::Min),
                            };
                            ui.with_layout(egui_layout, |ui| {
                                for (i, (k, _)) in vals.options.iter().enumerate() {
                                    ui.radio_value(&mut vals.selected, i, k);
                                }
                            });
                            **zone = vals.options[vals.selected].1
                        }
                        (NumParamLayout::NumEntry, _) => {
                            ui.add(
                                egui::DragValue::new(*zone)
                                    .range(rng)
                                    .do_if_some(unit.as_deref(), |s, unit| s.suffix(unit)),
                            );
                        }
                        (layout, _) => {
                            ui.add(
                                egui::Slider::new(*zone, rng)
                                    .step_by(*step as f64)
                                    .do_if_some(unit.as_deref(), |s, unit| s.suffix(unit))
                                    .do_if(*layout == NumParamLayout::VerticalSlider, |s| {
                                        s.vertical()
                                    })
                                    .do_if(*scale == WidgetScale::Log, |s| s.logarithmic(true))
                                    .do_if(*scale == WidgetScale::Exp, |s| s.logarithmic(true)),
                            );
                        }
                    };
                });
            }
            DspWidget::NumDisplay {
                layout,
                style,
                label,
                zone,
                min,
                max,
                metadata:
                    NumMetadata {
                        unit,
                        scale,
                        hidden: false,
                        tooltip,
                        ..
                    },
            } => {
                let cur_val = **zone;
                let raw_t = (cur_val - *min) / (*max - *min);
                let display_t = apply_display_scale(raw_t.clamp(0.0, 1.0), scale, unit.as_deref());
                let unit_or_empty = unit.as_deref().unwrap_or("");
                let meter_id = ui.make_persistent_id(&*label);

                ui.vertical(|ui| {
                    let label_width = if !label.is_empty() {
                        let resp = ui
                            .label(&*label)
                            .interact(Sense::hover())
                            .do_if_some(tooltip.as_deref(), |s, tooltip| {
                                s.on_hover_text(tooltip.to_owned())
                            });
                        resp.rect.max.x - resp.rect.min.x
                    } else {
                        30.0
                    };
                    match (layout, style) {
                        (_, NumDisplayStyle::Led) => {
                            let led_radius = 10.0;
                            let (response, painter) = ui.allocate_painter(
                                egui::vec2(led_radius * 2.0 + 4.0, led_radius * 2.0 + 4.0),
                                Sense::hover(),
                            );

                            let center = response.rect.center();

                            let color = {
                                let mut state = ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .get_temp::<MeterState>(meter_id)
                                        .unwrap_or_default()
                                });
                                let c = state.update(raw_t);
                                ui.ctx()
                                    .memory_mut(|mem| mem.data.insert_temp(meter_id, state));
                                c
                            };

                            painter.circle_filled(
                                center,
                                led_radius + 2.0,
                                color.linear_multiply(0.3),
                            );
                            painter.circle_filled(center, led_radius, color);
                            painter.circle_stroke(
                                center,
                                led_radius,
                                egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
                            );

                            response.on_hover_text(format!("{:.2} {}", cur_val, unit_or_empty));
                        }
                        (_, NumDisplayStyle::Numerical) => {
                            let color = {
                                let mut state = ui.ctx().memory_mut(|mem| {
                                    mem.data
                                        .get_temp::<MeterState>(meter_id)
                                        .unwrap_or_default()
                                });
                                let c = state.update(raw_t);
                                ui.ctx()
                                    .memory_mut(|mem| mem.data.insert_temp(meter_id, state));
                                c
                            };
                            ui.colored_label(color, format!("{:.2} {}", cur_val, unit_or_empty));
                        }
                        (NumDisplayLayout::Horizontal, _) => {
                            ui.horizontal(|ui| {
                                ui.label(format!("{:.2}", min));
                                draw_bargraph_with_state(
                                    ui,
                                    meter_id,
                                    display_t,
                                    raw_t,
                                    cur_val,
                                    unit_or_empty,
                                    &NumDisplayLayout::Horizontal,
                                );
                                ui.label(format!("{:.2}{}", max, unit_or_empty));
                            });
                        }
                        (NumDisplayLayout::Vertical, _) => {
                            ui.allocate_ui_with_layout(
                                egui::vec2(label_width, 100.0),
                                Layout::top_down(Align::Center),
                                |ui| {
                                    ui.label(format!("{:.2}{}", max, unit_or_empty));
                                    draw_bargraph_with_state(
                                        ui,
                                        meter_id,
                                        display_t,
                                        raw_t,
                                        cur_val,
                                        unit_or_empty,
                                        &NumDisplayLayout::Vertical,
                                    );
                                    ui.label(format!("{:.2}", min));
                                },
                            );
                        }
                    };
                });
            }
            _ => {}
        }
    }
}

/// Draw and update the faust widgets inside an egui::Ui
pub fn faust_widgets_ui(ui: &mut egui::Ui, widgets: &mut [DspWidget<&mut f32>]) {
    faust_widgets_ui_rec(ui, widgets, false);
    // Sync V1 parameters to V2-V8 for unified control
    sync_voice_parameters(widgets);
}

/// Sync parameters from V1 to V2, V3, etc. so all voices use the same values
fn sync_voice_parameters(widgets: &mut [DspWidget<&mut f32>]) {
    // First, collect V1's parameter values
    let mut v1_values: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
    collect_voice_params(widgets, "V1", "", &mut v1_values);

    // Then apply to V2, V3, etc.
    for voice_num in 2..=16 {
        let voice_label = format!("V{}", voice_num);
        apply_voice_params(widgets, &voice_label, "", &v1_values);
    }
}

/// Recursively collect parameter values from a specific voice
fn collect_voice_params(
    widgets: &[DspWidget<&mut f32>],
    target_voice: &str,
    path: &str,
    values: &mut std::collections::HashMap<String, f32>,
) {
    for widget in widgets {
        match widget {
            DspWidget::Box { label, inner, .. } => {
                let new_path = if path.is_empty() {
                    label.clone()
                } else {
                    format!("{}/{}", path, label)
                };

                if label == target_voice {
                    // We're inside the target voice - collect all params within
                    collect_all_params(inner, "", values);
                } else {
                    // Keep searching
                    collect_voice_params(inner, target_voice, &new_path, values);
                }
            }
            _ => {}
        }
    }
}

/// Collect all parameters within a widget tree (relative paths)
fn collect_all_params(
    widgets: &[DspWidget<&mut f32>],
    path: &str,
    values: &mut std::collections::HashMap<String, f32>,
) {
    for widget in widgets {
        let label = widget.label();
        // Skip MIDI-controlled parameters to preserve polyphony
        if label == "freq" || label == "gate" || label == "gain" {
            continue;
        }

        let widget_path = if path.is_empty() {
            label.to_string()
        } else {
            format!("{}/{}", path, label)
        };

        match widget {
            DspWidget::Box { inner, .. } => {
                collect_all_params(inner, &widget_path, values);
            }
            DspWidget::NumParam { zone, .. } => {
                values.insert(widget_path, **zone);
            }
            DspWidget::BoolParam { zone, .. } => {
                values.insert(widget_path, **zone);
            }
            _ => {}
        }
    }
}

/// Apply parameter values to a specific voice
fn apply_voice_params(
    widgets: &mut [DspWidget<&mut f32>],
    target_voice: &str,
    path: &str,
    values: &std::collections::HashMap<String, f32>,
) {
    for widget in widgets {
        match widget {
            DspWidget::Box { label, inner, .. } => {
                let new_path = if path.is_empty() {
                    label.clone()
                } else {
                    format!("{}/{}", path, label)
                };

                if label == target_voice {
                    // We're inside the target voice - apply all params
                    apply_all_params(inner, "", values);
                } else {
                    // Keep searching
                    apply_voice_params(inner, target_voice, &new_path, values);
                }
            }
            _ => {}
        }
    }
}

/// Apply parameter values within a widget tree
fn apply_all_params(
    widgets: &mut [DspWidget<&mut f32>],
    path: &str,
    values: &std::collections::HashMap<String, f32>,
) {
    for widget in widgets {
        let label = widget.label();
        // Skip MIDI-controlled parameters to preserve polyphony
        if label == "freq" || label == "gate" || label == "gain" {
            continue;
        }

        let widget_path = if path.is_empty() {
            label.to_string()
        } else {
            format!("{}/{}", path, label)
        };

        match widget {
            DspWidget::Box { inner, .. } => {
                apply_all_params(inner, &widget_path, values);
            }
            DspWidget::NumParam { zone, .. } => {
                if let Some(&value) = values.get(&widget_path) {
                    **zone = value;
                }
            }
            DspWidget::BoolParam { zone, .. } => {
                if let Some(&value) = values.get(&widget_path) {
                    **zone = value;
                }
            }
            _ => {}
        }
    }
}
