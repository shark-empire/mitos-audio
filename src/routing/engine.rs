//! Rule evaluation. Pure with respect to rules; the restore *memory*
//! (remembered previous defaults) is stateful and persisted by the manager.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::devices::device::Device;
use crate::errors::AudioError;
use crate::streams::stream::AudioStream;

use super::rules::{glob_match_ci, ActionSpec, Rule, RuleFile, RuleMatch, Trigger};

/// Remembered defaults, persisted in `state.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingMemory {
    #[serde(default)]
    pub previous_output: Option<String>,
    #[serde(default)]
    pub previous_input: Option<String>,
}

/// Everything a trigger knows when rules are evaluated.
pub struct TriggerContext<'a> {
    pub trigger: Trigger,
    /// The device that appeared/vanished — or, for stream triggers, the
    /// stream's current device (enables device-scoped app rules).
    pub device: Option<&'a Device>,
    pub stream: Option<&'a AudioStream>,
    pub profile: &'a str,
}

/// A rule action with `target = "trigger"` resolved to a concrete device id.
#[derive(Debug, Clone)]
pub enum ResolvedAction {
    SetDefaultOutput { id: String, remember: bool },
    SetDefaultInput { id: String },
    SetProfile { profile: String },
    MoveStream { stream_id: String, to: String },
    RestoreOutput,
    RestoreInput,
}

pub struct RoutingEngine {
    rules: Mutex<Vec<Rule>>,
    memory: Mutex<RoutingMemory>,
}

impl RoutingEngine {
    /// Load leniently: unreadable/invalid rules disable routing (warn only).
    pub fn load(path: &str) -> Self {
        let rules = match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<RuleFile>(&text) {
                Ok(file) => file.rule,
                Err(e) => {
                    tracing::warn!(path, error = %e, "invalid routing rules — running without routing");
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                tracing::warn!(path, error = %e, "cannot read routing rules — running without routing");
                Vec::new()
            }
        };
        Self { rules: Mutex::new(rules), memory: Mutex::new(RoutingMemory::default()) }
    }

    /// Re-read the rules file (IPC `ReloadRouting`).
    pub fn reload(&self, path: &str) -> Result<usize, AudioError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| AudioError::Config(format!("cannot read {path}: {e}")))?;
        let file: RuleFile = toml::from_str(&text)?;
        let count = file.rule.len();
        *self.lock_rules() = file.rule;
        Ok(count)
    }

    pub fn rule_count(&self) -> usize {
        self.lock_rules().len()
    }

    /// Evaluate all rules for one trigger, in file order.
    /// Matching is pure; memory mutations happen when the manager *executes*
    /// the returned actions.
    pub fn evaluate(&self, ctx: &TriggerContext<'_>) -> Vec<ResolvedAction> {
        let rules = self.lock_rules().clone();
        let mut out = Vec::new();
        'rules: for rule in &rules {
            if !rule.enabled || rule.when != ctx.trigger {
                continue;
            }
            if !rule_matches(&rule.r#match, ctx) {
                continue;
            }
            for spec in &rule.action {
                match resolve_action(spec, ctx) {
                    Some(action) => out.push(action),
                    None => tracing::warn!(
                        rule = %rule.name,
                        "action skipped — no trigger device to resolve 'trigger' target"
                    ),
                }
            }
            if rule.stop {
                break 'rules;
            }
        }
        out
    }

    // ── memory (poisoning recovered — routing memory is advisory state) ──

    pub fn memory(&self) -> RoutingMemory {
        self.lock_memory().clone()
    }

    pub fn restore_memory(&self, memory: RoutingMemory) {
        *self.lock_memory() = memory;
    }

    pub fn note_output_switch(&self, previous: Option<String>) {
        self.lock_memory().previous_output = previous;
    }

    pub fn note_input_switch(&self, previous: Option<String>) {
        self.lock_memory().previous_input = previous;
    }

    pub fn take_previous_output(&self) -> Option<String> {
        self.lock_memory().previous_output.take()
    }

    pub fn take_previous_input(&self) -> Option<String> {
        self.lock_memory().previous_input.take()
    }

    fn lock_rules(&self) -> std::sync::MutexGuard<'_, Vec<Rule>> {
        self.rules.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_memory(&self) -> std::sync::MutexGuard<'_, RoutingMemory> {
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn rule_matches(m: &RuleMatch, ctx: &TriggerContext<'_>) -> bool {
    if let Some(pattern) = &m.profile {
        if !glob_match_ci(pattern, ctx.profile) {
            return false;
        }
    }
    if let Some(pattern) = &m.application {
        let Some(stream) = ctx.stream else { return false };
        if !glob_match_ci(pattern, &stream.application) {
            return false;
        }
    }
    let has_device_condition =
        m.kind.is_some() || m.bus.is_some() || m.name.is_some() || m.id.is_some();
    if has_device_condition {
        let Some(device) = ctx.device else { return false };
        if let Some(pattern) = &m.kind {
            if !glob_match_ci(pattern, &format!("{:?}", device.kind).to_lowercase()) {
                return false;
            }
        }
        if let Some(pattern) = &m.bus {
            if !glob_match_ci(pattern, &format!("{:?}", device.bus).to_lowercase()) {
                return false;
            }
        }
        if let Some(pattern) = &m.name {
            if !glob_match_ci(pattern, &device.name) {
                return false;
            }
        }
        if let Some(pattern) = &m.id {
            if !glob_match_ci(pattern, &device.id) {
                return false;
            }
        }
    }
    true
}

fn resolve_action(spec: &ActionSpec, ctx: &TriggerContext<'_>) -> Option<ResolvedAction> {
    Some(match spec {
        ActionSpec::SetDefaultOutput { target, remember } => ResolvedAction::SetDefaultOutput {
            id: resolve_target(target, ctx)?,
            remember: *remember,
        },
        ActionSpec::SetDefaultInput { target } => {
            ResolvedAction::SetDefaultInput { id: resolve_target(target, ctx)? }
        }
        ActionSpec::SetProfile { profile } => {
            ResolvedAction::SetProfile { profile: profile.clone() }
        }
        ActionSpec::MoveStream { to } => ResolvedAction::MoveStream {
            stream_id: ctx.stream?.id.clone(),
            to: to.clone(),
        },
        ActionSpec::RestoreOutput => ResolvedAction::RestoreOutput,
        ActionSpec::RestoreInput => ResolvedAction::RestoreInput,
    })
}

fn resolve_target(target: &str, ctx: &TriggerContext<'_>) -> Option<String> {
    if target.eq_ignore_ascii_case("trigger") {
        Some(ctx.device?.id.clone())
    } else {
        Some(target.to_string())
    }
}