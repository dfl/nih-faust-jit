//! Custom standalone entry point with device configuration support.
//!
//! Loads saved device preferences and passes them as CLI args to nih_plug's standalone.

use nih_faust_jit::{config, NihFaustJit};
use nih_plug::prelude::nih_export_standalone_with_args;

fn main() {
    // Set up Ctrl+C handler for graceful shutdown
    ctrlc::set_handler(|| {
        std::process::exit(0);
    }).ok();

    // Load saved configuration
    let cfg = config::StandaloneConfig::load();

    // Build command line arguments based on config
    let args: Vec<String> = std::env::args().collect();

    // Extract positional arguments (DSP script and audio file)
    let mut filtered_args = Vec::new();
    let mut positional_args = Vec::new();
    
    let mut it = args.into_iter();
    if let Some(exe) = it.next() {
        filtered_args.push(exe);
    }
    
    while let Some(arg) = it.next() {
        if arg == "standalone" {
            filtered_args.push(arg);
            continue;
        }

        if arg == "--debug" {
            std::env::set_var("NIH_FAUST_JIT_DEBUG", "1");
            continue;
        }
        
        if arg.starts_with('-') {
            let has_value = matches!(
                arg.as_str(),
                "--output-device" | "-o" |
                "--input-device" | "-i" |
                "--midi-input" | "-m" |
                "--buffer-size" | "-b" |
                "--sample-rate" | "-s" |
                "--input-gain" | "-g" |
                "--output-gain"
            );
            filtered_args.push(arg);
            if has_value {
                if let Some(val) = it.next() {
                    filtered_args.push(val);
                }
            }
        } else {
            positional_args.push(arg);
        }
    }
    
    if let Some(dsp) = positional_args.get(0) {
        std::env::set_var("NIH_FAUST_JIT_DSP_SCRIPT", dsp);
    }
    if let Some(audio) = positional_args.get(1) {
        std::env::set_var("NIH_FAUST_JIT_AUDIO_FILE", audio);
    }
    
    let mut args = filtered_args;

    // Only add device args if not already specified on command line
    let has_output = args.iter().any(|a| a == "--output-device" || a == "-o");
    let has_input = args.iter().any(|a| a == "--input-device" || a == "-i");
    let has_midi = args.iter().any(|a| a == "--midi-input" || a == "-m");

    // Add output device (default to system default if not configured)
    if !has_output {
        let output = cfg
            .output_device
            .or_else(config::default_output_device);
        if let Some(device) = output {
            args.push("--output-device".to_string());
            args.push(device);
        }
    }

    // Add input device if configured
    if !has_input {
        if let Some(device) = &cfg.input_device {
            args.push("--input-device".to_string());
            args.push(device.clone());
        }
    }

    // Add MIDI input if configured (or auto-select first available)
    if !has_midi {
        let midi = cfg.midi_input.or_else(|| {
            config::enumerate_midi_inputs().get(0).cloned()
        });
        if let Some(device) = midi {
            args.push("--midi-input".to_string());
            args.push(device);
        }
    }

    // Add sample rate if configured and not specified on command line
    let has_sample_rate = args.iter().any(|a| a == "--sample-rate" || a == "-s");
    if !has_sample_rate {
        if let Some(sr) = cfg.sample_rate {
            args.push("--sample-rate".to_string());
            args.push(sr.to_string());
        }
    }

    // Call the nih_plug standalone entry point with our modified args
    nih_export_standalone_with_args::<NihFaustJit, _>(args);
}
