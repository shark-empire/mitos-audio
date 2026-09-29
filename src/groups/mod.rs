//! Speaker groups: synchronized multi-speaker output.
//!
//! Asynchronous/independent multi-speaker output needs nothing new — a
//! stream already targets exactly one device, different streams already
//! target different devices independently, and they already coexist (see
//! `docs/architecture.md`). What's new here is **synchronized** fan-out:
//! one stream, mixed once, delivered to every member device at the same
//! instant.
//!
//! A group is a *virtual device*: it gets an id like any real `Device`,
//! streams target it exactly the way they'd target `"speakers"` or
//! `"hdmi"` (`CreateStream`, `MoveStream`, routing.toml — all unchanged),
//! and `engine::sink::SinkManager` recognizes that id and opens every
//! member's real output instead of one. See `engine::sink`'s
//! `group_mixer_loop` for the real-time side.
//!
//! mitos-audio does not measure acoustic or network latency itself — that
//! needs a per-transport round trip (and, per the original design notes,
//! "actual achievable synchronization depends on the underlying hardware
//! and transport, especially Bluetooth"). What it does with a *configured*
//! `latency_ms` per member is delay every other member by the difference,
//! so the slowest member sets the pace instead of a faster one playing
//! ahead of it — see [`compute_delays_ms`]. Continuous clock-drift
//! correction (independent hardware clocks slowly diverging over minutes)
//! is not implemented; see `docs/audio-model.md`'s groups section for
//! the honest boundary.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeakerGroup {
    pub id: String,
    pub name: String,
    pub members: Vec<GroupMember>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupMember {
    pub device_id: String,
    /// Configured output latency for this member, in milliseconds — see
    /// [`compute_delays_ms`]. 0 (the default) means "no compensation
    /// requested for this member," not "zero measured latency."
    #[serde(default)]
    pub latency_ms: u32,
}

impl SpeakerGroup {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self { id: id.into(), name: name.into(), members: Vec::new() }
    }

    pub fn has_member(&self, device_id: &str) -> bool {
        self.members.iter().any(|m| m.device_id == device_id)
    }
}

/// For each member, how long to *delay* it (ms) so every member lines up
/// with whichever one has the highest configured `latency_ms`. The
/// highest-latency member gets 0 (it already sets the pace); every other
/// member is held back by the difference.
pub fn compute_delays_ms(members: &[GroupMember]) -> Vec<(String, u32)> {
    let max_latency = members.iter().map(|m| m.latency_ms).max().unwrap_or(0);
    members.iter().map(|m| (m.device_id.clone(), max_latency.saturating_sub(m.latency_ms))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, latency_ms: u32) -> GroupMember {
        GroupMember { device_id: id.to_string(), latency_ms }
    }

    #[test]
    fn delays_bring_every_member_to_the_slowest_ones_pace() {
        let members = vec![member("a", 20), member("b", 80)];
        let delays = compute_delays_ms(&members);
        assert_eq!(delays.iter().find(|(id, _)| id == "a").unwrap().1, 60); // 80 - 20
        assert_eq!(delays.iter().find(|(id, _)| id == "b").unwrap().1, 0); // slowest — no delay
    }

    #[test]
    fn equal_latencies_need_no_compensation() {
        let members = vec![member("a", 40), member("b", 40)];
        let delays = compute_delays_ms(&members);
        assert!(delays.iter().all(|(_, d)| *d == 0));
    }

    #[test]
    fn unconfigured_latencies_default_to_zero_delay() {
        let members = vec![member("a", 0), member("b", 0)];
        let delays = compute_delays_ms(&members);
        assert!(delays.iter().all(|(_, d)| *d == 0));
    }

    #[test]
    fn three_members_all_align_to_the_slowest() {
        let members = vec![member("a", 10), member("b", 25), member("c", 60)];
        let delays = compute_delays_ms(&members);
        assert_eq!(delays.iter().find(|(id, _)| id == "a").unwrap().1, 50);
        assert_eq!(delays.iter().find(|(id, _)| id == "b").unwrap().1, 35);
        assert_eq!(delays.iter().find(|(id, _)| id == "c").unwrap().1, 0);
    }

    #[test]
    fn has_member_checks_by_device_id() {
        let mut group = SpeakerGroup::new("living-room", "Living Room");
        group.members.push(member("speakers", 0));
        assert!(group.has_member("speakers"));
        assert!(!group.has_member("hdmi"));
    }
}
