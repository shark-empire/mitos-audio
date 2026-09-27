//! Bluetooth audio profiles, mapped onto `crate::profiles`' catalog.
//!
//! mitos-audio already has a profile concept (`stereo`, `bluetooth-music`,
//! `bluetooth-headset`, …) that predates this module — see
//! `crate::profiles`. A Bluetooth *audio* profile (A2DP for music, HFP/HSP
//! for calls) is really just telling mitos-audio which of its own
//! profiles applies, so [`BluetoothProfile::mitos_profile_id`] is the only
//! thing this type needs to do.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BluetoothProfile {
    /// Advanced Audio Distribution Profile — high-quality one-way music streaming.
    A2dp,
    /// Hands-Free Profile — two-way, call-quality, enables the microphone.
    Hfp,
    /// Headset Profile — HFP's older, simpler predecessor; treated the same here.
    Hsp,
}

impl BluetoothProfile {
    /// The `crate::profiles` catalog id this Bluetooth profile activates.
    pub fn mitos_profile_id(self) -> &'static str {
        match self {
            BluetoothProfile::A2dp => "bluetooth-music",
            BluetoothProfile::Hfp | BluetoothProfile::Hsp => "bluetooth-headset",
        }
    }

    /// Whether this profile carries a microphone path (relevant to
    /// `crate::policy`'s mic access check once a device using it is the
    /// default input).
    pub fn has_microphone(self) -> bool {
        matches!(self, BluetoothProfile::Hfp | BluetoothProfile::Hsp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles;

    #[test]
    fn every_bluetooth_profile_maps_to_a_known_mitos_profile() {
        for p in [BluetoothProfile::A2dp, BluetoothProfile::Hfp, BluetoothProfile::Hsp] {
            assert!(
                profiles::is_known(p.mitos_profile_id()),
                "{:?} maps to an unknown profile id",
                p
            );
        }
    }

    #[test]
    fn only_call_profiles_carry_a_microphone() {
        assert!(!BluetoothProfile::A2dp.has_microphone());
        assert!(BluetoothProfile::Hfp.has_microphone());
        assert!(BluetoothProfile::Hsp.has_microphone());
    }
}
