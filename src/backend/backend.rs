use crate::devices::device::Device;
use crate::errors::AudioError;
use crate::monitoring::LevelFrame;

/// Hardware-facing interface. Implementations must be `Send + Sync`;
/// they are invoked through `tokio::task::spawn_blocking`.
///
/// Contract:
/// - `scan` is the source of truth for which devices exist and their
///   current hardware volume/mute.
/// - `set_volume`/`set_mute` return `Ok(false)` when the device has no
///   such hardware control (soft-fail, not an error).
/// - `levels` is only called while level subscribers exist; backends may
///   park hardware (close capture PCMs) when `set_metering_active(false)`.
pub trait AudioBackend: Send + Sync {
    fn name(&self) -> &'static str;

    fn scan(&self) -> Result<Vec<Device>, AudioError>;

    fn set_volume(&self, device: &Device, volume: u32) -> Result<bool, AudioError>;

    fn set_mute(&self, device: &Device, mute: bool) -> Result<bool, AudioError>;

    /// Current level frame. Default: zeros (backend without metering).
    fn levels(&self) -> Result<LevelFrame, AudioError> {
        Ok(LevelFrame::default())
    }

    /// Level subscribers appeared (true) / all vanished (false).
    /// Default: no-op.
    fn set_metering_active(&self, _active: bool) {}
}