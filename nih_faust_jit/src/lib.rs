use nih_plug::{
    log::{log, Level},
    midi::MidiResult,
    prelude::*,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};

use faust_jit::{DspWidget, Zone};

/// Log at Info level only when --debug flag is set
macro_rules! debug_log {
    ($($arg:tt)*) => {
        if std::env::var("NIH_FAUST_JIT_DEBUG").is_ok() {
            log!(Level::Info, $($arg)*);
        }
    };
}

pub mod audio_file;
pub mod config;
mod gui;
pub mod presets;

#[derive(Debug)]
enum DspState {
    NoDspScript,
    Loaded(faust_jit::SingletonDsp),
    Failed(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SelectedPaths {
    dsp_script: Option<std::path::PathBuf>,
    dsp_lib_path: std::path::PathBuf,
}

pub struct NihFaustJit {
    sample_rate: Arc<AtomicF32>,
    params: Arc<NihFaustJitParams>,
    dsp_state: Arc<RwLock<DspState>>,
    // Audio file playback (standalone mode)
    audio_file_player: Arc<RwLock<audio_file::AudioFilePlayer>>,
    playback_state: Arc<audio_file::PlaybackState>,
    debug_mode: bool,
    /// The last known values of parameters (for change logging)
    last_param_values: std::sync::Mutex<HashMap<String, f32>>,
}

#[derive(Params)]
struct NihFaustJitParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[persist = "editor-state"]
    nih_egui_state: Arc<nih_plug_egui::EguiState>,

    #[persist = "selected-paths"]
    selected_paths: Arc<RwLock<SelectedPaths>>,

    #[persist = "dsp-nvoices"]
    dsp_nvoices: Arc<RwLock<i32>>,

    /// Persisted Faust DSP parameter values (maps label path to value)
    #[persist = "faust-param-values"]
    faust_param_values: Arc<RwLock<HashMap<String, f32>>>,

    /// Flag to prevent save_widget_values from overwriting preset values during load
    preset_loading: Arc<AtomicBool>,
}

impl NihFaustJit {
    /// Clone from the plugin the Arcs that the GUI thread will need
    pub(crate) fn gui_arcs(&self) -> gui::GuiArcs {
        gui::GuiArcs {
            nih_egui_state: Arc::clone(&self.params.nih_egui_state),
            selected_paths: Arc::clone(&self.params.selected_paths),
            dsp_state: Arc::clone(&self.dsp_state),
            dsp_nvoices: Arc::clone(&self.params.dsp_nvoices),
            faust_param_values: Arc::clone(&self.params.faust_param_values),
            preset_loading: Arc::clone(&self.params.preset_loading),
            audio_file_player: Arc::clone(&self.audio_file_player),
            playback_state: Arc::clone(&self.playback_state),
        }
    }
}

// ============================================================================
// Parameter Persistence Helpers
// ============================================================================

/// Build widget path, skipping empty labels to avoid paths like "Razor//Drive"
fn build_widget_path(path: &str, label: &str) -> String {
    if label.is_empty() {
        path.to_string()
    } else if path.is_empty() {
        label.to_string()
    } else {
        format!("{}/{}", path, label)
    }
}

/// Recursively save widget values to a HashMap
/// If `for_preset` is true, parameters with [nopre:1] metadata are skipped
pub(crate) fn save_widget_values<Z: Zone>(widgets: &[DspWidget<Z>], storage: &mut HashMap<String, f32>, path: &str, for_preset: bool) {
    for widget in widgets {
        let widget_path = build_widget_path(path, widget.label());

        match widget {
            DspWidget::NumParam { zone, metadata, .. } => {
                // Skip parameters marked with [nopre:1] only when saving for presets
                if !(for_preset && metadata.nopre) && !widget_path.is_empty() {
                    storage.insert(widget_path, zone.cur_value());
                }
            }
            DspWidget::BoolParam { zone, nopre, .. } => {
                // Skip parameters marked with [nopre:1] only when saving for presets
                if !(for_preset && *nopre) && !widget_path.is_empty() {
                    storage.insert(widget_path, zone.cur_value());
                }
            }
            DspWidget::Box { inner, .. } => {
                save_widget_values(inner, storage, &widget_path, for_preset);
            }
            DspWidget::NumDisplay { .. } => {
                // Displays are read-only, don't save
            }
        }
    }
}

/// Log parameter changes (for debugging filter instability)
#[allow(dead_code)]
pub(crate) fn log_param_changes<Z: Zone>(widgets: &[DspWidget<Z>], old_values: &HashMap<String, f32>, path: &str) {
    if std::env::var("NIH_FAUST_JIT_DEBUG").is_err() {
        return;
    }
    for widget in widgets {
        let widget_path = build_widget_path(path, widget.label());

        match widget {
            DspWidget::NumParam { zone, .. } => {
                let new_val = zone.cur_value();
                if let Some(&old_val) = old_values.get(&widget_path) {
                    if (new_val - old_val).abs() > 0.001 {
                        log!(Level::Info, "PARAM CHANGE: {} = {} -> {}", widget_path, old_val, new_val);
                    }
                }
            }
            DspWidget::BoolParam { zone, .. } => {
                let new_val = zone.cur_value();
                if let Some(&old_val) = old_values.get(&widget_path) {
                    if (new_val - old_val).abs() > 0.001 {
                        log!(Level::Info, "PARAM CHANGE: {} = {} -> {}", widget_path, old_val, new_val);
                    }
                }
            }
            DspWidget::Box { inner, .. } => {
                log_param_changes(inner, old_values, &widget_path);
            }
            DspWidget::NumDisplay { .. } => {}
        }
    }
}

/// Recursively restore widget values from a HashMap
fn restore_widget_values(widgets: &mut [DspWidget<&mut f32>], saved: &HashMap<String, f32>, path: &str) {
    for widget in widgets {
        let widget_path = build_widget_path(path, widget.label());

        match widget {
            DspWidget::NumParam { zone, min, max, .. } => {
                // Restore all parameters that have saved values (including nopre ones)
                // nopre only affects what gets saved to preset FILES, not DSP state restoration
                if let Some(&value) = saved.get(&widget_path) {
                    **zone = value.clamp(*min, *max);
                }
            }
            DspWidget::BoolParam { zone, .. } => {
                if let Some(&value) = saved.get(&widget_path) {
                    **zone = value;
                }
            }
            DspWidget::Box { inner, .. } => {
                restore_widget_values(inner, saved, &widget_path);
            }
            DspWidget::NumDisplay { .. } => {
                // Displays are read-only, don't restore
            }
        }
    }
}

impl Default for NihFaustJit {
    fn default() -> Self {
        let playback_state = Arc::new(audio_file::PlaybackState::new());
        Self {
            sample_rate: Arc::new(AtomicF32::new(0.0)),
            params: Arc::new(NihFaustJitParams::default()),
            dsp_state: Arc::new(RwLock::new(DspState::NoDspScript)),
            audio_file_player: Arc::new(RwLock::new(audio_file::AudioFilePlayer::new(
                Arc::clone(&playback_state),
            ))),
            playback_state,
            debug_mode: std::env::var("NIH_FAUST_JIT_DEBUG").is_ok(),
            last_param_values: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl Default for NihFaustJitParams {
    fn default() -> Self {
        Self {
            gain: FloatParam::new("Gain", 1.0, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(50.0)),

            nih_egui_state: nih_plug_egui::EguiState::from_size(800, 700),

            selected_paths: Arc::new(RwLock::new(SelectedPaths {
                dsp_script: std::env::var("NIH_FAUST_JIT_DSP_SCRIPT").ok().map(PathBuf::from),
                dsp_lib_path: env!("DSP_LIBS_PATH").into(),
            })),

            dsp_nvoices: Arc::new(RwLock::new(-1)),  // AutoDetect from script metadata

            faust_param_values: Arc::new(RwLock::new(HashMap::new())),

            preset_loading: Arc::new(AtomicBool::new(false)),
        }
    }
}

pub enum Tasks {
    ReloadDsp,
    LoadAudioFile(PathBuf),
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, strum_macros::EnumIter)]
// We don't reuse faust_jit::DspLoadMode because we need a pure enum here
pub enum DspType {
    AutoDetect,
    Effect,
    Instrument,
}

impl DspType {
    fn from_nvoices(nvoices: i32) -> Self {
        match faust_jit::DspLoadMode::from_nvoices(nvoices) {
            faust_jit::DspLoadMode::AutoDetect => DspType::AutoDetect,
            faust_jit::DspLoadMode::Effect => DspType::Effect,
            faust_jit::DspLoadMode::Instrument { nvoices: _ } => DspType::Instrument,
        }
    }
}

impl Plugin for NihFaustJit {
    const NAME: &'static str = "nih-faust-jit";
    const VENDOR: &'static str = "Yves Pares";
    const URL: &'static str = env!("CARGO_PKG_HOMEPAGE");
    const EMAIL: &'static str = "yves.pares@gmail.com";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // The first audio IO layout is used as the default. The other layouts may be selected either
    // explicitly or automatically by the host or the user depending on the plugin API/backend.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),

        aux_input_ports: &[],
        aux_output_ports: &[],

        // Individual ports and the layout as a whole can be named here. By default these names
        // are generated as needed. This layout will be called 'Stereo', while a layout with
        // only one input and output channel would be called 'Mono'.
        names: PortNames::const_default(),
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::MidiCCs;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::None;

    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();

    type BackgroundTask = Tasks;

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let sample_rate_arc = Arc::clone(&self.sample_rate);
        // This function may be called before self.sample_rate has been properly
        // initialized, and the task executor closure cannot borrow self. This
        // is why the sample rate is stored in an Arc<AtomicF32> which we can
        // read later, when it is actually time to load a DSP

        let selected_paths_arc = Arc::clone(&self.params.selected_paths);
        let dsp_nvoices_arc = Arc::clone(&self.params.dsp_nvoices);
        let dsp_state_arc = Arc::clone(&self.dsp_state);
        let faust_param_values_arc = Arc::clone(&self.params.faust_param_values);
        let preset_loading_arc = Arc::clone(&self.params.preset_loading);
        let audio_file_player_arc = Arc::clone(&self.audio_file_player);
        let playback_state_arc = Arc::clone(&self.playback_state);

        let cache_folder = env!("LLVM_CACHE_FOLDER"); // Build-time env var
        let opt_cache = if cache_folder.is_empty() {
            None
        } else {
            log!(Level::Info, "Caching llvm bytecode in {}", cache_folder);
            Some(faust_jit::Cache::new(PathBuf::from(cache_folder)))
        };

        Box::new(move |task| match task {
            Tasks::LoadAudioFile(path) => {
                let target_sr = sample_rate_arc.load(Ordering::Relaxed) as u32;
                log!(Level::Info, "Loading audio file: {:?} (target SR: {})", path, target_sr);
                match audio_file::load_audio_file(&path, target_sr) {
                    Ok(decoded) => {
                        log!(
                            Level::Info,
                            "Loaded {} ({} channels, {} frames, {} Hz -> {} Hz)",
                            path.display(),
                            decoded.num_channels,
                            decoded.total_frames,
                            decoded.original_sample_rate,
                            decoded.sample_rate
                        );
                        let player = audio_file_player_arc.read().unwrap();
                        player.set_audio(decoded);
                        // Auto-play on load with looping enabled
                        playback_state_arc.position.store(0, Ordering::Relaxed);
                        playback_state_arc.looping.store(true, Ordering::Relaxed);
                        playback_state_arc.playing.store(true, Ordering::Relaxed);
                        log!(Level::Info, "Auto-play enabled for loaded audio file");
                    }
                    Err(e) => {
                        log!(Level::Error, "Failed to load audio file: {}", e);
                    }
                }
            }
            Tasks::ReloadDsp => {
                debug_log!("Reloading DSP...");
                let sample_rate = sample_rate_arc.load(Ordering::Relaxed);
                let selected_paths = selected_paths_arc.read().unwrap();
                let dsp_nvoices = *dsp_nvoices_arc.read().unwrap();
                let new_dsp_state = match &selected_paths.dsp_script {
                    Some(script_path) => {
                        match faust_jit::SingletonDsp::from_file(
                            opt_cache.as_ref(),
                            script_path,
                            &[&selected_paths.dsp_lib_path],
                            sample_rate as i32,
                            &faust_jit::DspLoadMode::from_nvoices(dsp_nvoices),
                        ) {
                            Err(msg) => DspState::Failed(msg),
                            Ok(dsp) => {
                                if dsp.info.num_inputs <= 2 && dsp.info.num_outputs <= 2 {
                                    // Restore saved parameter values
                                    let saved_values = faust_param_values_arc.read().unwrap();
                                    if !saved_values.is_empty() {
                                        dsp.with_widgets_mut(|widgets| {
                                            restore_widget_values(widgets, &saved_values, "");
                                        });
                                        debug_log!("Restored {} parameter values", saved_values.len());
                                    }
                                    DspState::Loaded(dsp)
                                } else {
                                    DspState::Failed(
                                        format!("DSP has {} input and {} output channels. Max is 2 for each", dsp.info.num_inputs, dsp.info.num_outputs)
                                    )
                                }
                            }
                        }
                    }
                    None => DspState::NoDspScript,
                };
                debug_log!(
                    "Loaded {:?} with sample_rate={}, nvoices={} => {:?}",
                    selected_paths,
                    sample_rate,
                    dsp_nvoices,
                    new_dsp_state
                );
                // This is the only place where the whole DSP state is locked in
                // write mode, and only so we can swap it with the newly loaded
                // one:
                *dsp_state_arc.write().unwrap() = new_dsp_state;
                // Clear preset loading flag (even on failure, to avoid getting stuck)
                preset_loading_arc.store(false, Ordering::SeqCst);
            }
        })
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        init_ctx: &mut impl InitContext<Self>,
    ) -> bool {
        // Resize buffers and perform other potentially expensive initialization operations here.
        // The `reset()` function is always called right after this function. You can remove this
        // function if you do not need it.
        self.sample_rate
            .store(buffer_config.sample_rate, Ordering::Relaxed);

        // Override DSP script if provided via command line
        if let Ok(dsp_path) = std::env::var("NIH_FAUST_JIT_DSP_SCRIPT") {
            let mut paths = self.params.selected_paths.write().unwrap();
            paths.dsp_script = Some(PathBuf::from(dsp_path));
        }

        // Load audio file if provided via command line
        if let Ok(audio_path) = std::env::var("NIH_FAUST_JIT_AUDIO_FILE") {
            init_ctx.execute(Tasks::LoadAudioFile(PathBuf::from(audio_path)));
        }

        init_ctx.execute(Tasks::ReloadDsp);
        true
    }

    fn reset(&mut self) {
        // Reset buffers and envelopes here. This can be called from the audio thread and may not
        // allocate. You can remove this function if you do not need it.
    }

    fn editor(&mut self, async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        gui::create_gui(self.gui_arcs(), async_executor)
    }

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        process_ctx: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Audio file playback: fill input buffer from loaded audio file
        let _audio_file_filled = if let Ok(player) = self.audio_file_player.try_read() {
            if let Some(audio) = player.get_audio_for_playback() {
                audio_file::fill_buffer_from_audio(&audio, &self.playback_state, buffer.as_slice())
            } else {
                false
            }
        } else {
            false
        };

        let dsp_state_guard = self.dsp_state.read().unwrap();

        // Debug: Log when both audio file is playing AND DSP is loaded
        static PROCESS_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = PROCESS_COUNT.fetch_add(1, Ordering::Relaxed);

        // Log every ~1 second (assuming 44100 Hz, 512 sample blocks = ~86 blocks/sec)
        if self.debug_mode {
            let params = self.params.faust_param_values.read().unwrap();
            let mut last_values = self.last_param_values.lock().unwrap();

            let mut changes = Vec::new();
            for (path, &val) in params.iter() {
                let prev = last_values.get(path).copied().unwrap_or(f32::NAN);
                if (val - prev).abs() > 0.0001 {
                    changes.push(format!("{}={:.4}", path, val));
                    last_values.insert(path.clone(), val);
                }
            }

            if !changes.is_empty() {
                log!(Level::Info, "PARAM CHANGES: {}", changes.join(", "));
            }
        }

        if let DspState::Loaded(dsp) = &*dsp_state_guard {
            // Handling transport & clock:
            let tp = process_ctx.transport();
            let opt_clock_data = match (tp.tempo, tp.pos_samples()) {
                (Some(tempo), Some(next_buffer_sample_position)) => Some(faust_jit::ClockData {
                    tempo,
                    next_buffer_size: buffer.samples(),
                    next_buffer_sample_position,
                }),
                _ => None,
            };
            dsp.handle_midi_sync(tp.playing, &opt_clock_data);

            // Handling MIDI events:
            while let Some(midi_event) = process_ctx.next_event() {
                let time = midi_event.timing() as f64;
                match midi_event.as_midi() {
                    None | Some(MidiResult::SysEx(_, _)) => { /* We ignore SysEx messages */ }
                    Some(MidiResult::Basic(bytes)) => dsp.handle_raw_midi(time, bytes),
                }
            }

            // Processing audio buffers:
            // Note: For mono DSPs (1 in, 1 out), only buffer[0] is processed
            
            // DEBUG: Log sample values before and after DSP processing
            let log_samples = count % 100 == 50; // Log different blocks than status logs
            let pre_rms = if log_samples {
                let buf = buffer.as_slice();
                let sum: f32 = buf[0].iter().take(64).map(|s| s * s).sum();
                (sum / 64.0).sqrt()
            } else { 0.0 };
            let pre_samples = if log_samples {
                let buf = buffer.as_slice();
                format!("{:.4}, {:.4}, {:.4}", buf[0].get(0).unwrap_or(&0.0), buf[0].get(1).unwrap_or(&0.0), buf[0].get(2).unwrap_or(&0.0))
            } else { String::new() };
            
            dsp.process_buffers(buffer.as_slice());
            
            if self.debug_mode && log_samples {
                let buf = buffer.as_slice();
                let post_sum: f32 = buf[0].iter().take(64).map(|s| s * s).sum();
                let post_rms = (post_sum / 64.0).sqrt();
                let post_samples = format!("{:.4}, {:.4}, {:.4}", buf[0].get(0).unwrap_or(&0.0), buf[0].get(1).unwrap_or(&0.0), buf[0].get(2).unwrap_or(&0.0));
                log!(Level::Info, "DSP: pre_rms={:.6}, post_rms={:.6}, pre=[{}], post=[{}]", 
                    pre_rms, post_rms, pre_samples, post_samples);
            }
            
            // For mono DSPs: copy processed left channel to right channel
            let is_mono_dsp = dsp.info.num_inputs <= 1 && dsp.info.num_outputs <= 1;
            if is_mono_dsp && buffer.channels() >= 2 {
                let buf_slice = buffer.as_slice();
                if buf_slice.len() >= 2 {
                    for i in 0..buf_slice[0].len() {
                        buf_slice[1][i] = buf_slice[0][i];
                    }
                }
            }
        }
        // Applying Gain parameter:
        for channel_samples in buffer.iter_samples() {
            let gain = self.params.gain.smoothed.next();

            for sample in channel_samples {
                *sample *= gain;
            }
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for NihFaustJit {
    const CLAP_ID: &'static str = "com.ypares.nih-faust-jit";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Using jit-compiled Faust DSP scripts");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;

    // Don't forget to change these features
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Instrument,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for NihFaustJit {
    const VST3_CLASS_ID: [u8; 16] = *b"nih-faust-jit-yp";

    // And also don't forget to change these categories
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[
        Vst3SubCategory::Fx,
        Vst3SubCategory::Instrument,
        Vst3SubCategory::Stereo,
    ];
}

nih_export_clap!(NihFaustJit);
nih_export_vst3!(NihFaustJit);
