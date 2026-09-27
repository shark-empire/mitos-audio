//! Noise suppression: an adaptive noise gate.
//!
//! This is an amplitude-domain gate, not spectral denoising — it tracks a
//! running noise-floor estimate and smoothly attenuates blocks whose level
//! sits close to that floor, rather than separating noise from signal
//! frequency-by-frequency (as e.g. RNNoise or WebRTC's spectral suppressor
//! do). That makes it cheap and dependency-free, and effective against
//! steady background hiss/hum between speech; it will not lift a voice out
//! of noise that overlaps it in both time and level. Swapping in a spectral
//! suppressor later is a drop-in replacement — it would sit at the same
//! point in `MicrophoneProcessor`, with the same `process(&mut [f32])`
//! shape.
//!
//! The floor tracker deliberately rises slowly and falls quickly: a burst
//! of loud sound (a word, a door slam) must not redefine "quiet" for the
//! next ten seconds, but genuine quiet should be recognized promptly.

pub struct NoiseGate {
    /// Running noise-floor estimate (linear RMS).
    floor: f32,
    /// Smoothed gate gain actually applied (1.0 = fully open).
    gate_gain: f32,
    /// A block is judged "noise" when its RMS is below `floor * threshold_ratio`.
    threshold_ratio: f32,
    /// Linear gain applied to a gated (judged-as-noise) block.
    attenuation: f32,
    /// One-pole smoothing applied to `gate_gain` itself, so the gate opens
    /// and closes over a few blocks instead of clicking instantly.
    smoothing: f32,
}

impl NoiseGate {
    pub fn new() -> Self {
        Self {
            floor: 0.01,
            gate_gain: 1.0,
            threshold_ratio: 4.0,
            attenuation: 0.05, // about -26 dB, not full silence (avoids an audible hard cut)
            smoothing: 0.2,
        }
    }

    pub fn gate_gain(&self) -> f32 {
        self.gate_gain
    }

    pub fn floor(&self) -> f32 {
        self.floor
    }

    /// Update the floor estimate and gate gain from this block, then apply
    /// the (smoothed) gate to it in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }
        let rms = rms_of(buf);
        if rms < self.floor {
            self.floor += (rms - self.floor) * 0.3; // fall quickly
        } else {
            self.floor += (rms - self.floor) * 0.01; // rise slowly
        }
        self.floor = self.floor.max(1e-5);

        let target_gain = if rms < self.floor * self.threshold_ratio {
            self.attenuation
        } else {
            1.0
        };
        self.gate_gain += (target_gain - self.gate_gain) * self.smoothing;

        for s in buf.iter_mut() {
            *s *= self.gate_gain;
        }
    }
}

impl Default for NoiseGate {
    fn default() -> Self {
        Self::new()
    }
}

fn rms_of(buf: &[f32]) -> f32 {
    let sum_sq: f32 = buf.iter().map(|s| s * s).sum();
    (sum_sq / buf.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    /// A small deterministic LCG so tests don't need a `rand` dependency.
    fn pseudo_noise(amplitude: f32, n: usize, seed: u32) -> Vec<f32> {
        let mut state = seed.wrapping_mul(2654435761).wrapping_add(1);
        (0..n)
            .map(|_| {
                state = state.wrapping_mul(1103515245).wrapping_add(12345);
                let unit = ((state >> 8) as f32 / u32::MAX as f32) * 2.0 - 1.0;
                unit * amplitude
            })
            .collect()
    }

    fn sine(freq: f32, amplitude: f32, n: usize, sample_rate: f32) -> Vec<f32> {
        (0..n).map(|i| amplitude * (TAU * freq * i as f32 / sample_rate).sin()).collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        rms_of(buf)
    }

    #[test]
    fn steady_low_level_noise_is_attenuated() {
        let mut gate = NoiseGate::new();
        let mut last_out_rms = 1.0;
        let mut in_rms = 1.0;
        for i in 0..60 {
            let mut block = pseudo_noise(0.002, 480, i);
            in_rms = rms(&block);
            gate.process(&mut block);
            last_out_rms = rms(&block);
        }
        assert!(
            last_out_rms < in_rms * 0.5,
            "expected the steady low-level block to be attenuated: in={in_rms} out={last_out_rms}"
        );
    }

    #[test]
    fn sustained_loud_tone_is_never_gated() {
        let mut gate = NoiseGate::new();
        let mut in_rms = 0.0;
        let mut out_rms = 0.0;
        for _ in 0..40 {
            let mut block = sine(440.0, 0.3, 480, 48_000.0);
            in_rms = rms(&block);
            gate.process(&mut block);
            out_rms = rms(&block);
        }
        assert!(
            out_rms > in_rms * 0.9,
            "a sustained loud tone should stay essentially un-attenuated: in={in_rms} out={out_rms}"
        );
    }

    #[test]
    fn gate_opens_once_a_loud_block_follows_quiet_ones() {
        let mut gate = NoiseGate::new();
        // Let the floor settle on quiet noise first.
        for i in 0..30 {
            let mut block = pseudo_noise(0.002, 480, i);
            gate.process(&mut block);
        }
        assert!(gate.gate_gain() < 0.5, "gate should have closed on the quiet run");

        // Now a real, loud voice-like tone arrives.
        let mut in_rms = 0.0;
        let mut out_rms = 0.0;
        for _ in 0..30 {
            let mut block = sine(300.0, 0.3, 480, 48_000.0);
            in_rms = rms(&block);
            gate.process(&mut block);
            out_rms = rms(&block);
        }
        assert!(
            out_rms > in_rms * 0.9,
            "gate should have opened back up for the loud block: in={in_rms} out={out_rms}"
        );
    }
}
