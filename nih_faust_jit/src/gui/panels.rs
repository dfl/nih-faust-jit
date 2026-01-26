use nih_plug::prelude::*;
use nih_plug_egui::egui;
use egui_plot::{Line, Plot, PlotPoints};
use std::sync::{atomic::Ordering, Arc};

use crate::{audio_file, config, oversampling::OversamplingFactor, save_widget_values, testbench, DspState, DspType, Tasks};
use std::collections::HashMap;
use super::{GuiArcs, GuiState};

pub(crate) fn enum_combobox<T: strum::IntoEnumIterator + PartialEq + std::fmt::Debug>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    selected: &mut T,
) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(format!("{:?}", selected))
        .show_ui(ui, |ui| {
            for variant in <T as strum::IntoEnumIterator>::iter() {
                let s = format!("{:?}", variant);
                ui.selectable_value(selected, variant, s);
            }
        });
}

pub(super) fn top_panel_contents(
    ui: &mut egui::Ui,
    arcs: &GuiArcs,
    async_executor: &AsyncExecutor<crate::NihFaustJit>,
    gui_state: &mut GuiState,
) {
    // DSP Settings section
    dsp_settings_panel(ui, arcs, async_executor, gui_state);

    // Presets section
    presets_panel(ui, arcs, async_executor, gui_state);

    // Test Input section (standalone mode only - in DAW, use DAW's audio routing)
    if arcs.is_standalone.load(Ordering::Relaxed) {
        test_input_panel(ui, arcs, async_executor, gui_state);
    }

    // Testbench section
    testbench_panel(ui, arcs);

    // Device Settings section
    device_settings_panel(ui, gui_state);
}

fn dsp_settings_panel(
    ui: &mut egui::Ui,
    arcs: &GuiArcs,
    async_executor: &AsyncExecutor<crate::NihFaustJit>,
    gui_state: &mut GuiState,
) {
    let selected_paths = arcs.selected_paths.read().unwrap();
    let script_path = selected_paths.dsp_script.clone();
    let lib_path = selected_paths.dsp_lib_path.clone();
    drop(selected_paths);

    // Row 1: Script path, Load, Reload, Edit buttons
    ui.horizontal(|ui| {
        ui.label("DSP:");
        match &script_path {
            Some(path) => {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
                ui.label(name);
            }
            None => {
                ui.colored_label(egui::Color32::GRAY, "(none)");
            }
        }

        if ui.add(egui::Button::new("Load").fill(egui::Color32::DARK_GRAY)).clicked() {
            let pending = Arc::clone(&gui_state.pending_script_path);
            let start_path = script_path.clone().unwrap_or_else(|| lib_path.clone());
            std::thread::spawn(move || {
                if let Some(path) = rfd::FileDialog::new()
                    .set_directory(&start_path)
                    .add_filter("Faust DSP", &["dsp"])
                    .pick_file()
                {
                    *pending.write().unwrap() = Some(path);
                }
            });
        }

        if script_path.is_some() {
            if ui.button("Reload").clicked() {
                async_executor.execute_background(crate::Tasks::ReloadDsp);
            }
            if ui.button("Edit").clicked() {
                if let Some(path) = &script_path {
                    let _ = open::that(path);
                }
            }
        }

        ui.separator();

        // DSP type selector
        let mut nvoices = *arcs.dsp_nvoices.read().unwrap();
        let mut selected_dsp_type = DspType::from_nvoices(nvoices);
        let last_dsp_type = selected_dsp_type;
        ui.label("Type:");
        enum_combobox(ui, "dsp-type-combobox", &mut selected_dsp_type);
        match selected_dsp_type {
            DspType::AutoDetect => nvoices = -1,
            DspType::Effect => nvoices = 0,
            DspType::Instrument => {
                if selected_dsp_type != last_dsp_type { nvoices = 1; }
                ui.add(egui::Slider::new(&mut nvoices, 1..=32).text("voices"));
            }
        }
        *arcs.dsp_nvoices.write().unwrap() = nvoices;

        ui.separator();

        // Oversampling selector
        let current_os = arcs.pending_oversampling.load(Ordering::Relaxed);
        let actual_os = arcs.oversampling.load(Ordering::Relaxed);
        let mut selected_oversampling = OversamplingFactor::from_factor(current_os);
        let last_oversampling = selected_oversampling;
        ui.label("Oversample:");
        enum_combobox(ui, "oversampling-combobox", &mut selected_oversampling);
        if current_os != actual_os {
            ui.spinner();
        }
        if selected_oversampling != last_oversampling {
            arcs.pending_oversampling.store(selected_oversampling.factor() as u8, Ordering::Relaxed);
            async_executor.execute_background(Tasks::ReloadDsp);
        }
    });
}

fn presets_panel(
    ui: &mut egui::Ui,
    arcs: &GuiArcs,
    async_executor: &AsyncExecutor<crate::NihFaustJit>,
    gui_state: &mut GuiState,
) {
    let dsp_script = arcs.selected_paths.read().unwrap().dsp_script.clone();
    if dsp_script.is_none() {
        return;
    }

    let script_path = dsp_script.unwrap();
    let dsp_name = script_path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown");

    // Update preset cache if DSP changed
    if gui_state.cached_dsp_name.as_deref() != Some(dsp_name) {
        gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
        gui_state.cached_dsp_name = Some(dsp_name.to_string());
        gui_state.selected_preset = None;

        // Auto-load "Default" preset if it exists
        if gui_state.cached_presets.contains(&"Default".to_string()) {
            if let Ok(preset) = gui_state.preset_manager.load_preset(dsp_name, "Default") {
                gui_state.selected_preset = Some("Default".to_string());
                arcs.preset_loading.store(true, Ordering::SeqCst);
                // Merge preset values, preserving nopre parameters
                arcs.faust_param_values.write().unwrap().extend(preset.values);
                async_executor.execute_background(crate::Tasks::ReloadDsp);
            }
        }
    }

    ui.horizontal(|ui| {
        ui.label("Preset:");
        egui::ComboBox::from_id_salt("preset-selector")
            .selected_text(gui_state.selected_preset.clone().unwrap_or_else(|| "-".to_string()))
            .show_ui(ui, |ui| {
                for preset_name in &gui_state.cached_presets {
                    let is_selected = gui_state.selected_preset.as_ref() == Some(preset_name);
                    if ui.selectable_label(is_selected, preset_name).clicked() && !is_selected {
                        gui_state.selected_preset = Some(preset_name.clone());
                        if let Ok(preset) = gui_state.preset_manager.load_preset(dsp_name, preset_name) {
                            arcs.preset_loading.store(true, Ordering::SeqCst);
                            // Merge preset values, preserving nopre parameters
                            arcs.faust_param_values.write().unwrap().extend(preset.values);
                            async_executor.execute_background(crate::Tasks::ReloadDsp);
                        }
                    }
                }
            });

        let delete_btn = ui.add_enabled(gui_state.selected_preset.is_some(), egui::Button::new("Del"));
        let popup_id = ui.make_persistent_id("delete-preset-confirm");
        if delete_btn.clicked() {
            gui_state.confirm_delete_preset = true;
        }
        if gui_state.confirm_delete_preset {
            egui::Area::new(popup_id)
                .order(egui::Order::Foreground)
                .fixed_pos(delete_btn.rect.left_bottom())
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label("Delete?");
                        ui.horizontal(|ui| {
                            if ui.button("Yes").clicked() {
                                if let Some(preset_name) = &gui_state.selected_preset {
                                    if gui_state.preset_manager.delete_preset(dsp_name, preset_name).is_ok() {
                                        gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
                                        gui_state.selected_preset = None;
                                    }
                                }
                                gui_state.confirm_delete_preset = false;
                            }
                            if ui.button("No").clicked() {
                                gui_state.confirm_delete_preset = false;
                            }
                        });
                    });
                });
        }

        ui.separator();
        ui.label("Save:");
        ui.add(egui::TextEdit::singleline(&mut gui_state.save_preset_name).desired_width(100.0));
        if ui.add_enabled(!gui_state.save_preset_name.is_empty(), egui::Button::new("Save")).clicked() {
            // Collect values from widgets, excluding [nopre:1] parameters
            let values = if let DspState::Loaded(dsp) = &*arcs.dsp_state.read().unwrap() {
                let mut preset_values = HashMap::new();
                dsp.with_widgets(|widgets| {
                    save_widget_values(widgets, &mut preset_values, "", true);
                });
                preset_values
            } else {
                arcs.faust_param_values.read().unwrap().clone()
            };
            if gui_state.preset_manager.save_preset(dsp_name, &gui_state.save_preset_name, values).is_ok() {
                gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
                gui_state.selected_preset = Some(gui_state.save_preset_name.clone());
                gui_state.save_preset_name.clear();
            }
        }
    });
}

