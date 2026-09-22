use serde::{Deserialize, Serialize};

/// One level-meter frame (normalized 0.0–1.0).
///
/// `output_level`/`input_level` are smoothed RMS; `peak` tracks decaying
/// peaks; `clipping` is true when the input hit near-full-scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct LevelFrame {
    pub output_level: f32,
    pub input_level: f32,
    pub peak: f32,
    pub clipping: bool,
}