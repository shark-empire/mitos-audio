//! Real ALSA backend.
//!
//! - **Enumeration**: `/proc/asound/cards` + `/proc/asound/pcm`
//!   (stable procfs text interface — every PCM device, incl. HDMI).
//! - **Control**: ALSA mixer simple elements (`alsa` crate) for
//!   hardware volume and mute.
//! - **Not covered yet**: Bluetooth (via future mitos-bluetooth),
//!   sample-format probing (needs PCM open), PCM metering.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use alsa::mixer::{Mixer, Selem, SelemChannelId, SelemId};

use super::backend::AudioBackend;
use crate::devices::device::{Bus, Device, DeviceKind, Direction};
use crate::errors::AudioError;

/// Mixer control names tried, in order, when looking for a volume control.
const OUTPUT_CONTROLS: &[&str] = &["Master", "Headphone", "Headphones", "Speaker", "PCM"];
const INPUT_CONTROLS: &[&str] = &["Capture", "Input", "Mic", "Internal Mic", "Headset Mic"];

pub struct AlsaBackend;

impl AlsaBackend {
    pub fn new() -> Result<Self, AudioError> {
        if !Path::new("/proc/asound").exists() {
            return Err(AudioError::Backend(
                "ALSA is not available (/proc/asound not found)".into(),
            ));
        }
        Ok(Self)
    }
}

// ─── procfs parsing ─────────────────────────────────────────────────────

struct CardInfo {
    index: u32,
    #[allow(dead_code)]
    id: String,
    driver: String,
    name: String,
}

struct PcmEntry {
    card: u32,
    device: u32,
    name: String,
    id: String,
    playback: bool,
    capture: bool,
}

/// `/proc/asound/cards`:
/// ```text
///  0 [PCH            ] : HDA-Intel - HDA Intel PCH
///                       HDA Intel PCH at 0xf7d34000 irq 48
/// ```
fn parse_cards() -> Result<Vec<CardInfo>, AudioError> {
    let text = fs::read_to_string("/proc/asound/cards")
        .map_err(|e| AudioError::Backend(format!("cannot read /proc/asound/cards: {e}")))?;

    let mut cards = Vec::new();
    for line in text.lines() {
        // Header lines contain " [", detail lines don't.
        let Some((head, tail)) = line.split_once('[') else { continue };
        let Ok(index) = head.trim().parse::<u32>() else { continue };
        let Some((id, rest)) = tail.split_once(']') else { continue };

        let after = rest.split_once(':').map(|(_, a)| a).unwrap_or(rest);
        let (driver, name) = match after.split_once(" - ") {
            Some((d, n)) => (d.trim().to_string(), n.trim().to_string()),
            None => (after.trim().to_string(), after.trim().to_string()),
        };
        cards.push(CardInfo { index, id: id.trim().to_string(), driver, name });
    }
    Ok(cards)
}

/// `/proc/asound/pcm`:
/// ```text
/// 00-00: ALC3232 Analog : ALC3232 Analog : playback 1 : capture 1
/// 00-03: HDMI 0 : HDMI 0 : playback 1
/// ```
fn parse_pcm_devices() -> Result<Vec<PcmEntry>, AudioError> {
    let text = fs::read_to_string("/proc/asound/pcm")
        .map_err(|e| AudioError::Backend(format!("cannot read /proc/asound/pcm: {e}")))?;

    let mut entries = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() { continue; }

        let Some((nums, rest)) = line.split_once(':') else { continue };
        let Some((card, device)) = nums.split_once('-') else { continue };
        let (Ok(card), Ok(device)) =
            (card.trim().parse::<u32>(), device.trim().parse::<u32>())
        else { continue };

        let mut parts = rest.split(':').map(str::trim);
        let name = parts.next().unwrap_or("PCM");
        let id = parts.next().unwrap_or(name);

        entries.push(PcmEntry {
            card,
            device,
            name: name.to_string(),
            id: id.to_string(),
            playback: rest.contains("playback"),
            capture: rest.contains("capture"),
        });
    }
    Ok(entries)
}

