//! Ingest conversion: client format → canonical (48 kHz stereo f32).

use crate::errors::AudioError;

use super::{MIX_CHANNELS, MIX_RATE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleFormat {
    S16Le,
    F32Le,
}

impl SampleFormat {
    pub fn parse(s: &str) -> Result<Self, AudioError> {
        match s {
            "s16le" => Ok(Self::S16Le),
            "f32le" => Ok(Self::F32Le),
            other => Err(AudioError::UnsupportedFormat(other.to_string())),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::S16Le => "s16le",
            Self::F32Le => "f32le",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub format: SampleFormat,
}

pub fn validate(rate: u32, channels: u16) -> Result<(), AudioError> {
    if !(1_000..=384_000).contains(&rate) {
        return Err(AudioError::UnsupportedFormat(format!("sample rate {rate}")));
    }
    if channels == 0 || channels > 8 {
        return Err(AudioError::UnsupportedFormat(format!("{channels} channels")));
    }
    Ok(())
}

/// Convert a raw PCM payload to the canonical format:
/// decode → channel up/downmix → linear resample.
pub fn ingest(fmt: &StreamFormat, bytes: &[u8]) -> Vec<f32> {
    let decoded = decode(fmt.format, bytes);
    let stereo = to_stereo(&decoded, fmt.channels as usize);
    if fmt.sample_rate == MIX_RATE {
        stereo
    } else {
        resample_stereo(&stereo, fmt.sample_rate)
    }
}

fn decode(format: SampleFormat, bytes: &[u8]) -> Vec<f32> {
    match format {
        SampleFormat::S16Le => bytes
            .chunks_exact(2)
            .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
            .collect(),
        SampleFormat::F32Le => bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    }
}

/// 1 → duplicate, 2 → as-is, >2 → first two channels (documented).
fn to_stereo(decoded: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        1 => decoded.iter().flat_map(|&s| [s, s]).collect(),
        2 => decoded.to_vec(),
        _ => decoded.chunks(channels).flat_map(|frame| [frame[0], frame[1]]).collect(),
    }
}

/// Naive linear resampling on interleaved stereo. Fine for v0.4; a
/// band-limited resampler is on the roadmap.
fn resample_stereo(input: &[f32], src_rate: u32) -> Vec<f32> {
    let frames_in = input.len() / MIX_CHANNELS;
    if frames_in == 0 {
        return Vec::new();
    }
    let frames_out =
        ((frames_in as f64) * f64::from(MIX_RATE) / f64::from(src_rate)).floor() as usize;
    let step = f64::from(src_rate) / f64::from(MIX_RATE);
    let mut out = Vec::with_capacity(frames_out * MIX_CHANNELS);
    for i in 0..frames_out {
        let pos = i as f64 * step;
        let i0 = pos.floor() as usize;
        let i1 = (i0 + 1).min(frames_in - 1);
        let t = (pos - i0 as f64) as f32;
        for c in 0..MIX_CHANNELS {
            let a = input[i0 * MIX_CHANNELS + c];
            let b = input[i1 * MIX_CHANNELS + c];
            out.push(a + (b - a) * t);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s16_decodes_to_unit_range() {
        let bytes = i16::MAX.to_le_bytes();
        assert!((decode(SampleFormat::S16Le, &bytes)[0] - 1.0).abs() < 1e-4);
    }

    #[test]
    fn mono_upmixes() {
        let decoded = [0.25f32, 0.5];
        assert_eq!(to_stereo(&decoded, 1), vec![0.25, 0.25, 0.5, 0.5]);
    }

    #[test]
    fn multichannel_takes_first_two() {
        let decoded = [0.1f32, 0.2, 0.3, 0.4, 0.5, 0.6];
        assert_eq!(to_stereo(&decoded, 3), vec![0.1, 0.2, 0.4, 0.5]);
    }

    #[test]
    fn passthrough_when_native_rate() {
        let fmt = StreamFormat { sample_rate: MIX_RATE, channels: 2, format: SampleFormat::S16Le };
        assert_eq!(ingest(&fmt, &[0, 0, 0, 0]).len(), 4);
    }

    #[test]
    fn resample_44100_to_48000() {
        let input: Vec<f32> = (0..441 * MIX_CHANNELS).map(|i| (i % 97) as f32 / 97.0).collect();
        let out = resample_stereo(&input, 44_100);
        assert_eq!(out.len(), 480 * MIX_CHANNELS);
    }
}