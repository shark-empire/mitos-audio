//! Linear gain from a dB value — the actual math behind the `mic_gain`
//! (dB, clamped ±30) the control plane has tracked since `SetMicrophoneGain`
//! landed. No caller applies it to samples yet (there is no capture data
//! plane to apply it *to* — see `crate::microphone` module docs) but the
//! function is exercised directly by its own tests below.

/// Convert a dB value to a linear amplitude multiplier (`10^(db/20)`).
pub fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Apply `gain_db` to every sample in `buf`, in place.
pub fn apply_gain_db(buf: &mut [f32], gain_db: f32) {
    let gain = db_to_linear(gain_db);
    if (gain - 1.0).abs() < f32::EPSILON {
        return; // 0 dB — skip the multiply
    }
    for s in buf.iter_mut() {
        *s *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_db_is_unity() {
        assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn plus_twenty_db_is_10x() {
        assert!((db_to_linear(20.0) - 10.0).abs() < 1e-3);
    }

    #[test]
    fn minus_twenty_db_is_tenth() {
        assert!((db_to_linear(-20.0) - 0.1).abs() < 1e-4);
    }

    #[test]
    fn apply_scales_buffer() {
        let mut buf = [0.1f32, -0.2, 0.3];
        apply_gain_db(&mut buf, 20.0);
        assert!((buf[0] - 1.0).abs() < 1e-3);
        assert!((buf[1] + 2.0).abs() < 1e-3);
        assert!((buf[2] - 3.0).abs() < 1e-3);
    }

    #[test]
    fn zero_db_leaves_buffer_untouched() {
        let original = [0.1f32, -0.2, 0.3];
        let mut buf = original;
        apply_gain_db(&mut buf, 0.0);
        assert_eq!(buf, original);
    }
}
