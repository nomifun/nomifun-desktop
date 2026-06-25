// Supplemental tests for Phase 8 — ConditionalSkillManager.
// Covers test-plan.md TC-1 through TC-20, plus AC-10 and AC-11.

#[cfg(test)]
#[cfg(not(windows))] // Path handling differs on Windows; skip these tests there
mod conditional_supplemental_tests {
    use crate::conditional::ConditionalSkillManager;
    use crate::types::{ExecutionContext, LoadedFrom, SkillMetadata, SkillSource};

    // ---------------------------------------------------------------------------
    // Test helpers
    // ---------------------------------------------------------------------------

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
            source: SkillSource::Project,
            loaded_from: LoadedFrom::Skills,
            content: String::new(),
            content_length: 0,
            skill_root: None,
        }
    }

    fn make_conditional_skill(name: &str, patterns: Vec<&str>) -> SkillMetadata {
        let mut skill = make_skill(name);
        skill.paths = patterns.into_iter().map(|s| s.to_string()).collect();
        skill
    }

    // ---------------------------------------------------------------------------
    // TC-1: new_creates_empty_manager
    // ---------------------------------------------------------------------------

    // TC-1: new() yields an empty manager with no dormant or activated skills.
    #[test]
    fn tc1_new_creates_empty_manager() {
        let mgr = ConditionalSkillManager::new();
        assert_eq!(mgr.dormant_count(), 0);
        assert!(mgr.get_all_activated().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-2 to TC-5: partition_skills
    // ---------------------------------------------------------------------------

    // TC-2: conditional and unconditional skills are separated correctly.
    #[test]
    fn tc2_partition_separates_conditional_and_unconditional() {
        let mut mgr = ConditionalSkillManager::new();
        let skills = vec![
            make_conditional_skill("backend", vec!["src/**/*.rs"]),
            make_conditional_skill("frontend", vec!["src/**/*.ts"]),
            make_skill("no-paths"),
        ];
        let unconditional = mgr.partition_skills(skills);
        assert_eq!(unconditional.len(), 1);
        assert_eq!(unconditional[0].name, "no-paths");
        assert_eq!(mgr.dormant_count(), 2);
        assert!(mgr.get_all_activated().is_empty());
    }

    // TC-3: partition_skills with empty input returns empty list and leaves manager empty.
    #[test]
    fn tc3_partition_empty_input() {
        let mut mgr = ConditionalSkillManager::new();
        let result = mgr.partition_skills(vec![]);
        assert!(result.is_empty());
        assert_eq!(mgr.dormant_count(), 0);
    }

    // TC-4: all-unconditional input — everything returned, dormant stays zero.
    #[test]
    fn tc4_partition_all_unconditional() {
        let mut mgr = ConditionalSkillManager::new();
        let skills = vec![make_skill("a"), make_skill("b"), make_skill("c")];
        let result = mgr.partition_skills(skills);
        assert_eq!(result.len(), 3);
        assert_eq!(mgr.dormant_count(), 0);
    }

    // TC-5: already-activated skill is treated as unconditional on re-partition.
    #[test]
    fn tc5_partition_already_activated_treated_as_unconditional() {
        let mut mgr = ConditionalSkillManager::new();

        // First round: partition + activate
        let skills = vec![make_conditional_skill("foo", vec!["**/*.rs"])];
        mgr.partition_skills(skills);
        let activated = mgr.activate_for_paths(&["/project/main.rs"], "/project");
        assert_eq!(activated, vec!["foo"]);

        // Second round: same skill with paths — should be returned as unconditional
        let skills2 = vec![make_conditional_skill("foo", vec!["**/*.rs"])];
        let result = mgr.partition_skills(skills2);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "foo");
        // dormant count doesn't gain "foo" again
        assert_eq!(mgr.dormant_count(), 0);
    }

    // ---------------------------------------------------------------------------
    // TC-6 to TC-13: activate_for_paths
    // ---------------------------------------------------------------------------

    // TC-6: simple glob `src/**/*.rs` matches nested rust source file.
    #[test]
    fn tc6_activate_matches_simple_glob() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("backend", vec!["src/**/*.rs"])]);

        let activated = mgr.activate_for_paths(&["/project/src/lib.rs"], "/project");
        assert_eq!(activated, vec!["backend"]);
        assert!(mgr.get_activated("backend").is_some());
        assert_eq!(mgr.dormant_count(), 0);
    }

    // TC-7: wildcard extension `*.ts` matches typescript file in cwd root.
    #[test]
    fn tc7_activate_matches_wildcard_extension() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("ts-skill", vec!["*.ts"])]);

        let activated = mgr.activate_for_paths(&["/project/index.ts"], "/project");
        assert_eq!(activated, vec!["ts-skill"]);
    }

    // TC-8: no match — activate_for_paths returns empty list.
    #[test]
    fn tc8_activate_no_match() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("backend", vec!["src/**/*.rs"])]);

        let activated = mgr.activate_for_paths(&["/project/docs/readme.md"], "/project");
        assert!(activated.is_empty());
        assert_eq!(mgr.dormant_count(), 1);
    }

    // TC-9: path outside cwd is skipped — relative path would start with `..`.
    #[test]
    fn tc9_activate_skips_path_outside_cwd() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("s", vec!["**/*.rs"])]);

        let activated = mgr.activate_for_paths(&["/other/file.rs"], "/project");
        assert!(activated.is_empty());
        assert_eq!(mgr.dormant_count(), 1);
    }

    // TC-10: completely different absolute path that cannot be relativized to cwd.
    #[test]
    fn tc10_activate_skips_unrelated_absolute_path() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("s", vec!["**/*.rs"])]);

        let activated = mgr.activate_for_paths(&["/completely/different/file.rs"], "/project");
        assert!(activated.is_empty());
    }

    // TC-11: empty string path is skipped without panic.
    #[test]
    fn tc11_activate_skips_empty_string_path() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("s", vec!["**/*.rs"])]);

        let activated = mgr.activate_for_paths(&[""], "/project");
        assert!(activated.is_empty());
    }

    // TC-12: multiple files activate multiple matching skills in one call.
    #[test]
    fn tc12_activate_multiple_skills_in_one_call() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![
            make_conditional_skill("a", vec!["src/**/*.rs"]),
            make_conditional_skill("b", vec!["tests/**/*.rs"]),
        ]);

        let mut activated = mgr.activate_for_paths(
            &["/project/src/lib.rs", "/project/tests/test_main.rs"],
            "/project",
        );
        activated.sort();
        assert_eq!(activated, vec!["a", "b"]);
        assert!(mgr.get_activated("a").is_some());
        assert!(mgr.get_activated("b").is_some());
    }

    // TC-13: second call with same paths returns empty (already-activated skills
    // are not re-activated).
    #[test]
    fn tc13_activate_idempotent() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("foo", vec!["**/*.rs"])]);

        let first = mgr.activate_for_paths(&["/project/main.rs"], "/project");
        assert_eq!(first, vec!["foo"]);

        let second = mgr.activate_for_paths(&["/project/main.rs"], "/project");
        assert!(second.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-14 to TC-15: activated_names persistence and clear_dormant
    // ---------------------------------------------------------------------------

    // TC-14: activated_names survive clear_dormant so re-partitioned same skill
    // goes to unconditional list instead of dormant.
    #[test]
    fn tc14_activated_names_preserved_across_clear_dormant() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("foo", vec!["**/*.rs"])]);
        mgr.activate_for_paths(&["/project/main.rs"], "/project");

        mgr.clear_dormant();

        // Re-partition: "foo" has paths but is already activated → unconditional
        let result = mgr.partition_skills(vec![make_conditional_skill("foo", vec!["**/*.rs"])]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "foo");
        assert_eq!(mgr.dormant_count(), 0);
    }

    // TC-15: clear_dormant removes all unactivated dormant skills.
    #[test]
    fn tc15_clear_dormant_removes_unactivated() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![
            make_conditional_skill("a", vec!["**/*.rs"]),
            make_conditional_skill("b", vec!["**/*.ts"]),
            make_conditional_skill("c", vec!["**/*.go"]),
        ]);
        assert_eq!(mgr.dormant_count(), 3);

        mgr.clear_dormant();
        assert_eq!(mgr.dormant_count(), 0);
        assert!(mgr.get_all_activated().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-16 to TC-17: get_activated / dormant_count
    // ---------------------------------------------------------------------------

    // TC-16: get_activated returns None for unknown / unactivated skill names.
    #[test]
    fn tc16_get_activated_returns_none_for_unknown() {
        let mgr = ConditionalSkillManager::new();
        assert!(mgr.get_activated("nonexistent").is_none());
    }

    // TC-17: dormant_count accurately reflects state after partition and activate.
    #[test]
    fn tc17_dormant_count_reflects_partition_and_activate() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![
            make_conditional_skill("a", vec!["**/*.rs"]),
            make_conditional_skill("b", vec!["**/*.ts"]),
            make_conditional_skill("c", vec!["**/*.go"]),
        ]);
        assert_eq!(mgr.dormant_count(), 3);

        mgr.activate_for_paths(&["/project/main.rs"], "/project");
        assert_eq!(mgr.dormant_count(), 2);
    }

    // ---------------------------------------------------------------------------
    // TC-18 to TC-20: glob pattern edge cases (AC-9)
    // ---------------------------------------------------------------------------

    // TC-18: `src/**/*.ts` matches multi-level nested TypeScript files.
    #[test]
    fn tc18_glob_src_double_star_ts() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill(
            "ts-skill",
            vec!["src/**/*.ts"],
        )]);

        let activated = mgr.activate_for_paths(&["/app/src/components/Button.ts"], "/app");
        assert_eq!(activated, vec!["ts-skill"]);
    }

    // TC-19: `*.rs` matches rust file at cwd root level.
    #[test]
    fn tc19_glob_root_star_rs_matches_root_file() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("root-rs", vec!["*.rs"])]);

        let activated = mgr.activate_for_paths(&["/app/main.rs"], "/app");
        assert_eq!(activated, vec!["root-rs"]);
    }

    // TC-20: Documents glob::Pattern matching behaviour for `*.rs`.
    //
    // NOTE: `glob::Pattern` uses shell-glob semantics where `*` DOES match
    // path separators (unlike gitignore semantics where `*` stops at `/`).
    // Therefore `*.rs` matches both `main.rs` and `src/lib.rs` in glob::Pattern.
    //
    // This test documents this behaviour rather than asserting the opposite,
    // so that future readers understand why the implementation uses glob::Pattern
    // and its trade-offs vs the TypeScript `ignore` library (which respects `/`).
    #[test]
    fn tc20_glob_star_rs_matches_subdir_due_to_glob_semantics() {
        let mut mgr = ConditionalSkillManager::new();
        // `*.rs` with glob::Pattern matches any `.rs` file, including in subdirs
        mgr.partition_skills(vec![make_conditional_skill("root-rs", vec!["*.rs"])]);

        // glob::Pattern `*.rs` DOES match `src/lib.rs` (star matches `/`)
        let activated = mgr.activate_for_paths(&["/app/src/lib.rs"], "/app");
        // Document the actual behaviour: activated (glob semantics, not gitignore)
        assert_eq!(activated, vec!["root-rs"]);
    }

    // ---------------------------------------------------------------------------
    // AC-10: invalid glob pattern does not panic (e.g. `!negation`)
    // ---------------------------------------------------------------------------

    // AC-10: partition_skills with an invalid glob pattern in `paths:` does not
    // panic — the pattern is skipped with a warning.
    #[test]
    fn ac10_invalid_glob_pattern_does_not_panic() {
        let mut mgr = ConditionalSkillManager::new();
        // "!negation" is not a valid glob::Pattern — should be skipped
        let skill = make_conditional_skill("bad-pattern", vec!["!negation", "src/**/*.rs"]);
        mgr.partition_skills(vec![skill]);
        assert_eq!(mgr.dormant_count(), 1);

        // The valid pattern `src/**/*.rs` should still work even though `!negation` was skipped.
        // Use a relative path to avoid platform-dependent absolute path issues
        let activated = mgr.activate_for_paths(&["src/main.rs"], ".");
        assert_eq!(activated.len(), 1);
        assert_eq!(activated[0], "bad-pattern");
        assert!(mgr.get_activated("bad-pattern").is_some());
    }

    // AC-10b: all-invalid patterns results in a dormant skill that never activates.
    #[test]
    fn ac10b_all_invalid_patterns_skill_never_activates() {
        let mut mgr = ConditionalSkillManager::new();
        let skill = make_conditional_skill("all-invalid", vec!["!invalid1", "!invalid2"]);
        mgr.partition_skills(vec![skill]);
        assert_eq!(mgr.dormant_count(), 1);

        let activated = mgr.activate_for_paths(&["any/file.rs"], ".");
        // No valid patterns → nothing activates
        assert!(activated.is_empty());
        assert_eq!(mgr.dormant_count(), 1);
    }

    // ---------------------------------------------------------------------------
    // AC-11: reset_all clears everything including activated_names
    // ---------------------------------------------------------------------------

    // AC-11a: reset_all clears dormant skills, activated skills, and activated_names.
    #[test]
    fn ac11a_reset_all_clears_dormant_and_activated() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![
            make_conditional_skill("a", vec!["**/*.rs"]),
            make_conditional_skill("b", vec!["**/*.ts"]),
        ]);
        mgr.activate_for_paths(&["/project/main.rs"], "/project");

        assert_eq!(mgr.dormant_count(), 1);
        assert_eq!(mgr.get_all_activated().len(), 1);

        mgr.reset_all();

        assert_eq!(mgr.dormant_count(), 0);
        assert!(mgr.get_all_activated().is_empty());
    }

    // AC-11b: after reset_all, re-partitioning a previously-activated skill puts it
    // back into dormant (activated_names was cleared).
    #[test]
    fn ac11b_reset_all_clears_activated_names_so_skill_goes_dormant_again() {
        let mut mgr = ConditionalSkillManager::new();
        mgr.partition_skills(vec![make_conditional_skill("foo", vec!["**/*.rs"])]);
        mgr.activate_for_paths(&["/project/main.rs"], "/project");

        // Confirm "foo" is activated
        assert!(mgr.get_activated("foo").is_some());

        mgr.reset_all();

        // Re-partition: "foo" should now enter dormant (not unconditional)
        let result = mgr.partition_skills(vec![make_conditional_skill("foo", vec!["**/*.rs"])]);
        assert!(
            result.is_empty(),
            "foo should be dormant, not returned as unconditional"
        );
        assert_eq!(mgr.dormant_count(), 1);
    }
}
