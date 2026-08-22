use std::collections::HashSet;
use std::io::{self, BufRead, Write};

pub struct ToolConfirmer {
    auto_approve: bool,
    /// Grants that belong to the running turn's runtime authority: the config
    /// baseline plus whatever a skill context modifier added mid-turn. A
    /// rejected accepted turn rolls these back to its captured root.
    allow_list: HashSet<String>,
    /// Tools the operator personally approved with "always" at an interactive
    /// prompt. This is a first-party human decision, not something the model
    /// granted itself, so an accepted-turn rollback must not revoke it.
    user_always: HashSet<String>,
    #[cfg(test)]
    check_count: usize,
}

/// The confirmation authority an accepted turn may mutate, captured before its
/// first provider await. Operator "always" grants are deliberately absent: a
/// rollback never touches them, so there is nothing to restore.
#[derive(Debug, Clone)]
pub struct ToolConfirmerAuthority {
    auto_approve: bool,
    allow_list: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResult {
    Approved,
    Denied,
    Quit,
}

impl ToolConfirmer {
    pub fn new(auto_approve: bool, allow_list: Vec<String>) -> Self {
        Self {
            auto_approve,
            allow_list: allow_list.into_iter().collect(),
            user_always: HashSet::new(),
            #[cfg(test)]
            check_count: 0,
        }
    }

    /// Returns whether auto-approve is enabled
    pub fn is_auto_approve(&self) -> bool {
        self.auto_approve
    }

    /// Add a tool name to the allow list at runtime.
    /// Used by skill context modifiers to grant auto-approval for specified tools.
    pub fn add_to_allow_list(&mut self, name: &str) {
        self.allow_list.insert(name.to_string());
    }

    /// Capture the turn-owned confirmation authority for accepted-turn rollback.
    pub fn authority_snapshot(&self) -> ToolConfirmerAuthority {
        ToolConfirmerAuthority {
            auto_approve: self.auto_approve,
            allow_list: self.allow_list.clone(),
        }
    }

    /// Restore the turn-owned confirmation authority, dropping every grant a
    /// rejected turn added. Operator "always" decisions are preserved: the
    /// rollback retracts the assistant's provisional turn, not a human choice.
    pub fn restore_authority(&mut self, authority: &ToolConfirmerAuthority) {
        self.auto_approve = authority.auto_approve;
        self.allow_list = authority.allow_list.clone();
    }

    /// Check if the tool needs confirmation. Returns the user's decision.
    pub fn check(&mut self, tool_name: &str, tool_input_display: &str) -> ConfirmResult {
        #[cfg(test)]
        {
            self.check_count += 1;
        }

        if self.auto_approve
            || self.allow_list.contains(tool_name)
            || self.user_always.contains(tool_name)
        {
            return ConfirmResult::Approved;
        }

        eprint!(
            "\n[tool] {}({})\nAllow? [y]es / [n]o / [a]lways / [q]uit > ",
            tool_name, tool_input_display
        );
        io::stderr().flush().unwrap();

        let mut input = String::new();
        if io::stdin().lock().read_line(&mut input).is_err() {
            return ConfirmResult::Denied;
        }

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" | "" => ConfirmResult::Approved,
            "a" | "always" => {
                // An explicit operator decision, recorded separately from the
                // turn's grants so a rejected turn cannot revoke it.
                self.user_always.insert(tool_name.to_string());
                ConfirmResult::Approved
            }
            "q" | "quit" => ConfirmResult::Quit,
            _ => ConfirmResult::Denied,
        }
    }

    #[cfg(test)]
    pub(crate) fn check_count(&self) -> usize {
        self.check_count
    }

    /// Record the same grant the interactive `[a]lways` answer records. The
    /// production path is `check()`, which cannot run without a terminal.
    #[cfg(test)]
    pub(crate) fn grant_user_always(&mut self, name: &str) {
        self.user_always.insert(name.to_string());
    }

