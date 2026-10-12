//! Typed permanent Patch recovery obligations shared by Runtime and lifecycle owners.
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPatchRecoveryState {
    pub version: u8,
    pub targets: Vec<String>,
    #[serde(default)]
    pub unresolved_targets: Vec<String>,
    #[serde(default)]
    pub unresolved_before_input: Option<usize>,
    pub target_budget_exceeded: bool,
}

impl Default for AgentPatchRecoveryState {
    fn default() -> Self {
        Self { version: 2, targets: Vec::new(), unresolved_targets: Vec::new(),
            unresolved_before_input: None, target_budget_exceeded: false }
    }
}

impl AgentPatchRecoveryState {
    pub fn has_pending(&self) -> bool {
        self.target_budget_exceeded || !self.targets.is_empty() || !self.unresolved_targets.is_empty()
    }

    pub fn validate(&self) -> Result<(), String> {
        let normalized = |path: &str| !path.is_empty() && path.len() <= 4096
            && path == path.trim() && !path.chars().any(char::is_control)
            && !path.contains(['\\', ':'])
            && path.split('/').all(|part| !matches!(part, "" | "." | ".."));
        if !matches!(self.version, 1 | 2)
            || (self.version == 1 && !self.unresolved_targets.is_empty())
            || self.targets.len() > 64 || self.unresolved_targets.len() > 64
            || self.targets.iter().chain(&self.unresolved_targets).map(String::len).sum::<usize>() > 32 * 1024
            || self.unresolved_before_input.is_some_and(|count| count == 0 || count > 32)
            || (self.unresolved_targets.is_empty() && self.unresolved_before_input.is_some())
            || self.targets.iter().collect::<BTreeSet<_>>().len() != self.targets.len()
            || self.unresolved_targets.iter().collect::<BTreeSet<_>>().len() != self.unresolved_targets.len()
            || self.targets.iter().chain(&self.unresolved_targets).any(|path| !normalized(path))
        { return Err("Invalid bounded patch recovery state".into()); }
        Ok(())
    }
}
