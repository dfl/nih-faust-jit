# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased] - Enhanced Branch

### Added

- **Knob widget** with 286° arc and vertical drag control
- **LED widget** with glow effect for boolean indicators
- **VU meter improvements** with color segments (green/orange/red zones) and smooth peak decay
- **Preset system** - save, load, and delete presets per DSP script
- **Audio file playback** in standalone mode (WAV, MP3, FLAC, M4A support)
- **Keyboard shortcuts**
  - CMD-R / CTRL-R to reload DSP script
  - Space bar to toggle audio playback
- **Parameter ordering** via `[N]` metadata in Faust code (standard Faust syntax)
- **Log/exp scale support** for sliders and bargraphs
- **dB unit detection** for automatic logarithmic scaling
- **Value tooltips** with fade animation on parameter hover
- **Shift-drag** for fine control (0.1x sensitivity)
- **Parameter persistence** across sessions (plugin mode)
- **File edit button** to open DSP script in system editor
- **Command-line options** for specifying DSP and audio files
- **Conditional debug logging** for parameter changes

### Fixed

- **Mono effect DSP loading** - effects now process audio input correctly instead of being treated as voice generators
- **Race condition in preset loading** - preset values now survive DSP reload
- **UI parameter sorting** - proper ordering with `[N]` metadata support

### Changed

- Default build config targets macOS/Homebrew (was Windows static linking)
- Updated to egui 0.31
- Replaced egui_file with rfd for native file dialogs
- Improved README documentation with new feature descriptions