    /// Whether `check()` would approve without prompting. Tests use this to
    /// observe grant provenance without driving stdin.
    #[cfg(test)]
    pub(crate) fn allows_without_prompt(&self, name: &str) -> bool {
        self.auto_approve || self.allow_list.contains(name) || self.user_always.contains(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auto_approve_always_allows() {
        let mut confirmer = ToolConfirmer::new(true, vec![]);
        assert_eq!(
            confirmer.check("Bash", "echo hello"),
            ConfirmResult::Approved
        );
        assert_eq!(
            confirmer.check("Read", "/tmp/file"),
            ConfirmResult::Approved
        );
        assert_eq!(
            confirmer.check("Write", "/tmp/out"),
            ConfirmResult::Approved
        );
    }

    #[test]
    fn test_allowlist_contains_tool() {
        let mut confirmer = ToolConfirmer::new(false, vec!["Read".into(), "Write".into()]);
        assert_eq!(
            confirmer.check("Read", "/tmp/file"),
            ConfirmResult::Approved
        );
        assert_eq!(
            confirmer.check("Write", "/tmp/out"),
            ConfirmResult::Approved
        );
    }

    #[test]
    fn test_allowlist_approves_even_when_auto_approve_is_false() {
        let mut confirmer = ToolConfirmer::new(false, vec!["Read".into()]);
        assert_eq!(
            confirmer.check("Read", "/some/path"),
            ConfirmResult::Approved
        );
    }

    // Phase 6: add_to_allow_list() grants runtime approval
    #[test]
    fn test_add_to_allow_list_grants_approval() {
        let mut confirmer = ToolConfirmer::new(false, vec![]);
        // Before: tool not in list (would prompt — skip interactive check, just verify membership)
        confirmer.add_to_allow_list("Write");
        // After: auto-approved without interactive prompt
        assert_eq!(
            confirmer.check("Write", "file.txt"),
            ConfirmResult::Approved
        );
    }

    // Phase 6: add_to_allow_list() is idempotent — adding twice has no bad effect
    #[test]
    fn test_add_to_allow_list_idempotent() {
        let mut confirmer = ToolConfirmer::new(false, vec![]);
        confirmer.add_to_allow_list("Bash");
        confirmer.add_to_allow_list("Bash"); // duplicate — HashSet, no panic
        assert_eq!(confirmer.check("Bash", "echo hi"), ConfirmResult::Approved);
    }

    // Phase 6: add_to_allow_list() does not affect unrelated tools
    #[test]
    fn test_add_to_allow_list_does_not_affect_other_tools() {
        let mut confirmer = ToolConfirmer::new(false, vec![]);
        confirmer.add_to_allow_list("Read");
        // Write is not in the list — check returns non-Approved for non-interactive
        // (we cannot test interactive input; verify Read is approved and Write is not in list)
        assert_eq!(confirmer.check("Read", "file.txt"), ConfirmResult::Approved);
        // We can't test the Denied path without stdin, but we verify allow_list state:
        assert!(confirmer.allow_list.contains("Read"));
        assert!(!confirmer.allow_list.contains("Write"));
    }

    // An accepted-turn rollback retracts skill grants without touching the
    // operator's own "always" decision.
    #[test]
    fn restoring_authority_drops_skill_grants_and_keeps_user_always() {
        let mut confirmer = ToolConfirmer::new(false, vec!["Read".into()]);
        let root = confirmer.authority_snapshot();

        confirmer.grant_user_always("Bash");
        confirmer.add_to_allow_list("Write");

        confirmer.restore_authority(&root);

        assert_eq!(confirmer.check("Read", "f"), ConfirmResult::Approved);
        assert_eq!(
            confirmer.check("Bash", "echo hi"),
            ConfirmResult::Approved,
            "an explicit operator always grant survives the rollback"
        );
        assert!(
            !confirmer.allow_list.contains("Write"),
            "a skill grant from the rejected turn must be revoked"
        );
    }

    // The snapshot is a value, so a later grant cannot leak into it.
    #[test]
    fn authority_snapshot_is_independent_of_later_grants() {
        let mut confirmer = ToolConfirmer::new(false, vec![]);
        let root = confirmer.authority_snapshot();
        confirmer.add_to_allow_list("Bash");
        assert!(root.allow_list.is_empty());

        confirmer.restore_authority(&root);
        assert!(!confirmer.allow_list.contains("Bash"));
    }
}
