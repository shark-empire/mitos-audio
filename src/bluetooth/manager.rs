//! The mitos-audio side of Bluetooth coordination.
//!
//! [`BluetoothCoordinator`] is the contract: whatever eventually watches
//! BlueZ for connect/disconnect events and profile/codec capabilities
//! calls these two methods. It is a plain trait rather than a fixed wire
//! format because *how* that future service reaches this one — a second
//! Unix socket, an in-process call if it turns out to live in the same
//! daemon, D-Bus directly — is exactly the kind of decision that should
//! stay open until `mitos-bluetooth` actually exists. [`BluetoothManager`]
//! is the trait's only implementation today, and is also usable directly
//! (e.g. from `backend::demo`, which does exactly that).

use std::collections::HashMap;
use std::sync::Mutex;

use super::codec::{self, Codec};
use super::device::BluetoothDeviceInfo;

pub trait BluetoothCoordinator: Send + Sync {
    fn on_device_connected(&self, info: BluetoothDeviceInfo);
    fn on_device_disconnected(&self, mac: &str);
}

#[derive(Default)]
pub struct BluetoothManager {
    connected: Mutex<HashMap<String, BluetoothDeviceInfo>>,
    active_codec: Mutex<HashMap<String, Codec>>,
}

impl BluetoothManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a connected device and negotiate a codec against
    /// `local_codecs` (what mitos-audio would prefer to use). Returns the
    /// negotiated codec, if any.
    pub fn note_connected(&self, info: BluetoothDeviceInfo, local_codecs: &[Codec]) -> Option<Codec> {
        let chosen = codec::negotiate(local_codecs, &info.codecs);
        if let Some(c) = chosen {
            self.active_codec.lock().unwrap_or_else(|e| e.into_inner()).insert(info.mac.clone(), c);
        }
        self.connected.lock().unwrap_or_else(|e| e.into_inner()).insert(info.mac.clone(), info);
        chosen
    }

    pub fn note_disconnected(&self, mac: &str) {
        self.connected.lock().unwrap_or_else(|e| e.into_inner()).remove(mac);
        self.active_codec.lock().unwrap_or_else(|e| e.into_inner()).remove(mac);
    }

    pub fn codec_for(&self, mac: &str) -> Option<Codec> {
        self.active_codec.lock().unwrap_or_else(|e| e.into_inner()).get(mac).copied()
    }

    pub fn device(&self, mac: &str) -> Option<BluetoothDeviceInfo> {
        self.connected.lock().unwrap_or_else(|e| e.into_inner()).get(mac).cloned()
    }

    pub fn connected_devices(&self) -> Vec<BluetoothDeviceInfo> {
        self.connected.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }
}

impl BluetoothCoordinator for BluetoothManager {
    fn on_device_connected(&self, info: BluetoothDeviceInfo) {
        // A2DP-leaning default preference; a device offering only HFP/HSP
        // still negotiates fine since `negotiate` just intersects the two
        // lists (mSBC isn't in this list, but that's fine: this manager
        // tracks the *A2DP* codec, and HFP-only devices are picked up via
        // `BluetoothProfile::has_microphone` instead of a negotiated codec).
        self.note_connected(info, codec::PREFERRED_A2DP_CODECS);
    }

    fn on_device_disconnected(&self, mac: &str) {
        self.note_disconnected(mac);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluetooth::profile::BluetoothProfile;

    fn sample_device() -> BluetoothDeviceInfo {
        let mut info = BluetoothDeviceInfo::new("AA:BB:CC:DD:EE:FF", "Test Headphones");
        info.profiles = vec![BluetoothProfile::A2dp, BluetoothProfile::Hfp];
        info.codecs = vec![Codec::Sbc, Codec::AacLc];
        info
    }

    #[test]
    fn connect_negotiates_and_disconnect_clears() {
        let mgr = BluetoothManager::new();
        let chosen = mgr.note_connected(sample_device(), codec::PREFERRED_A2DP_CODECS);
        assert_eq!(chosen, Some(Codec::AacLc));
        assert_eq!(mgr.codec_for("AA:BB:CC:DD:EE:FF"), Some(Codec::AacLc));
        assert_eq!(mgr.connected_devices().len(), 1);

        mgr.note_disconnected("AA:BB:CC:DD:EE:FF");
        assert_eq!(mgr.codec_for("AA:BB:CC:DD:EE:FF"), None);
        assert!(mgr.connected_devices().is_empty());
    }

    #[test]
    fn coordinator_trait_object_works() {
        let mgr: Box<dyn BluetoothCoordinator> = Box::new(BluetoothManager::new());
        mgr.on_device_connected(sample_device());
        mgr.on_device_disconnected("AA:BB:CC:DD:EE:FF");
        // Reaching here without a panic is the point: this is the exact
        // interface shape a future mitos-bluetooth integration will call.
    }

    #[test]
    fn unknown_device_has_no_codec() {
        let mgr = BluetoothManager::new();
        assert_eq!(mgr.codec_for("00:00:00:00:00:00"), None);
    }
}
