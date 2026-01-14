// Test Signal Generator for nih-faust-jit
// Provides various test signals for DSP testing and analysis

import("stdfaust.lib");

// ============================================================================
// Signal Selection
// ============================================================================

// Signal type: 0=Off, 1=Sine, 2=Square, 3=Triangle, 4=Sawtooth,
//              5=White Noise, 6=Pink Noise, 7=Impulse, 8=Frequency Sweep
signal_type = nentry("[0]Signal Type[style:menu{'Off':0;'Sine':1;'Square':2;'Triangle':3;'Sawtooth':4;'White Noise':5;'Pink Noise':6;'Impulse':7;'Frequency Sweep':8}]", 1, 0, 8, 1);

// ============================================================================
// Oscillator Controls
// ============================================================================

freq = hslider("[1]Frequency[unit:Hz][scale:log]", 440, 20, 20000, 1);
amp_db = hslider("[2]Amplitude[unit:dB]", -12, -60, 0, 0.1);
amp = ba.db2linear(amp_db);

// ============================================================================
// Frequency Sweep Controls
// ============================================================================

sweep_group(x) = hgroup("[3]Sweep", x);
sweep_start = sweep_group(hslider("[0]Start[unit:Hz][scale:log]", 20, 20, 20000, 1));
sweep_end = sweep_group(hslider("[1]End[unit:Hz][scale:log]", 20000, 20, 20000, 1));
sweep_time = sweep_group(hslider("[2]Duration[unit:s]", 5, 0.1, 30, 0.1));
sweep_mode = sweep_group(nentry("[3]Mode[style:menu{'One-shot':0;'Loop':1;'Ping-pong':2}]", 1, 0, 2, 1));

// ============================================================================
// Impulse Controls
// ============================================================================

impulse_group(x) = hgroup("[4]Impulse", x);
impulse_rate = impulse_group(hslider("[0]Rate[unit:Hz][scale:log]", 1, 0.1, 100, 0.1));

// ============================================================================
// Signal Generators
// ============================================================================

// Anti-aliased oscillators using PolyBLEP
sine_osc = os.osc(freq);
square_osc = os.square(freq);          // Alias-suppressed square
triangle_osc = os.triangle(freq);      // Alias-suppressed triangle
sawtooth_osc = os.sawtooth(freq);      // Alias-suppressed sawtooth

// Noise generators
white_noise = no.noise;
pink_noise = no.pink_noise;

// Impulse train
impulse_train = os.imptrain(impulse_rate);

// Frequency sweep with multiple modes
sweep_phasor = os.phasor(1, 1.0/sweep_time);

// Apply sweep mode
sweep_phase =
    (sweep_mode == 0) * min(sweep_phasor, 1) +                        // One-shot
    (sweep_mode == 1) * sweep_phasor +                                 // Loop
    (sweep_mode == 2) * (1 - abs(2*sweep_phasor - 1));                 // Ping-pong

// Logarithmic frequency interpolation
sweep_freq = sweep_start * pow(sweep_end/sweep_start, sweep_phase);
sweep_osc = os.osc(sweep_freq);

// ============================================================================
// Signal Selection and Output
// ============================================================================

// Select signal based on type
selected_signal =
    (signal_type == 0) * 0 +                   // Off
    (signal_type == 1) * sine_osc +            // Sine
    (signal_type == 2) * square_osc +          // Square
    (signal_type == 3) * triangle_osc +        // Triangle
    (signal_type == 4) * sawtooth_osc +        // Sawtooth
    (signal_type == 5) * white_noise +         // White Noise
    (signal_type == 6) * pink_noise +          // Pink Noise
    (signal_type == 7) * impulse_train +       // Impulse
    (signal_type == 8) * sweep_osc;            // Frequency Sweep

// Apply amplitude and output stereo
process = selected_signal * amp <: _, _;
