//! Named presets, each a starting curve for the 10 [`super::equalizer`]
//! bands plus whether the compressor stage should be on. The curves below
//! are reasonable, ordinary starting points (broad bass/treble lift for
//! Music, a presence-range lift for Voice/Podcast, and so on) — a tuned,
//! measured curve is out of scope here; `SetEqualizerBands` exists
//! precisely so a user or GUI can start from one of these and adjust.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preset {
    Flat,
    Music,
    Movie,
    Game,
    Voice,
    Podcast,
    /// Bands were set directly via `SetEqualizerBands` rather than a
    /// named preset — reported back so a GUI knows not to highlight one.
    Custom,
}

impl Preset {
    /// Gains in dB, in `equalizer::BAND_FREQUENCIES` order.
    pub fn bands(self) -> [f32; 10] {
        match self {
            Preset::Flat => [0.0; 10],
            Preset::Music => [3.0, 2.5, 1.0, -1.0, -1.5, -0.5, 1.0, 2.0, 3.0, 3.0],
            Preset::Movie => [4.0, 3.0, 2.0, 0.5, -1.0, 0.0, 1.0, 2.0, 3.0, 3.5],
            Preset::Game => [2.5, 2.0, 0.5, -1.0, 0.0, 1.0, 2.0, 3.0, 3.0, 2.0],
            Preset::Voice => [-5.0, -4.0, -2.0, 2.0, 4.0, 4.0, 3.0, 1.0, -2.0, -4.0],
            Preset::Podcast => [-6.0, -4.5, -1.5, 2.0, 3.0, 3.0, 2.0, 0.5, -1.5, -4.0],
            Preset::Custom => [0.0; 10],
        }
    }

    /// Whether this preset turns the compressor stage on. Flat and Custom
    /// leave dynamics untouched; the rest apply gentle-to-moderate
    /// compression appropriate to their use case.
    pub fn compressor(self) -> Option<(f32, f32)> {
        // (threshold_db, ratio)
        match self {
            Preset::Flat | Preset::Custom => None,
            Preset::Music => Some((-18.0, 2.5)),
            Preset::Movie => Some((-20.0, 3.0)),
            Preset::Game => Some((-16.0, 2.0)),
            Preset::Voice | Preset::Podcast => Some((-22.0, 4.0)),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Flat => "Flat",
            Preset::Music => "Music",
            Preset::Movie => "Movie",
            Preset::Game => "Game",
            Preset::Voice => "Voice",
            Preset::Podcast => "Podcast",
            Preset::Custom => "Custom",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "flat" => Some(Preset::Flat),
            "music" => Some(Preset::Music),
            "movie" => Some(Preset::Movie),
            "game" => Some(Preset::Game),
            "voice" => Some(Preset::Voice),
            "podcast" => Some(Preset::Podcast),
            "custom" => Some(Preset::Custom),
            _ => None,
        }
    }

    pub const ALL: &'static [Preset] = &[
        Preset::Flat,
        Preset::Music,
        Preset::Movie,
        Preset::Game,
        Preset::Voice,
        Preset::Podcast,
        Preset::Custom,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_every_label() {
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.label()), Some(*preset));
            assert_eq!(Preset::parse(&preset.label().to_lowercase()), Some(*preset));
        }
    }

    #[test]
    fn parse_rejects_unknown_names() {
        assert_eq!(Preset::parse("dubstep"), None);
    }

    #[test]
    fn flat_and_custom_have_no_gain_and_no_compressor() {
        assert_eq!(Preset::Flat.bands(), [0.0; 10]);
        assert_eq!(Preset::Custom.bands(), [0.0; 10]);
        assert_eq!(Preset::Flat.compressor(), None);
        assert_eq!(Preset::Custom.compressor(), None);
    }
}