pub(super) fn test_input_panel(
    ui: &mut egui::Ui,
    arcs: &GuiArcs,
    async_executor: &AsyncExecutor<crate::NihFaustJit>,
    gui_state: &mut GuiState,
) {
    ui.collapsing("Test Input", |ui| {
        // Check if main DSP is an effect (has inputs)
        let main_dsp_is_effect = if let DspState::Loaded(dsp) = &*arcs.dsp_state.read().unwrap() {
            dsp.info.num_inputs > 0
        } else {
            false
        };

        // Audio File section
        let player = arcs.audio_file_player.read().unwrap();
        let audio_info = player.get_audio_info();
        drop(player);

        let (has_audio, total_frames, sample_rate, filename) = match &audio_info {
            Some(audio) => {
                let name = audio.file_path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                (true, audio.total_frames, audio.sample_rate, Some(name))
            }
            None => (false, 0, 48000, None),
        };

        let test_signal_loaded = arcs.test_signal_dsp.read().unwrap().is_some();
        let test_signal_enabled = arcs.test_signal_enabled.load(Ordering::Relaxed);

        // First row: Audio load + Test signal generator button
        ui.horizontal(|ui| {
            // Audio file load button
            if ui.add_enabled(main_dsp_is_effect, egui::Button::new("Load Audio")).clicked() {
                let pending = Arc::clone(&gui_state.pending_audio_file);
                std::thread::spawn(move || {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Audio Files", &["wav", "mp3", "flac", "ogg", "m4a", "aac"])
                        .pick_file()
                    {
                        *pending.write().unwrap() = Some(path);
                    }
                });
            }

            if let Some(name) = &filename {
                ui.label(name);
            } else {
                ui.colored_label(egui::Color32::GRAY, "(none)");
            }

            ui.separator();

            // Test signal generator controls
            if test_signal_loaded {
                if ui.button("Unload Generator").clicked() {
                    async_executor.execute_background(crate::Tasks::UnloadTestSignals);
                }
                let mut enabled = test_signal_enabled;
                if ui.checkbox(&mut enabled, "Inject").changed() {
                    arcs.test_signal_enabled.store(enabled, Ordering::Relaxed);
                }
            } else {
                if ui.add_enabled(main_dsp_is_effect, egui::Button::new("Load Generator")).clicked() {
                    async_executor.execute_background(crate::Tasks::LoadTestSignals);
                }
            }

            if !main_dsp_is_effect {
                ui.colored_label(egui::Color32::GRAY, "(effect DSP required)");
            }
        });

        // Audio playback controls (if audio loaded)
        if has_audio {
            ui.horizontal(|ui| {
                let playing = arcs.playback_state.playing.load(Ordering::Relaxed);
                if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                    arcs.playback_state.playing.store(!playing, Ordering::Relaxed);
                }
                if ui.button("Stop").clicked() {
                    arcs.playback_state.playing.store(false, Ordering::Relaxed);
                    arcs.playback_state.position.store(0, Ordering::Relaxed);
                }
                let mut looping = arcs.playback_state.looping.load(Ordering::Relaxed);
                if ui.checkbox(&mut looping, "Loop").changed() {
                    arcs.playback_state.looping.store(looping, Ordering::Relaxed);
                }

                let pos = arcs.playback_state.position.load(Ordering::Relaxed);
                let sr = sample_rate as f64;
                let pos_secs = pos as f64 / sr;
                let total_secs = total_frames as f64 / sr;
                ui.label(format!("{}/{}", audio_file::format_duration(pos_secs), audio_file::format_duration(total_secs)));

                let mut pos_f = if total_frames > 0 { pos as f32 / total_frames as f32 } else { 0.0 };
                if ui.add(egui::Slider::new(&mut pos_f, 0.0..=1.0).show_value(false)).changed() {
                    arcs.playback_state.position.store((pos_f * total_frames as f32) as u64, Ordering::Relaxed);
                }
            });
        }

        // Show test signal DSP widgets when loaded
        if test_signal_loaded {
            if let Ok(test_dsp_guard) = arcs.test_signal_dsp.try_read() {
                if let Some(test_dsp) = test_dsp_guard.as_ref() {
                    ui.indent("test_signal_widgets", |ui| {
                        test_dsp.with_widgets_mut(|widgets| {
                            faust_jit_egui::faust_widgets_ui(ui, widgets);
                        });
                    });
                }
            }
        }
    });
}

