//! Bluetooth *audio* coordination — not a Bluetooth stack.
//!
//! mitos-audio does not talk to a Bluetooth controller, negotiate A2DP/HFP
//! itself, or manage pairing; a real Bluetooth device that's paired and
//! connected at the OS level (via BlueZ + bluealsa or similar) already
//! shows up here as an ordinary [`crate::devices::device::Device`] with
//! `bus: Bluetooth` — see `backend::alsa`'s scan and `backend::demo`'s
//! `bt-headset` for what that looks like. Duplicating BlueZ's job inside
//! the audio daemon was explicitly ruled out from the start (see the
//! original design notes) in favor of a future dedicated
//! `mitos-bluetooth` service.
//!
//! What *does* belong here is the audio-quality policy that service will
//! need from mitos-audio, and the small amount of state that's genuinely
//! about audio rather than the link:
//! - [`codec::negotiate`] — given the codecs both ends support, which one
//!   should actually get used.
//! - [`profile::BluetoothProfile`] — mapping A2DP/HFP/HSP onto mitos-audio's
//!   own profile catalog ([`crate::profiles`]) and knowing which of them
//!   carry a microphone.
//! - [`manager::BluetoothCoordinator`] — the trait that service calls into
//!   once it exists.
//!
//! [`manager::BluetoothManager`] is usable standalone today (see its own
//! tests) and is wired into [`crate::backend::demo`]'s Bluetooth device so
//! the whole path — connect, negotiate, populate `Device::codec` — is
//! exercised without needing real Bluetooth hardware or a real
//! mitos-bluetooth to talk to.

pub mod codec;
pub mod device;
pub mod manager;
pub mod profile;

pub use codec::Codec;
pub use device::BluetoothDeviceInfo;
pub use manager::{BluetoothCoordinator, BluetoothManager};
pub use profile::BluetoothProfile;