// ─── mixer helpers ──────────────────────────────────────────────────────

/// "hw:0,3" → 0
fn card_index_of(alsa_id: &str) -> Option<i32> {
    let rest = alsa_id.strip_prefix("hw:")?;
    rest.split(',').next()?.parse().ok()
}

fn open_mixer(card: i32) -> Result<Mixer, AudioError> {
    Mixer::new(&format!("hw:{card}"), false)
        .map_err(|e| AudioError::Backend(format!("cannot open mixer for hw:{card}: {e}")))
}

fn select_control(mixer: &Mixer, output: bool) -> Option<Selem<'_>> {
    let names = if output { OUTPUT_CONTROLS } else { INPUT_CONTROLS };
    for name in names {
        // SelemId::new(name, index) — None selects the default element index.
        if let Some(selem) = mixer.find_selem(&SelemId::new(name, None)) {
            let capable = if output {
                selem.has_playback_volume()
            } else {
                selem.has_capture_volume()
            };
            if capable {
                return Some(selem);
            }
        }
    }
    None
}

fn percent_to_raw(selem: &Selem, percent: u32, output: bool) -> Option<i64> {
    let (min, max) = volume_range(selem, output)?;
    if max <= min { return None; } // fixed-volume control
    Some(min + (max - min) * i64::from(percent) / 100)
}

fn raw_to_percent(selem: &Selem, raw: i64, output: bool) -> Option<u32> {
    let (min, max) = volume_range(selem, output)?;
    if max <= min { return None; }
    Some(((raw - min) * 100 / (max - min)).clamp(0, 100) as u32)
}

fn volume_range(selem: &Selem, output: bool) -> Option<(i64, i64)> {
    if output {
        selem.get_playback_volume_range().ok()
    } else {
        selem.get_capture_volume_range().ok()
    }
}

/// Live (volume, muted) from hardware. `None` = no usable control.
fn read_mixer_state(card: u32, output: bool) -> Option<(u32, bool)> {
    let mixer = open_mixer(card as i32).ok()?;
    let selem = select_control(&mixer, output)?;
    // FrontLeft also addresses mono controls (ALSA aliases SCHN_MONO == 0).
    let channel = SelemChannelId::FrontLeft;

    let raw = if output {
        selem.get_playback_volume(channel).ok()?
    } else {
        selem.get_capture_volume(channel).ok()?
    };
    let volume = raw_to_percent(&selem, raw, output)?;

    let muted = if output {
        selem.has_playback_switch()
            && selem.get_playback_switch(channel).map(|v| v == 0).unwrap_or(false)
    } else {
        selem.has_capture_switch()
            && selem.get_capture_switch(channel).map(|v| v == 0).unwrap_or(false)
    };
    Some((volume, muted))
}

// ─── AudioBackend implementation ────────────────────────────────────────

impl AudioBackend for AlsaBackend {
    fn name(&self) -> &'static str { "alsa" }

