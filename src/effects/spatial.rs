//! Stereo widening via mid/side scaling.
//!
//! This is the honest scope of "spatial audio" mitos-audio implements
//! today: split a stereo signal into mid (`(L+R)/2`, the mono-compatible
//! part) and side (`(L-R)/2`, the stereo difference), scale the side
//! component, and recombine. It changes perceived stereo width; it is not
//! HRTF-based 3D positioning or room correction (both listed as "on the
//! roadmap" in `docs/audio-model.md`) — those need a head model or a
//! measured room response respectively, neither of which mitos-audio has
//! input for yet. Swapping in either later is a drop-in replacement at the
//! same point in [`super::EffectsChain`].

pub struct SpatialWidener {
    /// 1.0 = unchanged, >1.0 = wider, <1.0 = narrower (toward mono at 0.0).
    /// Values much above ~2.0 start to sound unnatural and can hurt mono
    /// compatibility; this is not clamped here so callers can experiment,
    /// but `crate::effects::presets` sticks to modest values.
    pub width: f32,
}

impl SpatialWidener {
    pub fn new(width: f32) -> Self {
        Self { width }
    }

    /// Widen interleaved stereo audio in place. Does nothing at `width ==
    /// 1.0` (the common case: most presets don't touch width).
    pub fn process_stereo(&self, buf: &mut [f32]) {
        if (self.width - 1.0).abs() < 1e-6 {
            return;
        }
        for pair in buf.chunks_exact_mut(2) {
            let mid = (pair[0] + pair[1]) * 0.5;
            let side = (pair[0] - pair[1]) * 0.5 * self.width;
            pair[0] = mid + side;
            pair[1] = mid - side;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_width_is_a_no_op() {
        let widener = SpatialWidener::new(1.0);
        let original = [0.5f32, 0.1, -0.2, 0.3];
        let mut buf = original;
        widener.process_stereo(&mut buf);
        assert_eq!(buf, original);
    }

    #[test]
    fn doubling_width_doubles_the_stereo_difference_and_preserves_mid() {
        let widener = SpatialWidener::new(2.0);
        let mut buf = [0.5f32, 0.1]; // L=0.5, R=0.1 -> mid=0.3, side=0.2
        widener.process_stereo(&mut buf);
        assert!((buf[0] - 0.7).abs() < 1e-6, "L' should be 0.7, got {}", buf[0]);
        assert!((buf[1] - (-0.1)).abs() < 1e-6, "R' should be -0.1, got {}", buf[1]);
        let new_mid = (buf[0] + buf[1]) * 0.5;
        assert!((new_mid - 0.3).abs() < 1e-6, "mid (mono-compatible sum) must be preserved");
    }

    #[test]
    fn zero_width_collapses_to_mono() {
        let widener = SpatialWidener::new(0.0);
        let mut buf = [0.8f32, 0.2];
        widener.process_stereo(&mut buf);
        assert!((buf[0] - 0.5).abs() < 1e-6);
        assert!((buf[1] - 0.5).abs() < 1e-6);
        assert!((buf[0] - buf[1]).abs() < 1e-6, "L and R should be identical at width 0");
    }
}
