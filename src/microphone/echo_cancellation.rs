//! Acoustic echo cancellation via NLMS (normalized least-mean-squares).
//!
//! Standard technique, not a novel one: model the path from speaker to
//! microphone as an unknown FIR filter, adapt an estimate of it sample by
//! sample from the far-end (speaker/reference) signal, and subtract the
//! estimated echo from the microphone signal. What "far-end reference"
//! means here is the same buffer `engine::sink` just mixed for that
//! device — see the module docs on [`crate::microphone`] for why applying
//! this to live audio needs the capture data plane that doesn't exist yet.
//!
//! NLMS specifically (over plain LMS) normalizes the adaptation step by the
//! reference signal's energy, which keeps it stable across the wide volume
//! swings real playback audio has (LMS with a fixed step size that's stable
//! for quiet audio will diverge on loud audio, and vice versa).
//!
//! Cost is O(taps) per sample — at 128 taps and 48 kHz that is ~6.1M
//! multiply-adds/second per stream, comfortably real-time on any target
//! this daemon runs on, but it is *not* free; do not raise `taps` far
//! beyond what the room's actual echo tail needs (128 taps ≈ 2.7 ms of
//! echo path at 48 kHz — enough for a phone/laptop speaker-to-mic
//! distance, not for a large echoey room).

use std::collections::VecDeque;

pub struct EchoCanceller {
    weights: Vec<f32>,
    /// Most recent reference sample at the front.
    history: VecDeque<f32>,
    /// NLMS step size, `0 < mu < 2` for stability; 0.5 is a conservative
    /// middle ground between fast convergence and stability margin.
    mu: f32,
    /// Regularization added to reference energy so the normalization
    /// doesn't divide by ~0 during silence.
    eps: f32,
}

impl EchoCanceller {
    pub fn new(taps: usize) -> Self {
        Self {
            weights: vec![0.0; taps],
            history: VecDeque::from(vec![0.0; taps]),
            mu: 0.5,
            eps: 1e-6,
        }
    }

    /// 128 taps (~2.7 ms at 48 kHz) — see the module docs for why that's
    /// the default rather than something larger.
    pub fn with_defaults() -> Self {
        Self::new(128)
    }

    /// Cancel the portion of `mic` correlated with `reference` (the
    /// far-end/speaker signal that could be leaking into the microphone),
    /// sample by sample, adapting the internal filter as it goes and
    /// overwriting `mic` with the residual (the cleaned signal). Panics if
    /// the two buffers differ in length.
    pub fn process(&mut self, mic: &mut [f32], reference: &[f32]) {
        assert_eq!(mic.len(), reference.len(), "mic and reference buffers must match in length");
        for i in 0..mic.len() {
            self.history.pop_back();
            self.history.push_front(reference[i]);

            let estimate: f32 =
                self.weights.iter().zip(self.history.iter()).map(|(w, x)| w * x).sum();
            let error = mic[i] - estimate;

            let energy: f32 = self.history.iter().map(|x| x * x).sum::<f32>() + self.eps;
            let step = self.mu * error / energy;
            for (w, x) in self.weights.iter_mut().zip(self.history.iter()) {
                *w += step * x;
            }

            mic[i] = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn energy(buf: &[f32]) -> f32 {
        buf.iter().map(|s| s * s).sum()
    }

    #[test]
    fn converges_on_a_pure_delay_echo_path() {
        // The "room" is just a 5-sample delay: mic[n] = reference[n-5].
        let reference = pseudo_noise(0.3, 4000, 1);
        let delay = 5;
        let mut mic = vec![0.0f32; reference.len()];
        for n in delay..reference.len() {
            mic[n] = reference[n - delay];
        }

        let mut aec = EchoCanceller::new(32);
        let mut residual = mic.clone();
        aec.process(&mut residual, &reference);

        let early = energy(&residual[0..500]);
        let late = energy(&residual[residual.len() - 500..]);
        assert!(
            late < early * 0.1,
            "expected the residual echo to shrink well after convergence: early={early} late={late}"
        );
    }

    #[test]
    fn converges_on_an_undelayed_echo_path() {
        let reference = pseudo_noise(0.4, 4000, 2);
        let mic = reference.clone(); // echo path = identity, no near-end speech

        let mut aec = EchoCanceller::new(16);
        let mut residual = mic.clone();
        aec.process(&mut residual, &reference);

        let early = energy(&residual[0..300]);
        let late = energy(&residual[residual.len() - 300..]);
        assert!(
            late < early * 0.05,
            "expected near-total cancellation once converged: early={early} late={late}"
        );
    }

    #[test]
    fn does_not_destroy_uncorrelated_signal() {
        // Reference and mic share no relationship at all — there's no echo
        // to remove, so the canceller should leave most of the mic energy
        // alone rather than adapting itself into silence.
        let reference = pseudo_noise(0.3, 2000, 3);
        let mic_original = pseudo_noise(0.3, 2000, 99);

        let mut aec = EchoCanceller::new(32);
        let mut residual = mic_original.clone();
        aec.process(&mut residual, &reference);

        let original_energy = energy(&mic_original[mic_original.len() - 300..]);
        let late = energy(&residual[residual.len() - 300..]);
        assert!(
            late > original_energy * 0.5,
            "uncorrelated signal should mostly survive: original={original_energy} late={late}"
        );
    }
}
