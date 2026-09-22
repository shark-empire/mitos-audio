use crate::devices::device::Device;
use crate::errors::AudioError;

/// Hardware-facing interface. Implementations must be `Send + Sync`;
/// they are invoked through `tokio::task::spawn_blocking`.
///
/// Contract:
/// - `scan` is the source of truth for which devices exist and their
///   current hardware volume/mute.
/// - `set_volume`/`set_mute` return `Ok(false)` when the device has no
///   such hardware control (soft-fail, not an error).
pub trait AudioBackend: Send + Sync {
    /// Stable backend identifier ("alsa" / "demo").
    fn name(&self) -> &'static str;

    /// Enumerate all devices currently present, with live volume/mute
    /// read from hardware where available.
    fn scan(&self) -> Result<Vec<Device>, AudioError>;

    /// Push a volume change (0–100) to hardware.
    fn set_volume(&self, device: &Device, volume: u32) -> Result<bool, AudioError>;

    /// Push mute state to hardware.
    fn set_mute(&self, device: &Device, mute: bool) -> Result<bool, AudioError>;
}