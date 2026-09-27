//! Audio profile catalog.
//!
//! A "profile" (`stereo`, `surround-5.1`, `bluetooth-headset`, …) selects how
//! a device is driven — channel layout above all. Profiles were previously
//! validated against a flat list of known names only; that let you activate
//! e.g. `surround-5.1` while the active output was two-channel headphones.
//!
//! This module is the single source of truth for which profiles exist and
//! what they require, so [`validate_for_device`] can catch that case. Each
//! [`Device`] separately advertises which of these profiles *it* supports in
//! `Device::profiles` (populated by the backend, e.g. `backend::demo` or
//! `backend::alsa`) — the two lists (system-known vs. device-supported) are
//! intentionally different: a profile can be a real, known thing that this
//! particular device just doesn't offer.

use crate::devices::device::Device;
use crate::errors::AudioError;

/// One entry in the catalog. `channels` is the layout the profile implies;
/// it is informational for now (surfaced to GUIs via `ListProfiles`) and
/// used as a sanity check when a device's own channel count is known.
#[derive(Debug, Clone, Copy)]
pub struct ProfileSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub channels: u32,
    pub description: &'static str,
}

/// Every profile mitos-audio knows about. Devices declare a subset of these
/// ids in their own `profiles` field; see `backend::demo` / `backend::alsa`.
pub const CATALOG: &[ProfileSpec] = &[
    ProfileSpec {
        id: "stereo",
        label: "Stereo",
        channels: 2,
        description: "Plain two-channel output — the default for almost every device.",
    },
    ProfileSpec {
        id: "headphones",
        label: "Headphones",
        channels: 2,
        description: "Stereo tuned for headphone listening (no speaker crossfeed assumptions).",
    },
    ProfileSpec {
        id: "headset",
        label: "Headset",
        channels: 2,
        description: "Combined headphone + microphone device, wired or Bluetooth.",
    },
    ProfileSpec {
        id: "hdmi-stereo",
        label: "HDMI Stereo",
        channels: 2,
        description: "Stereo over an HDMI/DisplayPort link.",
    },
    ProfileSpec {
        id: "surround-5.1",
        label: "Surround 5.1",
        channels: 6,
        description: "Front L/R, center, LFE, rear L/R — needs a 6-channel-capable device.",
    },
    ProfileSpec {
        id: "surround-7.1",
        label: "Surround 7.1",
        channels: 8,
        description: "5.1 plus side L/R — needs an 8-channel-capable device.",
    },
    ProfileSpec {
        id: "bluetooth-music",
        label: "Bluetooth Music",
        channels: 2,
        description: "A2DP high-quality stereo playback profile.",
    },
    ProfileSpec {
        id: "bluetooth-headset",
        label: "Bluetooth Headset",
        channels: 2,
        description: "HFP/HSP two-way call-quality profile (lower bandwidth, mic enabled).",
    },
    ProfileSpec {
        id: "usb-dac",
        label: "USB DAC",
        channels: 2,
        description: "Stereo through a dedicated USB digital-to-analog converter.",
    },
];

/// Look up a catalog entry by id.
pub fn find(id: &str) -> Option<&'static ProfileSpec> {
    CATALOG.iter().find(|p| p.id == id)
}

/// Whether `id` is a profile mitos-audio knows about at all (independent of
/// whether any particular device currently supports it).
pub fn is_known(id: &str) -> bool {
    find(id).is_some()
}

/// All known profile ids, in catalog order (what `ListProfiles` returns).
pub fn ids() -> Vec<&'static str> {
    CATALOG.iter().map(|p| p.id).collect()
}

/// Validate that `profile` is both known *and* declared as supported by
/// `device` (its `profiles` list — see `Device::new` and the backends that
/// populate it). A device with channel count lower than the profile's
/// nominal channel count is rejected even if it lists the profile, since
/// that combination cannot be a hardware mistake — it can only be stale
/// device metadata.
pub fn validate_for_device(profile: &str, device: &Device) -> Result<(), AudioError> {
    let spec = find(profile).ok_or_else(|| AudioError::InvalidProfile(profile.to_string()))?;
    if !device.profiles.iter().any(|p| p == profile) {
        return Err(AudioError::ProfileNotSupported(profile.to_string(), device.id.clone()));
    }
    if device.channels < spec.channels {
        return Err(AudioError::ProfileNotSupported(profile.to_string(), device.id.clone()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::device::{Bus, DeviceKind, Direction};

    #[test]
    fn catalog_ids_are_unique() {
        let ids = ids();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate profile id in CATALOG");
    }

    #[test]
    fn unknown_profile_rejected() {
        let device = Device::new("spk", "Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal);
        assert!(matches!(
            validate_for_device("nonexistent", &device),
            Err(AudioError::InvalidProfile(_))
        ));
    }

    #[test]
    fn known_but_unsupported_profile_rejected() {
        // Device::new() defaults to `profiles: ["stereo"]`, channels: 2.
        let device = Device::new("spk", "Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal);
        assert!(matches!(
            validate_for_device("surround-5.1", &device),
            Err(AudioError::ProfileNotSupported(_, _))
        ));
    }

    #[test]
    fn supported_profile_accepted() {
        let device = Device::new("spk", "Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal);
        assert!(validate_for_device("stereo", &device).is_ok());
    }

    #[test]
    fn declared_but_underpowered_device_rejected() {
        // A device could (incorrectly) list a profile it lacks the channels
        // for; channel count is the final check.
        let mut device = Device::new("spk", "Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal);
        device.profiles.push("surround-5.1".into());
        device.channels = 2;
        assert!(matches!(
            validate_for_device("surround-5.1", &device),
            Err(AudioError::ProfileNotSupported(_, _))
        ));
    }
}
