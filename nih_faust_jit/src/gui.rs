mod panels;

use nih_plug::prelude::*;
use nih_plug_egui::egui;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use crate::{audio_file, config, presets::PresetManager, save_widget_values, DspState};

/// Data shared between the plugin and the GUI thread
pub(crate) struct GuiArcs {
    pub(crate) nih_egui_state: Arc<nih_plug_egui::EguiState>,
    pub(crate) selected_paths: Arc<RwLock<crate::SelectedPaths>>,
    pub(crate) dsp_state: Arc<RwLock<DspState>>,
    pub(crate) dsp_nvoices: Arc<RwLock<i32>>,
    pub(crate) faust_param_values: Arc<RwLock<HashMap<String, f32>>>,
    pub(crate) preset_loading: Arc<AtomicBool>,
    pub(crate) audio_file_player: Arc<RwLock<audio_file::AudioFilePlayer>>,
    pub(crate) playback_state: Arc<audio_file::PlaybackState>,
    /// Current oversampling factor (what DSP is actually loaded with)
    pub(crate) oversampling: Arc<std::sync::atomic::AtomicU8>,
    /// Pending oversampling factor (what user selected, updated after reload)
    pub(crate) pending_oversampling: Arc<std::sync::atomic::AtomicU8>,
}

/// Data owned only by the GUI thread
pub(crate) struct GuiState {
    /// Pending file dialog result for script
    pending_script_path: Arc<RwLock<Option<PathBuf>>>,
    /// Pending file dialog result for lib path
    pending_lib_path: Arc<RwLock<Option<PathBuf>>>,
    /// Pending audio file dialog result
    pending_audio_file: Arc<RwLock<Option<PathBuf>>>,
    /// Preset manager
    preset_manager: PresetManager,
    /// Current preset name being entered for saving
    save_preset_name: String,
    /// Currently selected preset name
    selected_preset: Option<String>,
    /// Cached list of presets for current DSP
    cached_presets: Vec<String>,
    /// DSP name for which presets are cached
    cached_dsp_name: Option<String>,
    /// Cached audio input devices
    audio_inputs: Vec<String>,
    /// Cached audio output devices
    audio_outputs: Vec<String>,
    /// Cached MIDI input devices
    midi_inputs: Vec<String>,
    /// Current device configuration
    device_config: config::StandaloneConfig,
    /// Whether device lists have been enumerated
    devices_enumerated: bool,
    /// Whether config has been modified (needs restart to apply)
    config_modified: bool,
    /// Whether delete preset confirmation is showing
    confirm_delete_preset: bool,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            pending_script_path: Arc::new(RwLock::new(None)),
            pending_lib_path: Arc::new(RwLock::new(None)),
            pending_audio_file: Arc::new(RwLock::new(None)),
            preset_manager: PresetManager::new(),
            save_preset_name: String::new(),
            selected_preset: None,
            cached_presets: Vec::new(),
            cached_dsp_name: None,
            audio_inputs: Vec::new(),
            audio_outputs: Vec::new(),
            midi_inputs: Vec::new(),
            device_config: config::StandaloneConfig::load(),
            devices_enumerated: false,
            config_modified: false,
            confirm_delete_preset: false,
        }
    }
}

pub(crate) fn create_gui(
    arcs: GuiArcs,
    async_executor: AsyncExecutor<crate::NihFaustJit>,
) -> Option<Box<dyn Editor>> {
    nih_plug_egui::create_egui_editor(
        Arc::clone(&arcs.nih_egui_state),
        GuiState::default(),
        |_, _| {},
        move |egui_ctx, _param_setter, gui_state| {
            if arcs.nih_egui_state.is_open() {
                // Handle space bar for play/pause toggle
                if egui_ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                    let playing = arcs.playback_state.playing.load(Ordering::Relaxed);
                    arcs.playback_state.playing.store(!playing, Ordering::Relaxed);
                }

                // Handle CMD-R (macOS) or CTRL-R (other platforms) for DSP reload
                let reload_shortcut = egui_ctx.input(|i| {
                    i.key_pressed(egui::Key::R) && (i.modifiers.command || i.modifiers.ctrl)
                });
                if reload_shortcut && arcs.selected_paths.read().unwrap().dsp_script.is_some() {
                    async_executor.execute_background(crate::Tasks::ReloadDsp);
                }

                // Check for pending file dialog results
                check_pending_dialogs(&arcs, &async_executor, gui_state);

                // Top panel (DSP settings, loading):
                egui::TopBottomPanel::top("DSP loading")
                    .frame(egui::Frame::default().inner_margin(8.0))
                    .show(egui_ctx, |ui| {
                        panels::top_panel_contents(ui, &arcs, &async_executor, gui_state);
                    });

                // Central panel (plugin's GUI):
                egui::CentralPanel::default().show(egui_ctx, |ui| {
                    egui::ScrollArea::both()
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(
                            egui::scroll_area::ScrollBarVisibility::AlwaysVisible,
                        )
                        .show(ui, |ui| match &*arcs.dsp_state.read().unwrap() {
                            DspState::NoDspScript => {
                                ui.label("-- No DSP --");
                            }
                            DspState::Failed(faust_err_msg) => {
                                ui.colored_label(egui::Color32::LIGHT_RED, faust_err_msg);
                            }
                            DspState::Loaded(dsp) => {
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                                let margin = egui::Margin {
                                    left: 0,
                                    right: 5,
                                    top: 0,
                                    bottom: 8,
                                };
                                egui::Frame::default().outer_margin(margin).show(ui, |ui| {
                                    dsp.with_widgets_mut(|widgets| {
                                        faust_jit_egui::faust_widgets_ui(ui, widgets);
                                        let storage = arcs.faust_param_values.read().unwrap();
                                        crate::log_param_changes(widgets, &storage, "");
                                        drop(storage);
                                        if !arcs.preset_loading.load(Ordering::SeqCst) {
                                            let mut storage = arcs.faust_param_values.write().unwrap();
                                            save_widget_values(widgets, &mut storage, "");
                                        }
                                    })
                                });
                            }
                        });
                });
            }
        },
    )
}

fn check_pending_dialogs(
    arcs: &GuiArcs,
    async_executor: &AsyncExecutor<crate::NihFaustJit>,
    gui_state: &mut GuiState,
) {
    if let Some(path) = gui_state.pending_script_path.write().unwrap().take() {
        arcs.selected_paths.write().unwrap().dsp_script = Some(path);
        async_executor.execute_background(crate::Tasks::ReloadDsp);
    }

    if let Some(path) = gui_state.pending_lib_path.write().unwrap().take() {
        arcs.selected_paths.write().unwrap().dsp_lib_path = path;
    }

    if let Some(path) = gui_state.pending_audio_file.write().unwrap().take() {
        async_executor.execute_background(crate::Tasks::LoadAudioFile(path));
    }
}
