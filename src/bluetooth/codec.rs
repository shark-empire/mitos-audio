//! Bluetooth audio codecs, and picking one when both ends of a link
//! support more than the mandatory baseline.
//!
//! mitos-audio does not speak A2DP/HFP itself (see the `crate::bluetooth`
//! module docs) — a real `mitos-bluetooth` service does the actual codec
//! negotiation over the air. What belongs here, and does not belong to
//! that service, is *policy*: given a set of codecs mitos-audio would
//! prefer to use and a set the remote device advertises, which one should
//! win? That's a mitos-audio call because it's the same kind of decision
//! as everything else in `crate::profiles` and `crate::effects` — audio
//! quality policy, not link-layer plumbing.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    /// Mandatory baseline for A2DP — every device supports it.
    Sbc,
    AacLc,
    AptX,
    AptxHd,
    Ldac,
    /// Wideband speech codec for HFP (as opposed to narrowband CVSD).
    Msbc,
}

impl Codec {
    /// Human-readable label, e.g. for a GUI's device detail panel.
    pub fn label(self) -> &'static str {
        match self {
            Codec::Sbc => "SBC",
            Codec::AacLc => "AAC",
            Codec::AptX => "aptX",
            Codec::AptxHd => "aptX HD",
            Codec::Ldac => "LDAC",
            Codec::Msbc => "mSBC",
        }
    }

    /// Relative quality/bitrate rank used to pick a winner when several
    /// codecs are mutually supported — higher is more preferred. Not a
    /// physical unit, just an ordering; see `negotiate`.
    fn preference_rank(self) -> u8 {
        match self {
            Codec::Sbc => 0,
            Codec::AptX => 1,
            Codec::AacLc => 2,
            Codec::AptxHd => 3,
            Codec::Ldac => 4,
            Codec::Msbc => 0, // HFP has no higher-quality alternative to rank against here
        }
    }
}

/// mitos-audio's own preference order when nothing more specific is known
/// (used as the "local" side of `negotiate` for a freshly-seen device).
pub const PREFERRED_A2DP_CODECS: &[Codec] = &[Codec::Ldac, Codec::AptxHd, Codec::AacLc, Codec::AptX, Codec::Sbc];
pub const PREFERRED_HFP_CODECS: &[Codec] = &[Codec::Msbc];

/// Pick the highest-preference codec present in both `local` and `remote`.
/// `None` if they share nothing — which should not happen in practice
/// since SBC (A2DP) / CVSD-via-mSBC-fallback (HFP) are mandatory, but a
/// coordination layer should not assume the far end behaves.
pub fn negotiate(local: &[Codec], remote: &[Codec]) -> Option<Codec> {
    local
        .iter()
        .filter(|c| remote.contains(c))
        .max_by_key(|c| c.preference_rank())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_highest_ranked_mutual_codec() {
        let local = PREFERRED_A2DP_CODECS;
        let remote = [Codec::Sbc, Codec::AacLc, Codec::AptX];
        assert_eq!(negotiate(local, &remote), Some(Codec::AacLc));
    }

    #[test]
    fn falls_back_to_only_shared_codec() {
        let local = PREFERRED_A2DP_CODECS;
        let remote = [Codec::Sbc];
        assert_eq!(negotiate(local, &remote), Some(Codec::Sbc));
    }

    #[test]
    fn no_overlap_is_none() {
        let local = [Codec::Ldac];
        let remote = [Codec::Sbc];
        assert_eq!(negotiate(&local, &remote), None);
    }

    #[test]
    fn prefers_ldac_when_both_sides_support_everything() {
        let remote = [Codec::Sbc, Codec::AptX, Codec::AacLc, Codec::AptxHd, Codec::Ldac];
        assert_eq!(negotiate(PREFERRED_A2DP_CODECS, &remote), Some(Codec::Ldac));
    }
}