pub(super) fn device_settings_panel(ui: &mut egui::Ui, gui_state: &mut GuiState) {
    ui.collapsing("Audio/MIDI Settings", |ui| {
        if !gui_state.devices_enumerated {
            let (inputs, outputs) = config::enumerate_audio_devices();
            gui_state.audio_inputs = inputs;
            gui_state.audio_outputs = outputs;
            gui_state.midi_inputs = config::enumerate_midi_inputs();
            gui_state.devices_enumerated = true;
        }

        if ui.button("Refresh Devices").clicked() {
            let (inputs, outputs) = config::enumerate_audio_devices();
            gui_state.audio_inputs = inputs;
            gui_state.audio_outputs = outputs;
            gui_state.midi_inputs = config::enumerate_midi_inputs();
        }

        ui.add_space(4.0);

        // Output device
        ui.horizontal(|ui| {
            ui.label("Output:");
            let current_output = gui_state
                .device_config
                .output_device
                .clone()
                .unwrap_or_else(|| "(Default)".to_string());

            egui::ComboBox::from_id_salt("output-device")
                .selected_text(&current_output)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            gui_state.device_config.output_device.is_none(),
                            "(Default)",
                        )
                        .clicked()
                    {
                        gui_state.device_config.output_device = None;
                        gui_state.config_modified = true;
                    }
                    for device in &gui_state.audio_outputs {
                        let is_selected = gui_state.device_config.output_device.as_ref() == Some(device);
                        if ui.selectable_label(is_selected, device).clicked() {
                            gui_state.device_config.output_device = Some(device.clone());
                            gui_state.config_modified = true;
                        }
                    }
                });
        });

        // Input device
        ui.horizontal(|ui| {
            ui.label("Input:");
            let current_input = gui_state
                .device_config
                .input_device
                .clone()
                .unwrap_or_else(|| "(None)".to_string());

            egui::ComboBox::from_id_salt("input-device")
                .selected_text(&current_input)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            gui_state.device_config.input_device.is_none(),
                            "(None)",
                        )
                        .clicked()
                    {
                        gui_state.device_config.input_device = None;
                        gui_state.config_modified = true;
                    }
                    for device in &gui_state.audio_inputs {
                        let is_selected = gui_state.device_config.input_device.as_ref() == Some(device);
                        if ui.selectable_label(is_selected, device).clicked() {
                            gui_state.device_config.input_device = Some(device.clone());
                            gui_state.config_modified = true;
                        }
                    }
                });
        });

        // MIDI input
        ui.horizontal(|ui| {
            ui.label("MIDI In:");
            let current_midi = gui_state
                .device_config
                .midi_input
                .clone()
                .unwrap_or_else(|| "(None)".to_string());

            egui::ComboBox::from_id_salt("midi-input")
                .selected_text(&current_midi)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(gui_state.device_config.midi_input.is_none(), "(None)")
                        .clicked()
                    {
                        gui_state.device_config.midi_input = None;
                        gui_state.config_modified = true;
                    }
                    for device in &gui_state.midi_inputs {
                        let is_selected = gui_state.device_config.midi_input.as_ref() == Some(device);
                        if ui.selectable_label(is_selected, device).clicked() {
                            gui_state.device_config.midi_input = Some(device.clone());
                            gui_state.config_modified = true;
                        }
                    }
                });
        });

        // Sample rate
        ui.horizontal(|ui| {
            ui.label("Sample Rate:");
            let current_sr = gui_state
                .device_config
                .sample_rate
                .map(|sr| format!("{} Hz", sr))
                .unwrap_or_else(|| "(Default 48000)".to_string());

            egui::ComboBox::from_id_salt("sample-rate")
                .selected_text(&current_sr)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(
                            gui_state.device_config.sample_rate.is_none(),
                            "(Default 48000)",
                        )
                        .clicked()
                    {
                        gui_state.device_config.sample_rate = None;
                        gui_state.config_modified = true;
                    }
                    for sr in [44100u32, 48000, 88200, 96000] {
                        let label = format!("{} Hz", sr);
                        let is_selected = gui_state.device_config.sample_rate == Some(sr);
                        if ui.selectable_label(is_selected, &label).clicked() {
                            gui_state.device_config.sample_rate = Some(sr);
                            gui_state.config_modified = true;
                        }
                    }
                });
            ui.label("(Use 44100 for Bluetooth)").on_hover_text(
                "Many Bluetooth devices don't support 48000 Hz.\nIf you hear no audio with Bluetooth, try 44100 Hz."
            );
        });

        ui.add_space(4.0);

        if gui_state.config_modified {
            ui.horizontal(|ui| {
                if ui.button("Save Settings").clicked() {
                    if let Err(e) = gui_state.device_config.save() {
                        nih_plug::log::log!(nih_plug::log::Level::Error, "Failed to save config: {}", e);
                    } else {
                        gui_state.config_modified = false;
                    }
                }
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Restart app to apply device changes",
                );
            });
        }
    });
}

pub(super) fn testbench_panel(ui: &mut egui::Ui, arcs: &GuiArcs) {
    egui::CollapsingHeader::new("Testbench")
        .default_open(true)
        .show(ui, |ui| {
        // Check if main DSP is an effect (for comparison feature)
        let main_dsp_is_effect = if let DspState::Loaded(dsp) = &*arcs.dsp_state.read().unwrap() {
            dsp.info.num_inputs > 0
        } else {
            false
        };

        // Try to lock the testbench GUI state
        let mut testbench_gui = match arcs.testbench_gui.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                ui.label("Testbench busy...");
                return;
            }
        };

        // Update visualization from ring buffer data
        // Pause/play controls visualization for both instruments and effects
        // (playing defaults to true, so visualization shows immediately)
        let is_playing = arcs.playback_state.playing.load(Ordering::Relaxed);
        if is_playing {
            testbench_gui.update();
        }

        // Tab selection
        ui.horizontal(|ui| {
            ui.selectable_value(&mut testbench_gui.state.selected_tab, 0, "Oscilloscope");
            ui.selectable_value(&mut testbench_gui.state.selected_tab, 1, "Spectrum");
            ui.selectable_value(&mut testbench_gui.state.selected_tab, 2, "ERB Bands");
            ui.selectable_value(&mut testbench_gui.state.selected_tab, 3, "Benchmark");
        });

        ui.separator();

        match testbench_gui.state.selected_tab {
            0 => oscilloscope_panel(ui, &mut testbench_gui.state.oscilloscope),
            1 => spectrum_panel(ui, &mut testbench_gui.state, main_dsp_is_effect),
            2 => erb_panel(ui, &mut testbench_gui.state, main_dsp_is_effect),
            3 => benchmark_panel(ui, &arcs.testbench_metrics),
            _ => {}
        }
    });
}

