//! Routing rules: `/etc/mitos/routing.toml`
//!
//! ```toml
//! [[rule]]
//! name = "bluetooth-headset-priority"
//! when = "device-added"
//! match = { kind = "headset", bus = "bluetooth" }
//!
//! [[rule.action]]
//! type = "set-default-output"
//! target = "trigger"        # the device that triggered the rule
//! remember = true           # save the previous default for restore
//! ```

use serde::Deserialize;

/// What fires a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    DeviceAdded,
    DeviceRemoved,
    StreamAdded,
    ProfileChanged,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RuleFile {
    #[serde(default)]
    pub rule: Vec<Rule>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub name: String,
    pub when: Trigger,
    #[serde(default)]
    pub r#match: RuleMatch,
    pub action: Vec<ActionSpec>,
    /// Stop rule evaluation after this rule fires.
    #[serde(default)]
    pub stop: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// Conditions — all present patterns must match (AND). `*`/`?` globs,
/// case-insensitive. Empty = matches anything.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct RuleMatch {
    /// Device kind: "speakers", "headset", "hdmi", "usb", "microphone", …
    pub kind: Option<String>,
    /// Bus: "internal", "usb", "bluetooth", "hdmi", "virtual"
    pub bus: Option<String>,
    /// Human-readable device name, e.g. "HDAIntel*"
    pub name: Option<String>,
    /// Device id, e.g. "alsa:hw:1,0"
    pub id: Option<String>,
    /// Stream application name (stream triggers), e.g. "mitos-*"
    pub application: Option<String>,
    /// Current profile at trigger time, e.g. "bluetooth-*"
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ActionSpec {
    SetDefaultOutput {
        /// "trigger" (the triggering device) or a literal device id.
        target: String,
        /// Remember the previous default for `restore-output`.
        #[serde(default)]
        remember: bool,
    },
    SetDefaultInput {
        target: String,
    },
    SetProfile {
        profile: String,
    },
    MoveStream {
        /// Destination device id.
        to: String,
    },
    RestoreOutput,
    RestoreInput,
}

/// Glob match supporting `*` and `?` (case-insensitive wrapper below).
pub(crate) fn glob_match(p: &[char], t: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_p, mut star_t) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star_p = pi;
            pi += 1;
            star_t = ti;
        } else if star_p != usize::MAX {
            pi = star_p + 1;
            star_t += 1;
            ti = star_t;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

pub(crate) fn glob_match_ci(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    glob_match(&p, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs() {
        assert!(glob_match_ci("headset", "Headset"));
        assert!(glob_match_ci("bluetooth*", "Bluetooth Headset"));
        assert!(glob_match_ci("mitos-*", "mitos-music"));
        assert!(glob_match_ci("*game*", "My Game"));
        assert!(glob_match_ci("alsa:hw:?,0", "alsa:hw:1,0"));
        assert!(!glob_match_ci("alsa:hw:?,*", "alsa:hw:12,0"));
        assert!(glob_match_ci("a*c*e", "abcde"));
        assert!(!glob_match_ci("hdmi*", "speakers"));
    }

    #[test]
    fn parses_rules() {
        let text = r#"
[[rule]]
name = "bt-connect"
when = "device-added"
match = { kind = "headset", bus = "bluetooth" }

[[rule.action]]
type = "set-default-output"
target = "trigger"
remember = true

[[rule.action]]
type = "set-profile"
profile = "bluetooth-headset"
"#;
        let file: RuleFile = toml::from_str(text).unwrap();
        assert_eq!(file.rule.len(), 1);
        assert_eq!(file.rule[0].when, Trigger::DeviceAdded);
        assert_eq!(file.rule[0].action.len(), 2);
        assert!(file.rule[0].enabled);
        assert!(!file.rule[0].stop);
    }
}