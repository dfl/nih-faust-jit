use nih_plug::prelude::*;
use nih_plug_egui::egui;
use std::sync::{atomic::Ordering, Arc};

use crate::{audio_file, config, DspState, DspType};
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
    let mut nvoices = *arcs.dsp_nvoices.read().unwrap();
    let mut selected_dsp_type = DspType::from_nvoices(nvoices);
    let last_dsp_type = selected_dsp_type;
    ui.horizontal(|ui| {
        ui.label("DSP type:");
        enum_combobox(ui, "dsp-type-combobox", &mut selected_dsp_type);
        match selected_dsp_type {
            DspType::AutoDetect => {
                nvoices = -1;
                ui.label("DSP type and number of voices will be detected from script metadata");
            }
            DspType::Effect => {
                nvoices = 0;
                ui.label("DSP will be loaded as monophonic effect");
            }
            DspType::Instrument => {
                if selected_dsp_type != last_dsp_type {
                    nvoices = 1;
                }
                ui.add(egui::Slider::new(&mut nvoices, 1..=32).text("voices"));
            }
        }
    });
    *arcs.dsp_nvoices.write().unwrap() = nvoices;

    let selected_paths = arcs.selected_paths.read().unwrap();

    ui.label(format!(
        "Faust DSP libraries path: {}",
        selected_paths.dsp_lib_path.display()
    ));
    if ui.button("Set Faust libraries path").clicked() {
        let pending = Arc::clone(&gui_state.pending_lib_path);
        let start_path = selected_paths.dsp_lib_path.clone();
        std::thread::spawn(move || {
            if let Some(path) = rfd::FileDialog::new()
                .set_directory(&start_path)
                .pick_folder()
            {
                *pending.write().unwrap() = Some(path);
            }
        });
    }

    ui.horizontal(|ui| {
        match &selected_paths.dsp_script {
            Some(path) => {
                ui.label(format!("DSP script: {}", path.display()));
                if ui.button("Edit").clicked() {
                    if let Err(e) = open::that(path) {
                        nih_plug::log::log!(
                            nih_plug::log::Level::Error,
                            "Failed to open editor: {}",
                            e
                        );
                    }
                }
            }
            None => {
                ui.colored_label(egui::Color32::YELLOW, "No DSP script selected");
            }
        };
    });

    drop(selected_paths);

    if ui
        .add(egui::Button::new("Set or reload DSP script").fill(egui::Color32::DARK_GRAY))
        .clicked()
    {
        let pending = Arc::clone(&gui_state.pending_script_path);
        let selected_paths = arcs.selected_paths.read().unwrap();
        let start_path = selected_paths
            .dsp_script
            .clone()
            .unwrap_or_else(|| selected_paths.dsp_lib_path.clone());
        drop(selected_paths);

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

    if arcs.selected_paths.read().unwrap().dsp_script.is_some() {
        if ui.button("Reload DSP").clicked() {
            async_executor.execute_background(crate::Tasks::ReloadDsp);
        }
    }

    // Preset section
    let dsp_script = arcs.selected_paths.read().unwrap().dsp_script.clone();
    if let Some(script_path) = dsp_script {
        let dsp_name = script_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown");

        if gui_state.cached_dsp_name.as_deref() != Some(dsp_name) {
            gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
            gui_state.cached_dsp_name = Some(dsp_name.to_string());
            gui_state.selected_preset = None;
        }

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Presets:");

            egui::ComboBox::from_id_salt("preset-selector")
                .selected_text(
                    gui_state
                        .selected_preset
                        .clone()
                        .unwrap_or_else(|| "Select preset...".to_string()),
                )
                .show_ui(ui, |ui| {
                    for preset_name in &gui_state.cached_presets {
                        let is_selected = gui_state.selected_preset.as_ref() == Some(preset_name);
                        if ui
                            .selectable_label(is_selected, preset_name)
                            .clicked()
                        {
                            gui_state.selected_preset = Some(preset_name.clone());
                        }
                    }
                });

            if ui
                .add_enabled(gui_state.selected_preset.is_some(), egui::Button::new("Load"))
                .clicked()
            {
                if let Some(preset_name) = &gui_state.selected_preset {
                    match gui_state.preset_manager.load_preset(dsp_name, preset_name) {
                        Ok(preset) => {
                            arcs.preset_loading.store(true, Ordering::SeqCst);
                            *arcs.faust_param_values.write().unwrap() = preset.values;
                            async_executor.execute_background(crate::Tasks::ReloadDsp);
                        }
                        Err(e) => {
                            nih_plug::log::log!(nih_plug::log::Level::Error, "Load preset error: {}", e);
                        }
                    }
                }
            }

            if ui
                .add_enabled(gui_state.selected_preset.is_some(), egui::Button::new("Delete"))
                .clicked()
            {
                if let Some(preset_name) = &gui_state.selected_preset {
                    if let Err(e) = gui_state.preset_manager.delete_preset(dsp_name, preset_name) {
                        nih_plug::log::log!(nih_plug::log::Level::Error, "Delete preset error: {}", e);
                    } else {
                        gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
                        gui_state.selected_preset = None;
                    }
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Save as:");
            ui.text_edit_singleline(&mut gui_state.save_preset_name);
            if ui
                .add_enabled(!gui_state.save_preset_name.is_empty(), egui::Button::new("Save"))
                .clicked()
            {
                let values = arcs.faust_param_values.read().unwrap().clone();
                match gui_state.preset_manager.save_preset(
                    dsp_name,
                    &gui_state.save_preset_name,
                    values,
                ) {
                    Ok(()) => {
                        gui_state.cached_presets = gui_state.preset_manager.list_presets(dsp_name);
                        gui_state.selected_preset = Some(gui_state.save_preset_name.clone());
                        gui_state.save_preset_name.clear();
                    }
                    Err(e) => {
                        nih_plug::log::log!(nih_plug::log::Level::Error, "Save preset error: {}", e);
                    }
                }
            }
        });
    }

    ui.separator();
    audio_file_panel(ui, arcs, gui_state);

    ui.separator();
    device_settings_panel(ui, gui_state);
}

