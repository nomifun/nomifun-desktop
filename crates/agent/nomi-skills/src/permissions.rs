use crate::types::SkillMetadata;

/// A parsed deny rule for skill name matching.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRule {
    /// Exact name match: `"commit"` matches only `"commit"`.
    Exact(String),
    /// Prefix match with trailing wildcard: `"db:*"` is stored as
    /// `Prefix("db:")`.
    Prefix(String),
}

impl PermissionRule {
    pub fn parse(rule: &str) -> Self {
        if let Some(prefix) = rule.strip_suffix('*') {
            Self::Prefix(prefix.to_string())
        } else {
            Self::Exact(rule.to_string())
        }
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            Self::Exact(exact) => exact == name,
            Self::Prefix(prefix) => name.starts_with(prefix),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillPermission {
    Allow,
    Deny,
}

/// Applies the only runtime Skill policy retained by FullAuto: explicit deny
/// rules block synchronously, and every other selected Skill executes.
pub struct SkillPermissionChecker {
    deny_rules: Vec<PermissionRule>,
}

impl SkillPermissionChecker {
    pub fn new(deny: Vec<String>) -> Self {
        Self {
            deny_rules: deny
                .iter()
                .map(|rule| PermissionRule::parse(rule))
                .collect(),
        }
    }

    pub fn check(&self, skill: &SkillMetadata) -> SkillPermission {
        if self
            .deny_rules
            .iter()
            .any(|rule| rule.matches(&skill.name))
        {
            SkillPermission::Deny
        } else {
            SkillPermission::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ExecutionContext, LoadedFrom, SkillMetadata, SkillSource};

    fn make_skill(name: &str) -> SkillMetadata {
        SkillMetadata {
            name: name.to_string(),
            display_name: None,
            description: String::new(),
            has_user_specified_description: false,
            allowed_tools: vec![],
            argument_hint: None,
            argument_names: vec![],
            when_to_use: None,
            version: None,
            model: None,
            disable_model_invocation: false,
            user_invocable: true,
            execution_context: ExecutionContext::Inline,
            agent: None,
            effort: None,
            paths: vec![],
            hooks_raw: None,
            source: SkillSource::User,
            loaded_from: LoadedFrom::Skills,
            content: String::new(),
            content_length: 0,
            skill_root: None,
        }
    }

    #[test]
    fn parses_exact_rule() {
        let rule = PermissionRule::parse("commit");
        assert_eq!(rule, PermissionRule::Exact("commit".to_string()));
        assert!(rule.matches("commit"));
        assert!(!rule.matches("commit-all"));
    }

    #[test]
    fn parses_prefix_rule_with_boundary() {
        let rule = PermissionRule::parse("db:*");
        assert_eq!(rule, PermissionRule::Prefix("db:".to_string()));
        assert!(rule.matches("db:migrate"));
        assert!(rule.matches("db:"));
        assert!(!rule.matches("db"));
        assert!(!rule.matches("database"));
    }

    #[test]
    fn explicit_deny_blocks() {
        let checker = SkillPermissionChecker::new(vec!["dangerous".to_string()]);
        assert_eq!(
            checker.check(&make_skill("dangerous")),
            SkillPermission::Deny
        );
    }

    #[test]
    fn non_denied_skill_runs_regardless_of_hooks_or_exact_tools() {
        let checker = SkillPermissionChecker::new(Vec::new());
        let mut skill = make_skill("full-auto");
        skill.hooks_raw = Some(serde_json::json!({"pre": "echo hi"}));
        skill.allowed_tools = vec!["Read".to_string()];
        assert_eq!(checker.check(&skill), SkillPermission::Allow);
    }

    #[test]
    fn deny_still_wins_for_skill_with_hooks_and_exact_tools() {
        let checker = SkillPermissionChecker::new(vec!["blocked:*".to_string()]);
        let mut skill = make_skill("blocked:task");
        skill.hooks_raw = Some(serde_json::json!({"pre": "echo hi"}));
        skill.allowed_tools = vec!["Bash".to_string()];
        assert_eq!(checker.check(&skill), SkillPermission::Deny);
    }
}