    fn scan(&self) -> Result<Vec<Device>, AudioError> {
        let cards = parse_cards()?;
        let pcms = parse_pcm_devices()?;
        let cards_by_index: HashMap<u32, &CardInfo> =
            cards.iter().map(|c| (c.index, c)).collect();

        let mut devices = Vec::new();
        for pcm in &pcms {
            let direction = match (pcm.playback, pcm.capture) {
                (false, false) => continue, // control-only node
                (true, false) => Direction::Output,
                (false, true) => Direction::Input,
                (true, true) => Direction::Both,
            };

            let card = cards_by_index.get(&pcm.card).copied();
            let card_name = card.map(|c| c.name.as_str()).unwrap_or("unknown card");
            let card_driver = card.map(|c| c.driver.as_str()).unwrap_or("unknown");

            // Classification heuristics from PCM/card names.
            let hay = format!("{} {} {}", pcm.name, pcm.id, card_name).to_lowercase();
            let is_usb = hay.contains("usb") || card_driver.to_lowercase().contains("usb");

            let kind = if hay.contains("hdmi") {
                DeviceKind::Hdmi
            } else if hay.contains("displayport") {
                DeviceKind::DisplayPort
            } else if hay.contains("headset") {
                DeviceKind::Headset
            } else if hay.contains("headphone") {
                DeviceKind::Headphones
            } else if is_usb {
                DeviceKind::Usb
            } else if hay.contains("mic") || direction == Direction::Input {
                DeviceKind::Microphone
            } else {
                DeviceKind::Speakers
            };

            let bus = if is_usb {
                Bus::Usb
            } else if matches!(kind, DeviceKind::Hdmi | DeviceKind::DisplayPort) {
                Bus::Hdmi
            } else {
                Bus::Internal
            };

            let id = format!("alsa:hw:{},{}", pcm.card, pcm.device);
            let mut device = Device::new(&id, pcm.name, kind, direction, bus);
            device.description = format!("{} — {} ({})", pcm.name, card_name, card_driver);
            device.alsa = Some(format!("hw:{},{}", pcm.card, pcm.device));
            if kind == DeviceKind::Hdmi {
                device.profiles = vec!["hdmi-stereo".into(), "stereo".into()];
                device.active_profile = Some("hdmi-stereo".into());
            }

            // Live hardware volume/mute.
            let output_side = direction != Direction::Input;
            if let Some((volume, muted)) = read_mixer_state(pcm.card, output_side) {
                device.volume = volume;
                device.muted = muted;
            }
            devices.push(device);
        }

        // Fallback for cards with no PCM lines (rare): one device per card.
        if devices.is_empty() {
            for card in &cards {
                let mut device = Device::new(
                    &format!("alsa:hw:{}", card.index),
                    &card.name,
                    DeviceKind::Speakers,
                    Direction::Both,
                    Bus::Internal,
                );
                device.alsa = Some(format!("hw:{}", card.index));
                if let Some((v, m)) = read_mixer_state(card.index, true) {
                    device.volume = v;
                    device.muted = m;
                }
                devices.push(device);
            }
        }

        Ok(devices)
    }

    fn set_volume(&self, device: &Device, volume: u32) -> Result<bool, AudioError> {
        let Some(alsa_id) = &device.alsa else { return Ok(false) };
        let Some(card) = card_index_of(alsa_id) else { return Ok(false) };
        let output = device.direction != Direction::Input;

        let mixer = open_mixer(card)?;
        let Some(selem) = select_control(&mixer, output) else { return Ok(false) };
        let Some(raw) = percent_to_raw(&selem, volume, output) else { return Ok(false) };

        if output {
            selem.set_playback_volume_all(raw)
                .map_err(|e| AudioError::Backend(format!("set_playback_volume_all: {e}")))?;
        } else {
            selem.set_capture_volume_all(raw)
                .map_err(|e| AudioError::Backend(format!("set_capture_volume_all: {e}")))?;
        }
        Ok(true)
    }

    fn set_mute(&self, device: &Device, mute: bool) -> Result<bool, AudioError> {
        let Some(alsa_id) = &device.alsa else { return Ok(false) };
        let Some(card) = card_index_of(alsa_id) else { return Ok(false) };
        let output = device.direction != Direction::Input;

        let mixer = open_mixer(card)?;
        let Some(selem) = select_control(&mixer, output) else { return Ok(false) };
        // ALSA switch convention: 1 = on, 0 = muted.
        let value = i64::from(!mute);

        if output {
            if !selem.has_playback_switch() { return Ok(false); }
            selem.set_playback_switch_all(value)
                .map_err(|e| AudioError::Backend(format!("set_playback_switch_all: {e}")))?;
        } else {
            if !selem.has_capture_switch() { return Ok(false); }
            selem.set_capture_switch_all(value)
                .map_err(|e| AudioError::Backend(format!("set_capture_switch_all: {e}")))?;
        }
        Ok(true)
    }
}