fn oscilloscope_panel(ui: &mut egui::Ui, osc: &mut testbench::OscilloscopeState) {
    // Controls
    ui.horizontal(|ui| {
        ui.label("Time:");
        // Time scale as milliseconds, converted to/from samples
        let mut time_ms = osc.display_samples as f32 / osc.sample_rate * 1000.0;
        if ui.add(egui::Slider::new(&mut time_ms, 0.5..=100.0).logarithmic(true).suffix(" ms")).changed() {
            osc.display_samples = ((time_ms / 1000.0) * osc.sample_rate) as usize;
            osc.display_samples = osc.display_samples.clamp(64, 8192);
        }
        ui.label("Vertical:");
        ui.add(egui::Slider::new(&mut osc.vertical_scale, 0.1..=10.0).logarithmic(true));
        ui.checkbox(&mut osc.trigger_enabled, "Trigger");
        if osc.trigger_enabled {
            ui.add(egui::Slider::new(&mut osc.trigger_level, -1.0..=1.0).text("Level"));
        }
    });

    // Convert samples to plot points (x-axis in milliseconds)
    let samples_to_ms = 1000.0 / osc.sample_rate;
    let left_points: PlotPoints = osc
        .left_buffer
        .iter()
        .enumerate()
        .map(|(i, &y)| [(i as f32 * samples_to_ms) as f64, (y * osc.vertical_scale) as f64])
        .collect();

    let right_points: PlotPoints = osc
        .right_buffer
        .iter()
        .enumerate()
        .map(|(i, &y)| [(i as f32 * samples_to_ms) as f64, (y * osc.vertical_scale) as f64])
        .collect();

    let left_line = Line::new(left_points)
        .color(egui::Color32::from_rgb(100, 200, 100))
        .name("Left");
    let right_line = Line::new(right_points)
        .color(egui::Color32::from_rgb(100, 100, 200))
        .name("Right");

    let time_span_ms = osc.display_samples as f64 * samples_to_ms as f64;

    // Plot with zoom/pan support (double-click to reset view)
    Plot::new("oscilloscope")
        .height(200.0)
        .include_y(-1.0 * osc.vertical_scale as f64)
        .include_y(1.0 * osc.vertical_scale as f64)
        .include_x(0.0)
        .include_x(time_span_ms)
        .x_axis_label("Time (ms)")
        .y_axis_label("Amplitude")
        .allow_zoom([true, false])  // Allow horizontal zoom only
        .allow_drag([true, false])  // Allow horizontal drag only
        .allow_scroll([true, false])  // Allow horizontal scroll only
        .show(ui, |plot_ui| {
            plot_ui.line(left_line);
            plot_ui.line(right_line);
        });

    // Info
    let duration_ms = osc.display_samples as f32 / osc.sample_rate * 1000.0;
    ui.label(format!(
        "{} samples ({:.1} ms) | Scroll/pinch to zoom, drag to pan, double-click to reset",
        osc.display_samples,
        duration_ms,
    ));
}

fn spectrum_panel(ui: &mut egui::Ui, state: &mut testbench::TestbenchState, is_effect: bool) {
    // Controls
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.show_spectrogram, "Spectrogram");
        if is_effect {
            ui.checkbox(&mut state.show_comparison, "Compare with input");
            if state.show_comparison && !state.show_spectrogram {
                ui.checkbox(&mut state.normalize_to_max, "Normalize max");
            }
        }
        ui.add(egui::Slider::new(&mut state.spectrum.min_db, -120.0..=-20.0).text("Min dB"));
        ui.add(egui::Slider::new(&mut state.spectrum.max_db, -20.0..=20.0).text("Max dB"));
        if state.show_spectrogram {
            // Max frequency slider for spectrogram y-axis zoom
            let mut max_khz = state.spectrum.max_display_freq / 1000.0;
            if ui.add(egui::Slider::new(&mut max_khz, 0.5..=20.0).text("Max kHz").logarithmic(true)).changed() {
                state.spectrum.max_display_freq = max_khz * 1000.0;
            }
        } else {
            ui.add(egui::Slider::new(&mut state.spectrum.smoothing, 0.0..=0.99).text("Smooth"));
        }
    });

    // Sync spectrogram state to spectrum
    state.spectrum.show_waterfall = state.show_spectrogram;

    if state.show_spectrogram {
        // Spectrogram display with optional comparison
        if state.show_comparison && is_effect {
            spectrogram_comparison_display(ui, &mut state.spectrum, &state.pre_spectrum);
            // Legend for difference-based comparison mode
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(50, 55, 60), "■ Unchanged");
                ui.colored_label(egui::Color32::from_rgb(255, 40, 200), "■ Created");
                ui.colored_label(egui::Color32::from_rgb(255, 140, 30), "■ Strengthened");
                ui.colored_label(egui::Color32::from_rgb(40, 120, 180), "■ Weakened");
                ui.colored_label(egui::Color32::from_rgb(20, 20, 80), "■ Removed");
            });
        } else {
            spectrogram_display(ui, &mut state.spectrum);
        }
    } else {
        // Line spectrum display with optional comparison

        // Calculate F0 normalization offsets if enabled
        let (pre_f0_offset, post_f0_offset) = if state.normalize_to_max && state.show_comparison && is_effect {
            // Find peak (F0) in both spectra within audible range (20Hz - 5kHz for fundamental)
            let pre_peak = find_spectrum_peak(&state.pre_spectrum.smoothed_db, &state.pre_spectrum, 20.0, 5000.0);
            let post_peak = find_spectrum_peak(&state.spectrum.smoothed_db, &state.spectrum, 20.0, 5000.0);
            (pre_peak, post_peak)
        } else {
            (0.0, 0.0)
        };

        let post_points: PlotPoints = state.spectrum
            .smoothed_db
            .iter()
            .enumerate()
            .filter_map(|(i, &db)| {
                let freq = state.spectrum.bin_to_freq(i);
                if freq >= 20.0 && freq <= state.spectrum.sample_rate / 2.0 {
                    Some([freq.log10() as f64, (db - post_f0_offset) as f64])
                } else {
                    None
                }
            })
            .collect();

        let post_line = Line::new(post_points)
            .color(egui::Color32::from_rgb(255, 150, 50))  // Orange = output
            .name("Output");

        // Adjust Y axis for normalized view
        let (y_min, y_max) = if state.normalize_to_max && state.show_comparison && is_effect {
            (-60.0, 10.0)  // Relative dB range around max
        } else {
            (state.spectrum.min_db as f64, state.spectrum.max_db as f64)
        };

        let y_label = if state.normalize_to_max && state.show_comparison && is_effect {
            "Relative to max (dB)"
        } else {
            "Magnitude (dB)"
        };

        let plot = Plot::new("spectrum")
            .height(200.0)
            .include_y(y_min)
            .include_y(y_max)
            .include_x(20.0_f64.log10())  // 20 Hz
            .include_x(20000.0_f64.log10())  // 20 kHz
            .x_axis_label("Frequency (Hz)")
            .y_axis_label(y_label)
            .x_axis_formatter(|mark, _range| {
                // Convert log10 value back to frequency
                let freq = 10.0_f64.powf(mark.value);
                if freq >= 1000.0 {
                    format!("{:.0}k", freq / 1000.0)
                } else {
                    format!("{:.0}", freq)
                }
            })
            .label_formatter(|_name, value| {
                // Convert log10 x value back to frequency
                let freq = 10.0_f64.powf(value.x);
                let freq_str = if freq >= 1000.0 {
                    format!("{:.1}kHz", freq / 1000.0)
                } else {
                    format!("{:.0}Hz", freq)
                };
                format!("{}\n{:.1} dB", freq_str, value.y)
            });

        if state.show_comparison && is_effect {
            // Pre-DSP spectrum (input)
            let pre_points: PlotPoints = state.pre_spectrum
                .smoothed_db
                .iter()
                .enumerate()
                .filter_map(|(i, &db)| {
                    let freq = state.pre_spectrum.bin_to_freq(i);
                    if freq >= 20.0 && freq <= state.pre_spectrum.sample_rate / 2.0 {
                        Some([freq.log10() as f64, (db - pre_f0_offset) as f64])
                    } else {
                        None
                    }
                })
                .collect();

            let pre_line = Line::new(pre_points)
                .color(egui::Color32::from_rgb(100, 150, 255))  // Blue = input
                .name("Input");

            plot.show(ui, |plot_ui| {
                plot_ui.line(pre_line);
                plot_ui.line(post_line);
            });
        } else {
            plot.show(ui, |plot_ui| {
                plot_ui.line(post_line);
            });
        }
    }

    // Info and legend
    ui.horizontal(|ui| {
        ui.label(format!(
            "FFT: {} ({:.1} Hz res)",
            state.spectrum.fft_size,
            state.spectrum.sample_rate / state.spectrum.fft_size as f32,
        ));
        if state.show_comparison && is_effect {
            if state.spectrum.show_waterfall {
                // Spectrogram overlay legend
                ui.colored_label(egui::Color32::from_rgb(255, 100, 50), "■ Output");
                ui.colored_label(egui::Color32::from_rgb(50, 100, 255), "■ Input");
                ui.colored_label(egui::Color32::from_rgb(200, 100, 200), "■ Both");
            } else {
                // Line chart legend
                ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "■ Input");
                ui.colored_label(egui::Color32::from_rgb(255, 150, 50), "■ Output");
                if state.normalize_to_max {
                    ui.label("(normalized to max)");
                }
            }
        }
    });
}

