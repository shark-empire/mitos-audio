//! What a Bluetooth coordination source (today: nothing; eventually:
//! `mitos-bluetooth`) tells mitos-audio about a connected device.

use super::codec::Codec;
use super::profile::BluetoothProfile;

#[derive(Debug, Clone, PartialEq)]
pub struct BluetoothDeviceInfo {
    /// The device's Bluetooth address, used as its stable identity —
    /// display names are not unique and can change.
    pub mac: String,
    pub name: String,
    /// Every profile this device advertises (a headset commonly offers
    /// both A2DP and HFP; mitos-audio picks between them based on how it's
    /// being used — see `crate::routing`).
    pub profiles: Vec<BluetoothProfile>,
    /// Every codec the device advertises support for, across all its
    /// profiles combined — `codec::negotiate` narrows this down.
    pub codecs: Vec<Codec>,
    pub battery_percent: Option<u8>,
}

impl BluetoothDeviceInfo {
    pub fn new(mac: impl Into<String>, name: impl Into<String>) -> Self {
        Self { mac: mac.into(), name: name.into(), profiles: Vec::new(), codecs: Vec::new(), battery_percent: None }
    }
}
