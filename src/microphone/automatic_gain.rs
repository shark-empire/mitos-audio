//! A simple feedback automatic gain control (AGC).
//!
//! Tracks the block RMS of incoming audio and moves a single gain factor
//! toward whatever would put that RMS at `target_rms`, faster when turning
//! *down* (attack — react quickly to a sudden loud burst) than when turning
//! *up* (release — don't visibly "breathe" during quiet passages). Gain is
//! bounded to `±max_gain_db` so it can't amplify near-silence into audible
//! noise, and it holds steady (rather than climbing to the ceiling) during
//! true silence — see `holds_gain_during_silence` below.
//!
//! This is block-based (call [`AutomaticGainControl::process`] once per
//! chunk of samples you have on hand — e.g. once per 100 ms capture read),
//! not a per-sample compressor; see `crate::effects::compressor` for that.

use super::gain::db_to_linear;

pub struct AutomaticGainControl {
    target_rms: f32,
    gain: f32,
    max_gain: f32,
    min_gain: f32,
    attack: f32,
    release: f32,
}

impl AutomaticGainControl {
    /// `target_rms` is the linear RMS level (0.0-1.0) AGC aims for.
    /// `max_gain_db` bounds how far gain can move in either direction.
    pub fn new(target_rms: f32, max_gain_db: f32) -> Self {
        Self {
            target_rms: target_rms.max(1e-6),
            gain: 1.0,
            max_gain: db_to_linear(max_gain_db),
            min_gain: db_to_linear(-max_gain_db),
            attack: 0.5,
            release: 0.05,
        }
    }

    /// Sensible defaults for voice: target -18 dBFS RMS, up to 24 dB of
    /// makeup gain either way.
    pub fn for_voice() -> Self {
        Self::new(db_to_linear(-18.0), 24.0)
    }

    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Adjust gain based on this block's level, then apply it in place.
    pub fn process(&mut self, buf: &mut [f32]) {
        if buf.is_empty() {
            return;
        }
        let rms = rms_of(buf);
        // Below this, treat the block as silence: hold gain steady instead
        // of climbing toward max_gain chasing a level that isn't real
        // signal (comfort noise, a muted mic's DC offset, etc).
        if rms > 1e-4 {
            let desired = (self.target_rms / rms).clamp(self.min_gain, self.max_gain);
            let coeff = if desired < self.gain { self.attack } else { self.release };
            self.gain += (desired - self.gain) * coeff;
        }
        for s in buf.iter_mut() {
            *s = (*s * self.gain).clamp(-1.0, 1.0);
        }
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

    fn sine(freq: f32, amplitude: f32, n: usize, sample_rate: f32) -> Vec<f32> {
        (0..n).map(|i| amplitude * (TAU * freq * i as f32 / sample_rate).sin()).collect()
    }

    #[test]
    fn boosts_quiet_signal_toward_target() {
        let mut agc = AutomaticGainControl::new(0.2, 30.0);
        let mut last_rms = 0.0;
        for _ in 0..50 {
            let mut block = sine(440.0, 0.01, 480, 48_000.0); // very quiet
            agc.process(&mut block);
            last_rms = rms_of(&block);
        }
        // Started ~70x below target; after many blocks it should have
        // climbed substantially closer (not necessarily exact — release
        // is deliberately gentle).
        assert!(last_rms > 0.05, "expected meaningful boost, got rms={last_rms}");
    }

    #[test]
    fn attenuates_loud_signal_toward_target() {
        let mut agc = AutomaticGainControl::new(0.2, 30.0);
        let mut last_rms = 1.0;
        for _ in 0..20 {
            let mut block = sine(440.0, 0.9, 480, 48_000.0); // loud
            agc.process(&mut block);
            last_rms = rms_of(&block);
        }
        assert!(last_rms < 0.5, "expected attenuation, got rms={last_rms}");
    }

    #[test]
    fn holds_gain_during_silence() {
        let mut agc = AutomaticGainControl::new(0.2, 30.0);
        agc.gain = 3.0; // pretend it had settled on some boost
        let mut silence = vec![0.0f32; 480];
        agc.process(&mut silence);
        assert_eq!(agc.gain(), 3.0, "silence must not move gain toward the ceiling");
        assert!(silence.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn never_exceeds_configured_gain_bounds() {
        let mut agc = AutomaticGainControl::new(0.5, 6.0); // +/-6 dB only
        for _ in 0..100 {
            let mut block = sine(440.0, 0.001, 480, 48_000.0);
            agc.process(&mut block);
        }
        assert!(agc.gain() <= db_to_linear(6.0) + 1e-3);
        assert!(agc.gain() >= db_to_linear(-6.0) - 1e-3);
    }
}