fn spectrogram_display(ui: &mut egui::Ui, spectrum: &mut testbench::SpectrumState) {
    let height = 200.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );

    // Handle scroll/pinch zoom for frequency axis
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            // Zoom: scroll up = zoom in (lower max freq), scroll down = zoom out
            let zoom_factor = 1.0 - scroll * 0.002;
            spectrum.max_display_freq = (spectrum.max_display_freq * zoom_factor).clamp(500.0, 22000.0);
        }
    }

    let rect = response.rect;

    if spectrum.waterfall.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Collecting data...",
            egui::FontId::default(),
            egui::Color32::GRAY,
        );
        return;
    }

    // Draw spectrogram: time flows left to right (oldest on left), frequency bottom to top
    let num_frames = spectrum.waterfall.len();
    let total_bins = spectrum.waterfall[0].len();

    // Calculate which bin corresponds to max_display_freq
    let nyquist = spectrum.sample_rate / 2.0;
    let max_bin = ((spectrum.max_display_freq / nyquist) * total_bins as f32) as usize;
    let max_bin = max_bin.min(total_bins).max(1);

    // Limit display bins for performance, but only within the frequency range we care about
    let display_bins = max_bin.min(512);
    let bin_step = if max_bin > display_bins { max_bin / display_bins } else { 1 };
    let num_display_bins = max_bin / bin_step;

    let col_width = rect.width() / num_frames as f32;
    let row_height = rect.height() / num_display_bins as f32;

    let db_range = spectrum.max_db - spectrum.min_db;

    for (frame_idx, frame) in spectrum.waterfall.iter().enumerate() {
        let x = rect.left() + frame_idx as f32 * col_width;

        for bin_idx in 0..num_display_bins {
            let src_bin = bin_idx * bin_step;
            if src_bin >= frame.len() {
                continue;
            }

            let db = frame[src_bin];
            // Map dB to 0-1 range
            let normalized = ((db - spectrum.min_db) / db_range).clamp(0.0, 1.0);

            // Color: blue (low) -> cyan -> green -> yellow -> red (high)
            let color = db_to_color(normalized);

            // Y: low frequencies at bottom
            let y = rect.bottom() - (bin_idx as f32 + 1.0) * row_height;

            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(col_width + 1.0, row_height + 1.0),
                ),
                0.0,
                color,
            );
        }
    }

    // Show tooltip with frequency and dB when hovering
    if response.hovered() {
        if let Some(hover_pos) = ui.ctx().pointer_hover_pos() {
            if rect.contains(hover_pos) {
                // Calculate which bin the mouse is over
                let rel_y = rect.bottom() - hover_pos.y;
                let bin_idx = (rel_y / row_height) as usize;
                let src_bin = bin_idx * bin_step;

                // Calculate frequency for this bin
                let freq = src_bin as f32 * spectrum.sample_rate / spectrum.fft_size as f32;

                // Get dB value from the most recent frame
                let db = spectrum.waterfall.last()
                    .and_then(|frame| frame.get(src_bin))
                    .copied()
                    .unwrap_or(spectrum.min_db);

                response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.label(format!("{:.0} Hz", freq));
                    ui.label(format!("{:.1} dB", db));
                });
            }
        }
    }
}

