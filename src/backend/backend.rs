use crate::devices::device::Device;
use crate::errors::AudioError;
use crate::monitoring::LevelFrame;

/// One opened output device (the playback half of a backend). `write`
/// takes interleaved stereo f32 at the canonical mix rate (48 kHz);
/// implementations convert to the device's native format.
pub trait OutputDevice: Send {
    /// Write as many whole frames as the device accepts; returns frames written.
    fn write(&mut self, frames: &[f32]) -> Result<usize, AudioError>;

    /// Recover after underrun/suspend (ALSA `prepare`).
    fn recover(&mut self) -> Result<(), AudioError>;
}

/// Hardware-facing interface. Implementations must be `Send + Sync`;
/// they are invoked through `tokio::task::spawn_blocking`.
///
/// Contract:
/// - `scan` is the source of truth for which devices exist and their
///   current hardware volume/mute.
/// - `set_volume`/`set_mute` return `Ok(false)` when the device has no
///   such hardware control (soft-fail, not an error).
/// - `open_output` is called by the sink manager when a device has
///   streams routed to it; returned devices are written to from a
///   dedicated mixer thread.
pub trait AudioBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn scan(&self) -> Result<Vec<Device>, AudioError>;

    fn set_volume(&self, device: &Device, volume: u32) -> Result<bool, AudioError>;

    fn set_mute(&self, device: &Device, mute: bool) -> Result<bool, AudioError>;

    /// Open `device` for playback (audio plane). Default: unsupported.
    fn open_output(&self, _device: &Device) -> Result<Box<dyn OutputDevice>, AudioError> {
        Err(AudioError::Backend("backend has no playback support".into()))
    }

    /// Current level frame (input side; output is measured by the mixer).
    /// Default: zeros.
    fn levels(&self) -> Result<LevelFrame, AudioError> {
        Ok(LevelFrame::default())
    }

    /// Level subscribers appeared (true) / all vanished (false).
    /// Default: no-op.
    fn set_metering_active(&self, _active: bool) {}
}