pub(super) fn audio_file_panel(
    ui: &mut egui::Ui,
    arcs: &GuiArcs,
    gui_state: &mut GuiState,
) {
    ui.collapsing("Audio File Input", |ui| {
        let player = arcs.audio_file_player.read().unwrap();
        let audio_info = player.get_audio_info();
        drop(player);

        let (has_audio, total_frames, sample_rate) = match &audio_info {
            Some(audio) => {
                let filename = audio
                    .file_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown");
                ui.label(format!("File: {}", filename));
                if audio.original_sample_rate != audio.sample_rate {
                    ui.label(format!(
                        "Resampled: {} Hz -> {} Hz",
                        audio.original_sample_rate, audio.sample_rate
                    ));
                }
                (true, audio.total_frames, audio.sample_rate)
            }
            None => {
                ui.label("No audio file loaded");
                (false, 0, 48000)
            }
        };

        let dsp_loaded = matches!(&*arcs.dsp_state.read().unwrap(), DspState::Loaded(_));

        let load_btn = egui::Button::new("Load Audio File");
        let load_response = ui.add_enabled(dsp_loaded, load_btn);

        if !dsp_loaded {
            ui.colored_label(egui::Color32::YELLOW, "⚠ Load a DSP script first");
        }

        if load_response.clicked() {
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

        if has_audio {
            ui.horizontal(|ui| {
                let playing = arcs.playback_state.playing.load(Ordering::Relaxed);

                let play_label = if playing { "Pause" } else { "Play" };
                if ui.button(play_label).clicked() {
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
            });

            let pos = arcs.playback_state.position.load(Ordering::Relaxed);
            let sr = sample_rate as f64;
            let pos_secs = pos as f64 / sr;
            let total_secs = total_frames as f64 / sr;

            ui.horizontal(|ui| {
                ui.label(format!(
                    "{} / {}",
                    audio_file::format_duration(pos_secs),
                    audio_file::format_duration(total_secs)
                ));
            });

            let mut pos_f = if total_frames > 0 {
                pos as f32 / total_frames as f32
            } else {
                0.0
            };
            if ui
                .add(egui::Slider::new(&mut pos_f, 0.0..=1.0).show_value(false))
                .changed()
            {
                let new_pos = (pos_f * total_frames as f32) as u64;
                arcs.playback_state.position.store(new_pos, Ordering::Relaxed);
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
