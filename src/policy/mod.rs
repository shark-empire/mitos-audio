//! Application policy: `/etc/mitos/policy.toml`.
//!
//! Today this covers one thing — which applications may open a microphone
//! (recording/capture) stream — because that is the permission grant
//! `docs/integration.md` already called out as the next step for the
//! security model. The shape leaves room to grow (more resources, more
//! verbs) without a breaking change: add a field, default it permissively,
//! and older policy files keep working.
//!
//! ```toml
//! default_microphone = "allow"   # "allow" | "deny"
//!
//! [[app]]
//! name = "mitos-*"        # glob, case-insensitive — same matcher as routing.toml
//! microphone = "allow"
//! ```
//!
//! Loading is lenient like the routing engine: a missing or invalid file
//! does not stop the daemon — it just means every application is allowed
//! (the pre-policy behavior), logged at warn so it's not a silent surprise.

use std::sync::Mutex;

use serde::Deserialize;

use crate::errors::AudioError;
use crate::routing::rules::glob_match_ci;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    Allow,
    Deny,
}

impl Default for Access {
    fn default() -> Self {
        Access::Allow
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppRule {
    /// Application name glob (`*`, `?`), case-insensitive — matched against
    /// the same `application` string streams and routing rules use.
    pub name: String,
    #[serde(default)]
    pub microphone: Access,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct PolicyFile {
    default_microphone: Access,
    app: Vec<AppRule>,
}

pub struct PolicyEngine {
    file: Mutex<PolicyFile>,
}

impl PolicyEngine {
    /// Load leniently: unreadable/invalid policy allows everything (warn
    /// only) — a policy file is an opt-in restriction, not a requirement.
    pub fn load(path: &str) -> Self {
        let file = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<PolicyFile>(&text) {
                Ok(file) => file,
                Err(e) => {
                    tracing::warn!(path, error = %e, "invalid policy file — allowing all applications");
                    PolicyFile::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => PolicyFile::default(),
            Err(e) => {
                tracing::warn!(path, error = %e, "cannot read policy file — allowing all applications");
                PolicyFile::default()
            }
        };
        Self { file: Mutex::new(file) }
    }

    /// Re-read the policy file (IPC `ReloadPolicy`). Returns the number of
    /// per-application rules now loaded.
    pub fn reload(&self, path: &str) -> Result<usize, AudioError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AudioError::Config(format!("cannot read {path}: {e}")))?;
        let file: PolicyFile = toml::from_str(&text)?;
        let count = file.app.len();
        *self.lock() = file;
        Ok(count)
    }

    pub fn rule_count(&self) -> usize {
        self.lock().app.len()
    }

    /// Whether `application` may open a microphone (recording/capture)
    /// stream. First matching `[[app]]` rule wins (file order); with no
    /// match, `default_microphone` applies.
    pub fn check_microphone(&self, application: &str) -> bool {
        let file = self.lock();
        for rule in &file.app {
            if glob_match_ci(&rule.name, application) {
                return rule.microphone == Access::Allow;
            }
        }
        file.default_microphone == Access::Allow
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PolicyFile> {
        self.file.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_allows_when_unset() {
        let engine = PolicyEngine { file: Mutex::new(PolicyFile::default()) };
        assert!(engine.check_microphone("anything"));
    }

    #[test]
    fn explicit_deny_blocks_matching_app() {
        let file = PolicyFile {
            default_microphone: Access::Allow,
            app: vec![AppRule { name: "sketchy-*".into(), microphone: Access::Deny }],
        };
        let engine = PolicyEngine { file: Mutex::new(file) };
        assert!(!engine.check_microphone("sketchy-widget"));
        assert!(engine.check_microphone("mitos-music"));
    }

    #[test]
    fn default_deny_requires_explicit_allow() {
        let file = PolicyFile {
            default_microphone: Access::Deny,
            app: vec![AppRule { name: "mitos-*".into(), microphone: Access::Allow }],
        };
        let engine = PolicyEngine { file: Mutex::new(file) };
        assert!(engine.check_microphone("mitos-voice"));
        assert!(!engine.check_microphone("random-app"));
    }

    #[test]
    fn first_matching_rule_wins() {
        let file = PolicyFile {
            default_microphone: Access::Allow,
            app: vec![
                AppRule { name: "app-*".into(), microphone: Access::Deny },
                AppRule { name: "app-trusted".into(), microphone: Access::Allow },
            ],
        };
        let engine = PolicyEngine { file: Mutex::new(file) };
        // "app-*" (listed first) matches "app-trusted" too, so it still wins.
        assert!(!engine.check_microphone("app-trusted"));
    }

    #[test]
    fn parses_toml() {
        let text = r#"
default_microphone = "allow"

[[app]]
name = "mitos-*"
microphone = "allow"

[[app]]
name = "sketchy-widget"
microphone = "deny"
"#;
        let file: PolicyFile = toml::from_str(text).unwrap();
        assert_eq!(file.app.len(), 2);
        assert_eq!(file.default_microphone, Access::Allow);
        assert_eq!(file.app[1].microphone, Access::Deny);
    }
}