/// Spectrogram comparison with difference-based coloring.
fn spectrogram_comparison_display(
    ui: &mut egui::Ui,
    post: &mut testbench::SpectrumState,
    pre: &testbench::SpectrumState,
) {
    // Tuning constants
    let unchanged_brightness: f32 = 1.0;
    let diff_threshold: f32 = 0.01;
    let height = 200.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::click_and_drag(),
    );

    // Handle scroll/pinch zoom for frequency axis
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            let zoom_factor = 1.0 - scroll * 0.002;
            post.max_display_freq = (post.max_display_freq * zoom_factor).clamp(500.0, 22000.0);
        }
    }

    let rect = response.rect;

    if post.waterfall.is_empty() || pre.waterfall.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Collecting data...",
            egui::FontId::default(),
            egui::Color32::GRAY,
        );
        return;
    }

    let num_frames = post.waterfall.len().min(pre.waterfall.len());
    let total_bins = post.waterfall[0].len();

    // Calculate which bin corresponds to max_display_freq
    let nyquist = post.sample_rate / 2.0;
    let max_bin = ((post.max_display_freq / nyquist) * total_bins as f32) as usize;
    let max_bin = max_bin.min(total_bins).max(1);

    // Limit display bins for performance
    let display_bins = max_bin.min(512);
    let bin_step = if max_bin > display_bins { max_bin / display_bins } else { 1 };
    let num_display_bins = max_bin / bin_step;

    let col_width = rect.width() / num_frames as f32;
    let row_height = rect.height() / num_display_bins as f32;

    let db_range = post.max_db - post.min_db;

    // Difference-based coloring:
    // - Green/gray: output ≈ input (no change / bypass)
    // - Red/orange: output > input (plugin adds energy)
    // - Blue: output < input (plugin removes energy)
    for frame_idx in 0..num_frames {
        let post_frame = &post.waterfall[post.waterfall.len() - num_frames + frame_idx];
        let pre_frame = &pre.waterfall[pre.waterfall.len() - num_frames + frame_idx];
        let x = rect.left() + frame_idx as f32 * col_width;

        for bin_idx in 0..num_display_bins {
            let src_bin = bin_idx * bin_step;
            if src_bin >= post_frame.len() || src_bin >= pre_frame.len() {
                continue;
            }

            let post_db = post_frame[src_bin];
            let pre_db = pre_frame[src_bin];
            let post_norm = ((post_db - post.min_db) / db_range).clamp(0.0, 1.0);
            let pre_norm = ((pre_db - pre.min_db) / db_range).clamp(0.0, 1.0);

            // Signal level (for brightness) - use max of both
            let signal_level = post_norm.max(pre_norm);

            // Skip if no significant signal
            if signal_level < 0.02 {
                continue;
            }

            // Difference: positive = output > input, negative = output < input
            let diff = post_norm - pre_norm;

            // Threshold for "quiet" (no significant signal in input)
            let quiet_threshold = diff_threshold * 3.0;

            let color = if diff.abs() < diff_threshold {
                // Unchanged - dark/dim base
                let brightness = signal_level * unchanged_brightness * 0.3;
                egui::Color32::from_rgb(
                    (60.0 * brightness) as u8,
                    (70.0 * brightness) as u8,
                    (80.0 * brightness) as u8,
                )
            } else if diff > 0.0 {
                // Output > input
                if pre_norm < quiet_threshold {
                    // CREATED - new frequency, bright magenta (pops!)
                    egui::Color32::from_rgb(
                        (255.0 * post_norm) as u8,
                        (40.0 * post_norm) as u8,
                        (200.0 * post_norm) as u8,
                    )
                } else {
                    // STRENGTHENED - existing frequency amplified, orange
                    egui::Color32::from_rgb(
                        (255.0 * signal_level) as u8,
                        (140.0 * signal_level) as u8,
                        (30.0 * signal_level) as u8,
                    )
                }
            } else {
                // Output < input
                if post_norm < quiet_threshold {
                    // REMOVED - frequency eliminated, very dark blue
                    let dim = pre_norm * 0.4;
                    egui::Color32::from_rgb(
                        (20.0 * dim) as u8,
                        (20.0 * dim) as u8,
                        (80.0 * dim) as u8,
                    )
                } else {
                    // WEAKENED - frequency attenuated, dim cyan
                    let dim = signal_level * 0.7;
                    egui::Color32::from_rgb(
                        (40.0 * dim) as u8,
                        (120.0 * dim) as u8,
                        (180.0 * dim) as u8,
                    )
                }
            };

            let y = rect.bottom() - (bin_idx as f32 + 1.0) * row_height;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(col_width + 1.0, row_height + 1.0),
                ),
                0.0,
                color,
            );
        }
    }
}

/// Find the peak (F0) dB value in a spectrum within a frequency range.
/// Returns the dB value of the strongest bin in the range.
fn find_spectrum_peak(
    smoothed_db: &[f32],
    spectrum: &testbench::SpectrumState,
    min_freq: f32,
    max_freq: f32,
) -> f32 {
    let mut peak_db = f32::NEG_INFINITY;

    for (i, &db) in smoothed_db.iter().enumerate() {
        let freq = spectrum.bin_to_freq(i);
        if freq >= min_freq && freq <= max_freq && db > peak_db {
            peak_db = db;
        }
    }

    if peak_db.is_finite() {
        peak_db
    } else {
        0.0  // Fallback if no valid peak found
    }
}

/// Find the peak (F0) dB value in ERB bands within a frequency range.
/// Returns the dB value of the strongest band in the range.
fn find_erb_peak(
    smoothed_db: &[f32],
    erb: &testbench::ErbState,
    min_freq: f32,
    max_freq: f32,
) -> f32 {
    let mut peak_db = f32::NEG_INFINITY;

    for (i, &db) in smoothed_db.iter().enumerate() {
        let freq = erb.center_hz(i);
        if freq >= min_freq && freq <= max_freq && db > peak_db {
            peak_db = db;
        }
    }

    if peak_db.is_finite() {
        peak_db
    } else {
        0.0  // Fallback if no valid peak found
    }
}

fn erb_panel(ui: &mut egui::Ui, state: &mut testbench::TestbenchState, is_effect: bool) {
    let erb = &mut state.erb;
    let pre_erb = &state.pre_erb;

    // Controls
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.show_spectrogram, "Spectrogram");
        if is_effect {
            ui.checkbox(&mut state.show_comparison, "Compare with input");
            if state.show_comparison && !state.show_spectrogram {
                ui.checkbox(&mut state.normalize_to_max, "Normalize max");
            }
        }
        ui.add(egui::Slider::new(&mut erb.min_db, -120.0..=-20.0).text("Min dB"));
        ui.add(egui::Slider::new(&mut erb.max_db, -20.0..=20.0).text("Max dB"));
        if !state.show_spectrogram {
            ui.add(egui::Slider::new(&mut erb.smoothing, 0.0..=0.99).text("Smooth"));
        }
    });

    // Sync spectrogram state to erb
    erb.show_waterfall = state.show_spectrogram;

    if state.show_spectrogram {
        // ERB spectrogram display with optional comparison
        if state.show_comparison && is_effect {
            erb_spectrogram_comparison_display(ui, erb, pre_erb);
            // Legend for difference-based comparison mode
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(50, 55, 60), "■ Unchanged");
                ui.colored_label(egui::Color32::from_rgb(255, 40, 200), "■ Created");
                ui.colored_label(egui::Color32::from_rgb(255, 140, 30), "■ Strengthened");
                ui.colored_label(egui::Color32::from_rgb(40, 120, 180), "■ Weakened");
                ui.colored_label(egui::Color32::from_rgb(20, 20, 80), "■ Removed");
            });
        } else {
            erb_spectrogram_display(ui, erb);
        }
    } else {
        // Bar chart display with optional comparison
        erb_bars_display_with_comparison(ui, erb, pre_erb, state.show_comparison && is_effect, state.normalize_to_max);
    }

    // Info and legend
    ui.horizontal(|ui| {
        ui.label(format!(
            "{} ERB bands (Gammatone), 20-20k Hz",
            erb.num_bands
        ));
        if state.show_comparison && is_effect && !erb.show_waterfall {
            // Bar chart legend (spectrogram legend is shown inline above)
            ui.colored_label(egui::Color32::from_rgb(255, 150, 50), "■ Output");
            ui.colored_label(egui::Color32::from_rgb(100, 150, 255), "— Input");
            ui.colored_label(egui::Color32::from_rgb(255, 200, 50), "■ Added");
            if state.normalize_to_max {
                ui.label("(normalized to max)");
            }
        }
    });
}

