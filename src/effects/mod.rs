//! Per-device audio effects: equalizer, compressor, a basic stereo
//! widener, and a limiter — the "optional advanced layer" from the
//! original design notes. Unlike `crate::microphone` (which has no data
//! path to run on yet), this *is* wired into live audio: `engine::sink`'s
//! mixer applies a device's [`EffectsChain`] to every mixed period before
//! it reaches hardware. See `docs/audio-model.md` for the full data-flow
//! picture and `docs/security.md`/`docs/troubleshooting.md` for what to
//! expect performance-wise.
//!
//! There is deliberately no `surround.rs`: true virtual surround (or room
//! correction) needs an HRTF or a measured room response, neither of
//! which mitos-audio has an input for today. [`spatial::SpatialWidener`]
//! is the honest, basic version of "spatial audio" this module ships —
//! see its own docs for the boundary.

pub mod compressor;
pub mod equalizer;
pub mod limiter;
pub mod presets;
pub mod spatial;

use compressor::Compressor;
use equalizer::Equalizer;
use limiter::Limiter;
use presets::Preset;
use spatial::SpatialWidener;

/// One device's effects pipeline. Created once per sink (see
/// `engine::sink::SinkManager`) at the sample rate/channel count that sink
/// actually runs at, since biquad coefficients are sample-rate-dependent.
pub struct EffectsChain {
    pub enabled: bool,
    sample_rate: f32,
    channels: usize,
    preset: Preset,
    equalizer: Equalizer,
    compressor: Option<Compressor>,
    widener: SpatialWidener,
    limiter: Limiter,
}

impl EffectsChain {
    pub fn new(sample_rate: f32, channels: usize) -> Self {
        Self {
            enabled: false,
            sample_rate,
            channels,
            preset: Preset::Flat,
            equalizer: Equalizer::new(sample_rate, channels),
            compressor: None,
            widener: SpatialWidener::new(1.0),
            limiter: Limiter::new(-1.0, 50.0, sample_rate), // -1 dBFS ceiling
        }
    }

    pub fn preset(&self) -> Preset {
        self.preset
    }

    pub fn bands(&self) -> [f32; 10] {
        self.equalizer.bands()
    }

    pub fn width(&self) -> f32 {
        self.widener.width
    }

    /// Apply a named preset: sets the EQ curve and turns the compressor
    /// on/off per `Preset::compressor`. Does not change `enabled` — a
    /// preset can be dialed in before switching effects on.
    pub fn apply_preset(&mut self, preset: Preset) {
        self.preset = preset;
        self.equalizer.set_bands(preset.bands());
        self.compressor = preset
            .compressor()
            .map(|(threshold_db, ratio)| Compressor::new(threshold_db, ratio, 10.0, 150.0, self.sample_rate));
    }

    /// Set custom band gains directly; marks the preset as `Custom`.
    pub fn set_bands(&mut self, bands: [f32; 10]) {
        self.preset = Preset::Custom;
        self.equalizer.set_bands(bands);
    }

    pub fn set_width(&mut self, width: f32) {
        self.widener.width = width;
    }

    /// Process one mixed period, interleaved, in place. A no-op beyond the
    /// borrow itself when `enabled` is false.
    pub fn process(&mut self, buf: &mut [f32]) {
        if !self.enabled {
            return;
        }
        self.equalizer.process(buf);
        if let Some(c) = self.compressor.as_mut() {
            c.process(buf);
        }
        if self.channels == 2 {
            self.widener.process_stereo(buf);
        }
        // Always last, whenever effects are enabled at all: EQ boosts and
        // the widener's recombination can each push a sample over 0 dBFS
        // even when the untouched source never would have.
        self.limiter.process(buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn sine(freq: f32, amplitude: f32, n: usize, sample_rate: f32) -> Vec<f32> {
        (0..n).map(|i| amplitude * (TAU * freq * i as f32 / sample_rate).sin()).collect()
    }

    #[test]
    fn disabled_chain_is_a_no_op() {
        let mut chain = EffectsChain::new(48_000.0, 2);
        chain.apply_preset(Preset::Music); // configured, but not enabled
        let original = sine(1000.0, 0.5, 100, 48_000.0);
        let mut buf = original.clone();
        chain.process(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn enabled_flat_preset_still_limits_extreme_input() {
        let mut chain = EffectsChain::new(48_000.0, 1);
        chain.enabled = true;
        chain.apply_preset(Preset::Flat); // no EQ/compressor, but limiter is always active when enabled
        let mut buf = vec![1.0f32; 200];
        chain.process(&mut buf);
        let ceiling = 10f32.powf(-1.0 / 20.0);
        assert!(buf.iter().all(|s| s.abs() <= ceiling + 1e-6));
    }

    #[test]
    fn boosted_preset_never_exceeds_the_limiter_ceiling() {
        // Music boosts bass/treble; feed it full-scale audio across a
        // spread of frequencies and confirm the chain's own limiter still
        // catches everything — the end-to-end safety-net property that
        // matters more than any individual stage's behavior.
        let mut chain = EffectsChain::new(48_000.0, 2);
        chain.enabled = true;
        chain.apply_preset(Preset::Music);

        let mut buf = Vec::new();
        for freq in [60.0, 250.0, 1000.0, 4000.0, 12000.0] {
            for s in sine(freq, 0.98, 2000, 48_000.0) {
                buf.push(s); // left
                buf.push(s); // right
            }
        }
        chain.process(&mut buf);
        let ceiling = 10f32.powf(-1.0 / 20.0);
        assert!(
            buf.iter().all(|s| s.abs() <= ceiling + 1e-3),
            "no sample may exceed the limiter ceiling regardless of EQ boost"
        );
    }

    #[test]
    fn set_bands_marks_preset_custom() {
        let mut chain = EffectsChain::new(48_000.0, 2);
        chain.apply_preset(Preset::Movie);
        assert_eq!(chain.preset(), Preset::Movie);
        chain.set_bands([1.0; 10]);
        assert_eq!(chain.preset(), Preset::Custom);
        assert_eq!(chain.bands(), [1.0; 10]);
    }
}
