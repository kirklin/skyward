// Run the production waveform tests on Linux without opening a motor.
#![cfg(target_os = "linux")]

#[allow(dead_code)]
#[path = "../src/haptics.rs"]
mod haptics;
