use std::sync::{Mutex, MutexGuard};

use super::backend::OutputDevice;
use crate::engine::MIX_CHANNELS;
use super::backend::AudioBackend;
use crate::devices::device::{Bus, Device, DeviceKind, Direction};
use crate::errors::AudioError;
use crate::monitoring::LevelFrame;

/// Playback black hole for demo mode. The mixer thread still runs and
/// meters the *mixed* signal, so GUIs see real levels; samples are
/// discarded. This makes the whole audio plane testable without hardware.
pub struct DemoOutput;

impl OutputDevice for DemoOutput {
    fn write(&mut self, frames: &[f32]) -> Result<usize, AudioError> {
        Ok(frames.len() / MIX_CHANNELS)
    }

    fn recover(&mut self) -> Result<(), AudioError> {
        Ok(())
    }
}
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
    
    fn open_output(&self, _device: &Device) -> Result<Box<dyn OutputDevice>, AudioError> {
        Ok(Box::new(DemoOutput))
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
    
       fn levels(&self) -> Result<LevelFrame, AudioError> {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let guard = guard(&self.devices)?;
        let (volume, muted) = guard
            .iter()
            .find(|d| d.id == "speakers")
            .map(|d| (d.volume, d.muted))
            .unwrap_or((0, false));

        let output_level = if muted {
            0.0
        } else {
            let wave = 0.5 + 0.5 * (t * std::f64::consts::TAU * 1.1).sin();
            (0.30 + 0.55 * wave) as f32 * volume as f32 / 100.0
        };
        let input_level = (0.12 + 0.10 * (t * std::f64::consts::TAU * 0.7).sin()) as f32;
        let peak = (output_level * 1.05).min(1.0);

        Ok(LevelFrame {
            output_level,
            input_level,
            peak,
            clipping: peak >= 0.99,
        })
    }
}