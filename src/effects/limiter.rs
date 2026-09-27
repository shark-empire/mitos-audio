//! A peak limiter: instant attack (a sample is never allowed through above
//! `ceiling`), gradual release back toward unity gain. Runs last in
//! [`super::EffectsChain`] as a safety net — EQ boosts and the stereo
//! widener's mid/side recombination can each push a signal that started
//! under 0 dBFS back over it.

pub struct Limiter {
    /// Linear amplitude ceiling (e.g. `10^(-1/20)` for a -1 dBFS ceiling).
    ceiling: f32,
    release_coeff: f32,
    gain: f32,
}

impl Limiter {
    pub fn new(ceiling_db: f32, release_ms: f32, sample_rate: f32) -> Self {
        Self {
            ceiling: 10f32.powf(ceiling_db / 20.0),
            release_coeff: (-1.0 / (release_ms.max(0.01) * 0.001 * sample_rate)).exp(),
            gain: 1.0,
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for s in buf.iter_mut() {
            let peak = s.abs();
            let instantaneous_gain = if peak > self.ceiling { self.ceiling / peak } else { 1.0 };
            if instantaneous_gain < self.gain {
                // Attack: drop immediately, this sample must not clip.
                self.gain = instantaneous_gain;
            } else {
                // Release: ease back up.
                self.gain += (instantaneous_gain - self.gain) * (1.0 - self.release_coeff);
            }
            *s = (*s * self.gain).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_exceeds_ceiling() {
        let ceiling_db = -1.0;
        let mut limiter = Limiter::new(ceiling_db, 50.0, 48_000.0);
        let ceiling_linear = 10f32.powf(ceiling_db / 20.0);
        let mut buf = vec![0.0f32; 200];
        // A sharp step to full scale, which is exactly what a limiter must
        // catch even on the very first over-ceiling sample.
        for (i, s) in buf.iter_mut().enumerate() {
            *s = if i < 100 { 0.1 } else { 1.0 };
        }
        limiter.process(&mut buf);
        assert!(
            buf.iter().all(|s| s.abs() <= ceiling_linear + 1e-6),
            "no sample may exceed the ceiling, even the first one after a sudden jump"
        );
    }

    #[test]
    fn leaves_quiet_signal_untouched() {
        let mut limiter = Limiter::new(-1.0, 50.0, 48_000.0);
        let original = vec![0.1f32, -0.15, 0.2, -0.05];
        let mut buf = original.clone();
        limiter.process(&mut buf);
        for (a, b) in original.iter().zip(buf.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn recovers_gain_after_the_loud_passage_ends() {
        let mut limiter = Limiter::new(-1.0, 20.0, 48_000.0);
        let mut loud = vec![1.0f32; 500];
        limiter.process(&mut loud);
        // Gain should have dropped well below 1.0 to tame the loud passage.
        assert!(limiter.gain < 0.95);

        let mut quiet = vec![0.05f32; 3000];
        limiter.process(&mut quiet);
        // And recovered substantially back toward unity afterward.
        assert!(limiter.gain > 0.99, "expected gain to recover toward 1.0, got {}", limiter.gain);
    }
}
