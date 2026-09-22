//! Peer permission checks (SO_PEERCRED based).
//!
//! Allowed to connect (control AND data plane):
//! - root
//! - the daemon's own uid (same-user dev runs)
//! - members of the `mitos-audio` group — primary gid, or supplementary
//!   groups read from /proc/<pid>/status (works when the daemon is root)
//!
//! Setup: `sudo groupadd -r mitos-audio && sudo usermod -aG mitos-audio <user>`
//!
//! Requires tokio ≥ 1.24 (UCred::pid) and Rust ≥ 1.73 (chown) for the
//! group features; both degrade gracefully.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use tokio::net::unix::UCred;

pub struct Permissions {
    daemon_uid: u32,
    audio_group: Option<u32>,
}

impl Permissions {
    pub fn current() -> Self {
        let daemon_uid = fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
        let audio_group = read_group_gid("mitos-audio");
        Self { daemon_uid, audio_group }
    }

    pub fn audio_group_gid(&self) -> Option<u32> {
        self.audio_group
    }

    pub fn check(&self, cred: Option<&UCred>) -> bool {
        let Some(cred) = cred else { return false };
        let uid = cred.uid();
        if uid == 0 || uid == self.daemon_uid {
            return true;
        }
        let Some(gid) = self.audio_group else { return false };
        if cred.gid() == gid {
            return true;
        }
        // Supplementary groups of the connecting process.
        if let Some(pid) = cred.pid().filter(|p| *p > 0) {
            if let Ok(status) = fs::read_to_string(format!("/proc/{pid}/status")) {
                if let Some(line) = status.lines().find(|l| l.starts_with("Groups:")) {
                    let groups: HashSet<u32> = line["Groups:".len()..]
                        .split_whitespace()
                        .filter_map(|g| g.parse().ok())
                        .collect();
                    return groups.contains(&gid);
                }
            }
        }
        false
    }
}

fn read_group_gid(name: &str) -> Option<u32> {
    let group = fs::read_to_string("/etc/group").ok()?;
    for line in group.lines() {
        let mut fields = line.split(':');
        if fields.next() == Some(name) {
            fields.next(); // passwd field
            return fields.next().and_then(|g| g.parse().ok());
        }
    }
    None
}

/// Restrict a freshly-bound socket: 0660, and chown to the `mitos-audio`
/// group when permitted (root daemon) so group members may connect.
pub fn secure_socket(path: &str, group: Option<u32>) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o660));
    if let Some(gid) = group {
        let _ = std::os::unix::fs::chown(path, None, Some(gid));
    }
}