// Supplemental tests for Phase 5 permission system.
// Covers test-plan.md cases not present in the existing impl tests:
//   TC-P5-21: prefix deny rule matches all skills in a namespace, but not bare names without colon
//   TC-P5-22: PermissionRule::parse("") does not panic — treats empty string as Exact
//   TC-P5-23: PermissionRule::parse(":*") does not panic — Prefix with empty prefix

#[cfg(test)]
#[allow(clippy::module_inception)]
mod permissions_supplemental_tests {
    use crate::permissions::{PermissionRule, SkillPermission, SkillPermissionChecker};
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
            shell: None,
            paths: vec![],
            hooks_raw: None,
            source: SkillSource::User,
            loaded_from: LoadedFrom::Skills,
            content: String::new(),
            content_length: 0,
            skill_root: None,
        }
    }

    // TC-P5-21: prefix deny rule blocks all skills in a namespace,
    // but safe skills whose names lack the colon-prefix are still allowed.
    #[test]
    fn tc_p5_21_prefix_deny_blocks_namespace_but_not_bare_names() {
        let checker = SkillPermissionChecker::new(vec!["admin:*".to_string()], vec![], false);

        // "admin:create-user" matches "admin:*" → Deny
        let create_user = make_skill("admin:create-user");
        assert_eq!(checker.check(&create_user), SkillPermission::Deny);

        // "admin:delete-all" matches "admin:*" → Deny
        let delete_all = make_skill("admin:delete-all");
        assert_eq!(checker.check(&delete_all), SkillPermission::Deny);

        // "admins" does NOT match "admin:*" (no colon separator) and has no hooks/tools → Allow
        let admins = make_skill("admins");
        assert_eq!(checker.check(&admins), SkillPermission::Allow);

        // "admin" alone does NOT match "admin:*" → Allow via safe-properties
        let admin = make_skill("admin");
        assert_eq!(checker.check(&admin), SkillPermission::Allow);
    }

    // TC-P5-22: parse("") does not panic.
    // An empty rule string contains no ":*" suffix, so it becomes Exact("").
    #[test]
    fn tc_p5_22_parse_empty_string_does_not_panic() {
        let rule = PermissionRule::parse("");
        // Should produce an Exact rule with empty string
        assert_eq!(rule, PermissionRule::Exact("".to_string()));
        // Exact("") only matches the empty-string name
        assert!(rule.matches(""));
        assert!(!rule.matches("anything"));
    }

    // TC-P5-23: parse(":*") does not panic.
    // ":*" ends with "*", so strip_suffix('*') leaves ":", stored as Prefix(":").
    // Behaviour: matches any name that starts_with(":") — unusual but must not panic.
    #[test]
    fn tc_p5_23_parse_colon_star_does_not_panic() {
        let rule = PermissionRule::parse(":*");
        // Should produce a Prefix rule; the exact stored value is ":"
        assert_eq!(rule, PermissionRule::Prefix(":".to_string()));
        // A name starting with ":" would match
        assert!(rule.matches(":something"));
        // An ordinary name without leading ":" does not match
        assert!(!rule.matches("something"));
        assert!(!rule.matches(""));
    }
}
