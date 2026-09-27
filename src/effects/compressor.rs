//! A feed-forward compressor: track level with an asymmetric (attack vs.
//! release) envelope follower in the dB domain, and reduce gain above a
//! threshold by `1 - 1/ratio`. Standard design — the one deliberate
//! simplification is no look-ahead, so very fast transients can poke a few
//! samples above the target level before the envelope catches up. That's
//! what [`super::limiter::Limiter`] downstream in [`super::EffectsChain`]
//! is for.

pub struct Compressor {
    threshold_db: f32,
    ratio: f32,
    makeup_db: f32,
    attack_coeff: f32,
    release_coeff: f32,
    /// Smoothed level estimate, in dB.
    envelope_db: f32,
}

impl Compressor {
    pub fn new(threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, sample_rate: f32) -> Self {
        Self {
            threshold_db,
            ratio: ratio.max(1.0),
            makeup_db: 0.0,
            attack_coeff: one_pole_coeff(attack_ms, sample_rate),
            release_coeff: one_pole_coeff(release_ms, sample_rate),
            envelope_db: -120.0,
        }
    }

    /// Flat gain added after compression, e.g. to compensate for the
    /// average level a heavy ratio takes away.
    pub fn with_makeup_gain(mut self, makeup_db: f32) -> Self {
        self.makeup_db = makeup_db;
        self
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let level_db = 20.0 * s.abs().max(1e-9).log10();
            let coeff = if level_db > self.envelope_db { self.attack_coeff } else { self.release_coeff };
            self.envelope_db = coeff * self.envelope_db + (1.0 - coeff) * level_db;

            let gain_reduction_db = if self.envelope_db > self.threshold_db {
                (self.envelope_db - self.threshold_db) * (1.0 / self.ratio - 1.0)
            } else {
                0.0
            };
            let gain = 10f32.powf((gain_reduction_db + self.makeup_db) / 20.0);
            *s *= gain;
        }
    }
}

/// One-pole smoothing coefficient for a given time constant, so
/// `output += (1 - coeff) * (target - output)` reaches ~63% of the way to
/// `target` in `time_ms`.
fn one_pole_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (time_ms.max(0.01) * 0.001 * sample_rate)).exp()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, amplitude: f32, n: usize, sample_rate: f32) -> Vec<f32> {
        (0..n).map(|i| amplitude * (TAU * freq * i as f32 / sample_rate).sin()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0f32, |m, s| m.max(s.abs()))
    }

    #[test]
    fn signal_below_threshold_is_unaffected() {
        let mut comp = Compressor::new(-6.0, 4.0, 5.0, 100.0, 48_000.0);
        let quiet = sine(440.0, 0.05, 2000, 48_000.0); // well below -6 dBFS
        let mut buf = quiet.clone();
        comp.process(&mut buf);
        // Once the envelope has settled below threshold, gain reduction is 0.
        for (a, b) in quiet[500..].iter().zip(buf[500..].iter()) {
            assert!((a - b).abs() < 1e-4, "expected near-identity below threshold: {a} vs {b}");
        }
    }

    #[test]
    fn loud_signal_is_attenuated_toward_the_target_curve() {
        let mut comp = Compressor::new(-20.0, 4.0, 5.0, 100.0, 48_000.0);
        let loud = sine(440.0, 0.9, 4000, 48_000.0); // near full scale, well above threshold
        let mut buf = loud.clone();
        comp.process(&mut buf);
        let loud_peak = peak(&loud[2000..]);
        let compressed_peak = peak(&buf[2000..]);
        assert!(
            compressed_peak < loud_peak * 0.8,
            "expected meaningful gain reduction once settled: loud={loud_peak} compressed={compressed_peak}"
        );
    }

    #[test]
    fn higher_ratio_compresses_more() {
        let loud = sine(440.0, 0.9, 4000, 48_000.0);

        let mut gentle = Compressor::new(-20.0, 2.0, 5.0, 100.0, 48_000.0);
        let mut buf_gentle = loud.clone();
        gentle.process(&mut buf_gentle);

        let mut hard = Compressor::new(-20.0, 10.0, 5.0, 100.0, 48_000.0);
        let mut buf_hard = loud.clone();
        hard.process(&mut buf_hard);

        assert!(peak(&buf_hard[2000..]) < peak(&buf_gentle[2000..]));
    }
}
