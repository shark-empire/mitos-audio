//! Microphone signal processing.
//!
//! `AudioManager` has tracked mic mute and gain since early on; this module
//! adds the roadmap's other three: automatic gain control
//! ([`automatic_gain`]), noise suppression ([`noise_suppression`]), and
//! echo cancellation ([`echo_cancellation`]) — real, tested DSP, each in
//! its own file, combined here into [`MicrophoneProcessor`].
//!
//! **What this does *not* do yet: touch live audio.** mitos-audio's data
//! plane only carries playback right now — there is no capture path
//! (mic → daemon → app) for this to sit in. That's an existing, documented
//! limit (see `docs/audio-plane.md`'s "Limits" section), not a new one this
//! module introduces. What *is* wired up now, end to end: the control-plane
//! toggles (`GetMicrophone`, `SetMicrophoneProcessing`), persistence, the
//! CLI, and this processing pipeline itself — fully implemented and unit
//! tested against synthetic signals. When a capture path lands, it calls
//! [`MicrophoneProcessor::process`] once per captured block; nothing here
//! needs to change to support that.
//!
//! There is deliberately no `mute.rs`: muting is "return silence," which
//! doesn't need an algorithm of its own — see `process` below.

pub mod automatic_gain;
pub mod echo_cancellation;
pub mod gain;
pub mod noise_suppression;

use automatic_gain::AutomaticGainControl;
use echo_cancellation::EchoCanceller;
use noise_suppression::NoiseGate;

/// One application's microphone processing pipeline. Each stage is `None`
/// when disabled, so a fully-disabled processor costs nothing beyond the
/// (possible) manual gain multiply.
pub struct MicrophoneProcessor {
    pub muted: bool,
    /// Manual gain, applied only when `agc` is disabled (the two are
    /// alternatives, not stacked — see `process`).
    pub gain_db: f32,
    agc: Option<AutomaticGainControl>,
    noise_gate: Option<NoiseGate>,
    echo_canceller: Option<EchoCanceller>,
}

impl MicrophoneProcessor {
    pub fn new() -> Self {
        Self { muted: false, gain_db: 0.0, agc: None, noise_gate: None, echo_canceller: None }
    }

    pub fn set_agc(&mut self, enabled: bool) {
        self.agc = if enabled { Some(AutomaticGainControl::for_voice()) } else { None };
    }

    pub fn set_noise_suppression(&mut self, enabled: bool) {
        self.noise_gate = if enabled { Some(NoiseGate::new()) } else { None };
    }

    pub fn set_echo_cancellation(&mut self, enabled: bool) {
        self.echo_canceller = if enabled { Some(EchoCanceller::with_defaults()) } else { None };
    }

    pub fn agc_enabled(&self) -> bool {
        self.agc.is_some()
    }

    pub fn noise_suppression_enabled(&self) -> bool {
        self.noise_gate.is_some()
    }

    pub fn echo_cancellation_enabled(&self) -> bool {
        self.echo_canceller.is_some()
    }

    /// Run the enabled stages, in order: echo cancellation first (it needs
    /// the far-end `reference` before anything else reshapes the signal),
    /// then the noise gate, then either AGC or manual `gain_db` — whichever
    /// is active; they are alternatives, since both fighting to set the
    /// level would just make the result harder to predict.
    ///
    /// A muted mic returns silence without running any stage (there's
    /// nothing to gain-control, gate, or de-echo in silence, and skipping
    /// them avoids the AGC's gain estimate drifting toward its ceiling
    /// while chasing a signal that no one will hear).
    pub fn process(&mut self, mic: &mut [f32], reference: Option<&[f32]>) {
        if self.muted {
            for s in mic.iter_mut() {
                *s = 0.0;
            }
            return;
        }
        if let (Some(aec), Some(reference)) = (self.echo_canceller.as_mut(), reference) {
            if reference.len() == mic.len() {
                aec.process(mic, reference);
            }
        }
        if let Some(gate) = self.noise_gate.as_mut() {
            gate.process(mic);
        }
        if let Some(agc) = self.agc.as_mut() {
            agc.process(mic);
        } else {
            gain::apply_gain_db(mic, self.gain_db);
        }
    }
}

impl Default for MicrophoneProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn muted_produces_silence_regardless_of_other_settings() {
        let mut proc = MicrophoneProcessor::new();
        proc.muted = true;
        proc.gain_db = 20.0;
        proc.set_agc(true);
        let mut buf = [0.5f32, -0.5, 0.3];
        proc.process(&mut buf, None);
        assert_eq!(buf, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn everything_disabled_is_a_no_op() {
        let mut proc = MicrophoneProcessor::new();
        let original = [0.1f32, -0.2, 0.3, -0.05];
        let mut buf = original;
        proc.process(&mut buf, None);
        assert_eq!(buf, original);
    }

    #[test]
    fn manual_gain_applies_only_without_agc() {
        let mut proc = MicrophoneProcessor::new();
        proc.gain_db = 20.0; // 10x
        let mut buf = [0.05f32; 8];
        proc.process(&mut buf, None);
        assert!((buf[0] - 0.5).abs() < 1e-3, "expected manual +20dB gain, got {}", buf[0]);
    }

    #[test]
    fn agc_bypasses_manual_gain() {
        let mut proc = MicrophoneProcessor::new();
        proc.gain_db = 20.0; // would be 10x if applied — must not be, with AGC on
        proc.set_agc(true);
        let mut buf = vec![0.3f32; 480]; // already loud relative to AGC's voice target
        let input_rms = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();
        proc.process(&mut buf, None);
        let output_rms = (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt();
        assert!(
            output_rms < input_rms * 2.0,
            "AGC should govern the level, not the bypassed manual +20dB gain: in={input_rms} out={output_rms}"
        );
    }

    #[test]
    fn noise_suppression_and_echo_cancellation_toggle_independently() {
        let mut proc = MicrophoneProcessor::new();
        assert!(!proc.noise_suppression_enabled());
        assert!(!proc.echo_cancellation_enabled());
        proc.set_noise_suppression(true);
        assert!(proc.noise_suppression_enabled());
        assert!(!proc.echo_cancellation_enabled());
        proc.set_echo_cancellation(true);
        assert!(proc.echo_cancellation_enabled());
        proc.set_noise_suppression(false);
        assert!(!proc.noise_suppression_enabled());
        assert!(proc.echo_cancellation_enabled());
    }
}
