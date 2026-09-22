/// Peer permission checks (SO_PEERCRED based).
///
/// v0.1: root and the daemon's own uid are allowed.
/// Next: group `mitos-audio` + per-application policy from policy.toml.
pub struct Permissions {
    daemon_uid: u32,
}

impl Permissions {
    pub fn current() -> Self {
        use std::os::unix::fs::MetadataExt;
        let uid = std::fs::metadata("/proc/self").map(|m| m.uid()).unwrap_or(0);
        Self { daemon_uid: uid }
    }

    pub fn check_uid(&self, uid: Option<u32>) -> bool {
        match uid {
            Some(uid) => uid == 0 || uid == self.daemon_uid,
            None => false,
        }
    }
}