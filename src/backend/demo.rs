use std::sync::{Mutex, MutexGuard};

use super::backend::AudioBackend;
use crate::devices::device::{Bus, Device, DeviceKind, Direction};
use crate::errors::AudioError;

/// In-memory "hardware" for development, CI, and non-Linux builds.
///
/// Behaves like real hardware: volume/mute writes persist and are
/// visible to later scans, so the daemon can be exercised end-to-end
/// without a sound card.
pub struct DemoBackend {
    devices: Mutex<Vec<Device>>,
}

fn guard<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>, AudioError> {
    m.lock().map_err(|e| AudioError::Backend(format!("demo backend lock poisoned: {e}")))
}

impl DemoBackend {
    pub fn new() -> Self {
        let mut speakers =
            Device::new("speakers", "Internal Speakers", DeviceKind::Speakers, Direction::Output, Bus::Internal);
        speakers.description = "Built-in speakers (demo hardware)".into();

        let mut headphones =
            Device::new("headphones", "Analog Headphones", DeviceKind::Headphones, Direction::Output, Bus::Internal);
        headphones.description = "3.5mm jack (demo hardware)".into();

        let mut hdmi =
            Device::new("hdmi0", "HDMI 0", DeviceKind::Hdmi, Direction::Output, Bus::Hdmi);
        hdmi.description = "HDMI audio out (demo hardware)".into();
        hdmi.profiles = vec!["hdmi-stereo".into(), "stereo".into()];
        hdmi.active_profile = Some("hdmi-stereo".into());

        let mut usb_mic =
            Device::new("usb-mic", "USB Microphone", DeviceKind::Microphone, Direction::Input, Bus::Usb);
        usb_mic.description = "USB condenser mic (demo hardware)".into();

        let mut bt =
            Device::new("bt-headset", "Bluetooth Headset", DeviceKind::Headset, Direction::Both, Bus::Bluetooth);
        bt.description = "A2DP/HFP headset (demo hardware — real BT via mitos-bluetooth)".into();
        bt.profiles = vec!["bluetooth-music".into(), "bluetooth-headset".into()];
        bt.active_profile = Some("bluetooth-music".into());

        Self { devices: Mutex::new(vec![speakers, headphones, hdmi, usb_mic, bt]) }
    }
}

impl AudioBackend for DemoBackend {
    fn name(&self) -> &'static str { "demo" }

    fn scan(&self) -> Result<Vec<Device>, AudioError> {
        Ok(guard(&self.devices)?.clone())
    }

    fn set_volume(&self, device: &Device, volume: u32) -> Result<bool, AudioError> {
        for d in guard(&self.devices)?.iter_mut() {
            if d.id == device.id {
                d.volume = volume;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn set_mute(&self, device: &Device, mute: bool) -> Result<bool, AudioError> {
        for d in guard(&self.devices)?.iter_mut() {
            if d.id == device.id {
                d.muted = mute;
                return Ok(true);
            }
        }
        Ok(false)
    }
}