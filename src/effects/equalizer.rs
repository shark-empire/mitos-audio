//! A 10-band graphic equalizer built from cascaded peaking biquad filters.
//!
//! The filter design is the standard "Audio EQ Cookbook" (Robert
//! Bristow-Johnson) peaking-EQ biquad — not a novel derivation, just a
//! correct, widely-used implementation of it. Each band is independent
//! (its own coefficients and its own per-channel filter state); with every
//! band at 0 dB, `process` takes a fast no-op path rather than running ten
//! filters that would each individually resolve to unity gain anyway.

use std::f32::consts::TAU;

/// Center frequencies for the 10 bands, in the order every `[f32; 10]`
/// gains array in this module uses.
pub const BAND_FREQUENCIES: [f32; 10] =
    [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];

const BAND_Q: f32 = 1.0;
const FLAT_EPSILON: f32 = 0.01;

#[derive(Debug, Clone, Copy)]
struct BiquadCoeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl BiquadCoeffs {
    fn identity() -> Self {
        Self { b0: 1.0, b1: 0.0, b2: 0.0, a1: 0.0, a2: 0.0 }
    }

    /// RBJ cookbook peaking EQ coefficients, already normalized by a0.
    fn peaking(freq: f32, q: f32, gain_db: f32, sample_rate: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = TAU * freq / sample_rate;
        let (sin_w0, cos_w0) = (w0.sin(), w0.cos());
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a;
        let a0 = 1.0 + alpha / a;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a;

        Self { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0 }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BiquadState {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl BiquadState {
    fn process(&mut self, c: &BiquadCoeffs, x0: f32) -> f32 {
        let y0 = c.b0 * x0 + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x0;
        self.y2 = self.y1;
        self.y1 = y0;
        y0
    }
}

pub struct Equalizer {
    sample_rate: f32,
    channels: usize,
    gains_db: [f32; 10],
    coeffs: [BiquadCoeffs; 10],
    // One independent filter-state array per channel — biquads carry
    // memory (the last two in/out samples), so channels cannot share it.
    state: Vec<[BiquadState; 10]>,
}

impl Equalizer {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        let channels = channels.max(1);
        Self {
            sample_rate,
            channels,
            gains_db: [0.0; 10],
            coeffs: [BiquadCoeffs::identity(); 10],
            state: vec![[BiquadState::default(); 10]; channels],
        }
    }

    pub fn bands(&self) -> [f32; 10] {
        self.gains_db
    }

    /// Set all 10 band gains (dB) and recompute filter coefficients.
    /// Existing filter *state* (the running memory) is left alone — only
    /// changing gains mid-stream would otherwise click.
    pub fn set_bands(&mut self, gains_db: [f32; 10]) {
        self.gains_db = gains_db;
        for i in 0..10 {
            self.coeffs[i] = if gains_db[i].abs() < FLAT_EPSILON {
                BiquadCoeffs::identity()
            } else {
                BiquadCoeffs::peaking(BAND_FREQUENCIES[i], BAND_Q, gains_db[i], self.sample_rate)
            };
        }
    }

    /// Process interleaved audio (`channels` from `new`) in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.gains_db.iter().all(|g| g.abs() < FLAT_EPSILON) {
            return;
        }
        for (i, sample) in buf.iter_mut().enumerate() {
            let ch = i % self.channels;
            let mut x = *sample;
            for band in 0..10 {
                if self.gains_db[band].abs() >= FLAT_EPSILON {
                    x = self.state[ch][band].process(&self.coeffs[band], x);
                }
            }
            *sample = x;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU as TAU64;

    fn sine(freq: f32, amplitude: f32, n: usize, sample_rate: f32) -> Vec<f32> {
        (0..n).map(|i| amplitude * (TAU64 * freq * i as f32 / sample_rate).sin()).collect()
    }

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    #[test]
    fn flat_bands_are_a_no_op() {
        let mut eq = Equalizer::new(48_000.0, 1);
        let original = sine(1000.0, 0.5, 200, 48_000.0);
        let mut buf = original.clone();
        eq.process(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn boosting_a_band_increases_energy_at_its_center_frequency() {
        let sample_rate = 48_000.0;
        let dry = sine(1000.0, 0.2, 2000, sample_rate);

        let mut eq = Equalizer::new(sample_rate, 1);
        let mut bands = [0.0; 10];
        bands[5] = 12.0; // 1000 Hz band, +12 dB
        eq.set_bands(bands);
        let mut wet = dry.clone();
        eq.process(&mut wet);

        // Compare steady state only — skip the filter's brief settling
        // transient at the very start.
        let dry_rms = rms(&dry[500..]);
        let wet_rms = rms(&wet[500..]);
        assert!(
            wet_rms > dry_rms * 2.0,
            "expected a substantial boost at the band's center frequency: dry={dry_rms} wet={wet_rms}"
        );
    }

    #[test]
    fn cutting_a_band_decreases_energy_at_its_center_frequency() {
        let sample_rate = 48_000.0;
        let dry = sine(1000.0, 0.5, 2000, sample_rate);

        let mut eq = Equalizer::new(sample_rate, 1);
        let mut bands = [0.0; 10];
        bands[5] = -12.0;
        eq.set_bands(bands);
        let mut wet = dry.clone();
        eq.process(&mut wet);

        let dry_rms = rms(&dry[500..]);
        let wet_rms = rms(&wet[500..]);
        assert!(
            wet_rms < dry_rms * 0.5,
            "expected a substantial cut at the band's center frequency: dry={dry_rms} wet={wet_rms}"
        );
    }

    #[test]
    fn stereo_channels_are_filtered_independently() {
        // Interleaved stereo where only the left channel carries the
        // boosted band's tone; the right channel is silence. If channel
        // state were shared, the right channel would pick up the left's
        // filter memory.
        let sample_rate = 48_000.0;
        let mono = sine(1000.0, 0.3, 1000, sample_rate);
        let mut interleaved = vec![0.0f32; mono.len() * 2];
        for (i, s) in mono.iter().enumerate() {
            interleaved[i * 2] = *s; // left
                                     // right stays 0.0
        }

        let mut eq = Equalizer::new(sample_rate, 2);
        let mut bands = [0.0; 10];
        bands[5] = 12.0;
        eq.set_bands(bands);
        eq.process(&mut interleaved);

        let right: Vec<f32> = interleaved.iter().skip(1).step_by(2).copied().collect();
        assert!(right.iter().all(|&s| s == 0.0), "silent channel must stay silent");
    }
}