fn erb_bars_display_with_comparison(
    ui: &mut egui::Ui,
    post_erb: &testbench::ErbState,
    pre_erb: &testbench::ErbState,
    show_comparison: bool,
    normalize_to_max: bool,
) {
    let height = 200.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let rect = response.rect;
    let num_bands = post_erb.num_bands;
    let bar_width = rect.width() / num_bands as f32;

    // Calculate F0 normalization offsets if enabled
    let (pre_f0_offset, post_f0_offset) = if normalize_to_max && show_comparison {
        let pre_peak = find_erb_peak(&pre_erb.smoothed_db, pre_erb, 20.0, 5000.0);
        let post_peak = find_erb_peak(&post_erb.smoothed_db, post_erb, 20.0, 5000.0);
        (pre_peak, post_peak)
    } else {
        (0.0, 0.0)
    };

    // Use fixed range for normalized view, otherwise use erb settings
    let (min_db, max_db) = if normalize_to_max && show_comparison {
        (-60.0, 10.0)  // Relative dB range around max
    } else {
        (post_erb.min_db, post_erb.max_db)
    };
    let db_range = max_db - min_db;

    let input_color = egui::Color32::from_rgb(100, 150, 255);  // Blue
    let output_color = egui::Color32::from_rgb(255, 150, 50);  // Orange
    let added_color = egui::Color32::from_rgba_unmultiplied(255, 200, 50, 180);  // Yellow-ish

    for i in 0..num_bands {
        let raw_post_db = post_erb.smoothed_db.get(i).copied().unwrap_or(post_erb.min_db);
        let raw_pre_db = pre_erb.smoothed_db.get(i).copied().unwrap_or(pre_erb.min_db);

        // Apply F0 normalization
        let post_db = raw_post_db - post_f0_offset;
        let pre_db = raw_pre_db - pre_f0_offset;

        let post_norm = ((post_db - min_db) / db_range).clamp(0.0, 1.0);
        let post_height = post_norm * rect.height();

        let x = rect.left() + i as f32 * bar_width;

        if show_comparison {
            let pre_norm = ((pre_db - min_db) / db_range).clamp(0.0, 1.0);
            let pre_height = pre_norm * rect.height();

            // Draw output bar first (solid)
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, rect.bottom() - post_height),
                    egui::vec2(bar_width - 1.0, post_height),
                ),
                0.0,
                output_color,
            );

            // Draw input level as reference line on top
            let line_y = rect.bottom() - pre_height;
            painter.line_segment(
                [egui::pos2(x, line_y), egui::pos2(x + bar_width - 1.0, line_y)],
                egui::Stroke::new(2.0, input_color),
            );

            // Highlight where output exceeds input (added energy)
            let diff = post_db - pre_db;
            if diff > 1.0 {
                let diff_height = (post_height - pre_height).max(0.0);
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(x, rect.bottom() - post_height),
                        egui::vec2(bar_width - 1.0, diff_height),
                    ),
                    0.0,
                    added_color,
                );
            }
        } else {
            // Just draw output
            let color = db_to_color(post_norm);
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, rect.bottom() - post_height),
                    egui::vec2(bar_width - 1.0, post_height),
                ),
                0.0,
                color,
            );
        }
    }

    // Draw frequency labels at key points
    let label_freqs = [100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0, 20000.0];
    for &label_freq in &label_freqs {
        // Find the band closest to this frequency
        for i in 0..num_bands {
            let freq = post_erb.center_hz(i);
            if freq >= label_freq * 0.9 && freq <= label_freq * 1.1 {
                let x = rect.left() + i as f32 * bar_width + bar_width / 2.0;
                let label = if label_freq >= 1000.0 {
                    format!("{}k", (label_freq / 1000.0) as i32)
                } else {
                    format!("{}", label_freq as i32)
                };
                painter.text(
                    egui::pos2(x, rect.bottom() + 2.0),
                    egui::Align2::CENTER_TOP,
                    label,
                    egui::FontId::proportional(9.0),
                    egui::Color32::GRAY,
                );
                break;
            }
        }
    }

    // Tooltip
    if response.hovered() {
        if let Some(hover_pos) = ui.ctx().pointer_hover_pos() {
            if rect.contains(hover_pos) {
                let rel_x = hover_pos.x - rect.left();
                let band_idx = ((rel_x / bar_width) as usize).min(num_bands - 1);
                let freq = post_erb.center_hz(band_idx);
                let post_db = post_erb.smoothed_db.get(band_idx).copied().unwrap_or(post_erb.min_db);

                response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.label(format!("Band {} ({:.0} Hz)", band_idx, freq));
                    if show_comparison {
                        let pre_db = pre_erb.smoothed_db.get(band_idx).copied().unwrap_or(pre_erb.min_db);
                        let diff = post_db - pre_db;
                        ui.label(format!("In:  {:.1} dB", pre_db));
                        ui.label(format!("Out: {:.1} dB", post_db));
                        let sign = if diff >= 0.0 { "+" } else { "" };
                        ui.label(format!("Diff: {}{:.1} dB", sign, diff));
                    } else {
                        ui.label(format!("{:.1} dB", post_db));
                    }
                });
            }
        }
    }
}

fn erb_spectrogram_display(ui: &mut egui::Ui, erb: &testbench::ErbState) {
    let height = 200.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let rect = response.rect;

    if erb.waterfall.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Collecting data...",
            egui::FontId::default(),
            egui::Color32::GRAY,
        );
        return;
    }

    // Draw spectrogram: time flows left to right, frequency bottom to top
    let num_frames = erb.waterfall.len();
    let num_bands = erb.num_bands;

    let col_width = rect.width() / num_frames as f32;
    let row_height = rect.height() / num_bands as f32;

    let db_range = erb.max_db - erb.min_db;

    for (frame_idx, frame) in erb.waterfall.iter().enumerate() {
        let x = rect.left() + frame_idx as f32 * col_width;

        for (band_idx, &db) in frame.iter().enumerate() {
            let normalized = ((db - erb.min_db) / db_range).clamp(0.0, 1.0);
            let color = db_to_color(normalized);

            // Y: low frequencies at bottom
            let y = rect.bottom() - (band_idx as f32 + 1.0) * row_height;

            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(col_width + 1.0, row_height + 1.0),
                ),
                0.0,
                color,
            );
        }
    }

    // Show tooltip with frequency and dB when hovering
    if response.hovered() {
        if let Some(hover_pos) = ui.ctx().pointer_hover_pos() {
            if rect.contains(hover_pos) {
                let rel_y = rect.bottom() - hover_pos.y;
                let band_idx = ((rel_y / row_height) as usize).min(num_bands - 1);
                let freq = erb.center_hz(band_idx);

                let db = erb.waterfall.last()
                    .and_then(|frame| frame.get(band_idx))
                    .copied()
                    .unwrap_or(erb.min_db);

                response.clone().on_hover_ui_at_pointer(|ui| {
                    ui.label(format!("Band {}", band_idx));
                    ui.label(format!("{:.0} Hz", freq));
                    ui.label(format!("{:.1} dB", db));
                });
            }
        }
    }
}

