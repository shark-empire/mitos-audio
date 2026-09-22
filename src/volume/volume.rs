use crate::errors::AudioError;

/// Validated percentage volume (0–100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Volume(u32);

impl Volume {
    pub fn new(value: u32) -> Result<Self, AudioError> {
        if value <= 100 {
            Ok(Self(value))
        } else {
            Err(AudioError::InvalidVolume(value))
        }
    }

    pub fn get(self) -> u32 {
        self.0
    }
}