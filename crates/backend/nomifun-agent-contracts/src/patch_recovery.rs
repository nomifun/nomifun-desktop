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
        // Recovery retains the exact observed local path. Resolution belongs
        // to the live file owner, never to historical/checkpoint decoding.
        // Native paths may contain spaces, parent segments, drive/UNC prefixes
        // and legal control characters; only NUL is not an OS path character.
        let valid_local_path = |path: &str| !path.is_empty() && path.len() <= 4096
            && !path.contains('\0');
        if !matches!(self.version, 1 | 2)
            || (self.version == 1 && !self.unresolved_targets.is_empty())
            || self.targets.len() > 64 || self.unresolved_targets.len() > 64
            || self.targets.iter().chain(&self.unresolved_targets).map(String::len).sum::<usize>() > 32 * 1024
            || self.unresolved_before_input.is_some_and(|count| count == 0 || count > 32)
            || (self.unresolved_targets.is_empty() && self.unresolved_before_input.is_some())
            || self.targets.iter().collect::<BTreeSet<_>>().len() != self.targets.len()
            || self.unresolved_targets.iter().collect::<BTreeSet<_>>().len() != self.unresolved_targets.len()
            || self.targets.iter().chain(&self.unresolved_targets).any(|path| !valid_local_path(path))
        { return Err("Invalid bounded patch recovery state".into()); }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_preserves_native_global_targets_without_resolving_history() {
        let targets = ["../outside/file.txt", "/Volumes/Test/file.txt", r"C:\Users\Test User\报告.txt", r"\\server\share\file.txt", " spaced name "];
        let state = AgentPatchRecoveryState { targets: targets.iter().map(|path| (*path).to_owned()).collect(), ..Default::default() };
        state.validate().unwrap();
        let decoded: AgentPatchRecoveryState = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(decoded, state);
        for invalid in ["", "bad\0path"] {
            assert!(AgentPatchRecoveryState { targets: vec![invalid.into()], ..Default::default() }.validate().is_err());
        }
        assert!(AgentPatchRecoveryState { targets: vec!["../same".into(), "../same".into()], ..Default::default() }.validate().is_err());
        assert!(AgentPatchRecoveryState { unresolved_before_input: Some(1), ..Default::default() }.validate().is_err());
    }
}