/// ERB spectrogram comparison with difference-based coloring.
fn erb_spectrogram_comparison_display(
    ui: &mut egui::Ui,
    post_erb: &testbench::ErbState,
    pre_erb: &testbench::ErbState,
) {
    // Tuning constants
    let unchanged_brightness: f32 = 1.0;
    let diff_threshold: f32 = 0.01;
    let height = 200.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );

    let rect = response.rect;

    if post_erb.waterfall.is_empty() || pre_erb.waterfall.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Collecting data...",
            egui::FontId::default(),
            egui::Color32::GRAY,
        );
        return;
    }

    let num_frames = post_erb.waterfall.len().min(pre_erb.waterfall.len());
    let num_bands = post_erb.num_bands;

    let col_width = rect.width() / num_frames as f32;
    let row_height = rect.height() / num_bands as f32;

    let db_range = post_erb.max_db - post_erb.min_db;

    // Difference-based coloring
    for frame_idx in 0..num_frames {
        let post_frame = &post_erb.waterfall[post_erb.waterfall.len() - num_frames + frame_idx];
        let pre_frame = &pre_erb.waterfall[pre_erb.waterfall.len() - num_frames + frame_idx];
        let x = rect.left() + frame_idx as f32 * col_width;

        for band_idx in 0..num_bands {
            if band_idx >= post_frame.len() || band_idx >= pre_frame.len() {
                continue;
            }

            let post_db = post_frame[band_idx];
            let pre_db = pre_frame[band_idx];
            let post_norm = ((post_db - post_erb.min_db) / db_range).clamp(0.0, 1.0);
            let pre_norm = ((pre_db - pre_erb.min_db) / db_range).clamp(0.0, 1.0);

            // Signal level (for brightness) - use max of both
            let signal_level = post_norm.max(pre_norm);

            // Skip if no significant signal
            if signal_level < 0.02 {
                continue;
            }

            // Difference: positive = output > input, negative = output < input
            let diff = post_norm - pre_norm;

            // Threshold for "quiet" (no significant signal in input)
            let quiet_threshold = diff_threshold * 3.0;

            let color = if diff.abs() < diff_threshold {
                // Unchanged - dark/dim base
                let brightness = signal_level * unchanged_brightness * 0.3;
                egui::Color32::from_rgb(
                    (60.0 * brightness) as u8,
                    (70.0 * brightness) as u8,
                    (80.0 * brightness) as u8,
                )
            } else if diff > 0.0 {
                // Output > input
                if pre_norm < quiet_threshold {
                    // CREATED - new frequency, bright magenta (pops!)
                    egui::Color32::from_rgb(
                        (255.0 * post_norm) as u8,
                        (40.0 * post_norm) as u8,
                        (200.0 * post_norm) as u8,
                    )
                } else {
                    // STRENGTHENED - existing frequency amplified, orange
                    egui::Color32::from_rgb(
                        (255.0 * signal_level) as u8,
                        (140.0 * signal_level) as u8,
                        (30.0 * signal_level) as u8,
                    )
                }
            } else {
                // Output < input
                if post_norm < quiet_threshold {
                    // REMOVED - frequency eliminated, very dark blue
                    let dim = pre_norm * 0.4;
                    egui::Color32::from_rgb(
                        (20.0 * dim) as u8,
                        (20.0 * dim) as u8,
                        (80.0 * dim) as u8,
                    )
                } else {
                    // WEAKENED - frequency attenuated, dim cyan
                    let dim = signal_level * 0.7;
                    egui::Color32::from_rgb(
                        (40.0 * dim) as u8,
                        (120.0 * dim) as u8,
                        (180.0 * dim) as u8,
                    )
                }
            };

            let y = rect.bottom() - (band_idx as f32 + 1.0) * row_height;
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(col_width + 1.0, row_height + 1.0),
                ),
                0.0,
                color,
            );
        }
    }
}

/// Convert normalized dB value (0-1) to color
fn db_to_color(normalized: f32) -> egui::Color32 {
    // Heat map: black -> blue -> cyan -> green -> yellow -> red -> white
    let v = normalized.clamp(0.0, 1.0);

    if v < 0.2 {
        // Black to blue
        let t = v / 0.2;
        egui::Color32::from_rgb(0, 0, (t * 150.0) as u8)
    } else if v < 0.4 {
        // Blue to cyan
        let t = (v - 0.2) / 0.2;
        egui::Color32::from_rgb(0, (t * 200.0) as u8, 150 + (t * 55.0) as u8)
    } else if v < 0.6 {
        // Cyan to green
        let t = (v - 0.4) / 0.2;
        egui::Color32::from_rgb(0, 200 + (t * 55.0) as u8, (205.0 * (1.0 - t)) as u8)
    } else if v < 0.8 {
        // Green to yellow
        let t = (v - 0.6) / 0.2;
        egui::Color32::from_rgb((t * 255.0) as u8, 255, 0)
    } else {
        // Yellow to red
        let t = (v - 0.8) / 0.2;
        egui::Color32::from_rgb(255, (255.0 * (1.0 - t)) as u8, 0)
    }
}

fn benchmark_panel(ui: &mut egui::Ui, metrics: &testbench::SharedMetrics) {
    let last_ns = metrics.last_process_ns.load(Ordering::Relaxed);
    let max_ns = metrics.max_process_ns.load(Ordering::Relaxed);
    let avg_ns = metrics.average_process_ns();
    let buffer_size = metrics.buffer_size.load(Ordering::Relaxed);
    let sample_rate = metrics.sample_rate.load(Ordering::Relaxed);
    let cpu_percent = metrics.cpu_usage_percent();

    ui.horizontal(|ui| {
        ui.label(format!("Buffer: {} samples", buffer_size));
        ui.label(format!("Sample Rate: {} Hz", sample_rate));
    });

    ui.separator();

    egui::Grid::new("benchmark_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .show(ui, |ui| {
            ui.label("Current:");
            ui.label(format!("{:.1} µs", last_ns as f64 / 1000.0));
            ui.end_row();

            ui.label("Average:");
            ui.label(format!("{:.1} µs", avg_ns as f64 / 1000.0));
            ui.end_row();

            ui.label("Maximum:");
            ui.label(format!("{:.1} µs", max_ns as f64 / 1000.0));
            ui.end_row();

            ui.label("CPU Usage:");
            let cpu_color = if cpu_percent < 50.0 {
                egui::Color32::GREEN
            } else if cpu_percent < 80.0 {
                egui::Color32::YELLOW
            } else {
                egui::Color32::RED
            };
            ui.colored_label(cpu_color, format!("{:.1}%", cpu_percent));
            ui.end_row();
        });

    // Available time indicator
    if sample_rate > 0 && buffer_size > 0 {
        let available_us = (buffer_size as f64 / sample_rate as f64) * 1_000_000.0;
        ui.separator();
        ui.label(format!("Available time per buffer: {:.1} µs", available_us));

        // Progress bar for CPU usage
        ui.add(
            egui::ProgressBar::new(cpu_percent / 100.0)
                .text(format!("CPU: {:.1}%", cpu_percent))
                .fill(if cpu_percent < 50.0 {
                    egui::Color32::DARK_GREEN
                } else if cpu_percent < 80.0 {
                    egui::Color32::from_rgb(180, 180, 0)
                } else {
                    egui::Color32::DARK_RED
                }),
        );
    }

    if ui.button("Reset Metrics").clicked() {
        metrics.reset();
    }
}
