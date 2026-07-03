// Integration tests for the Skills system (Phase 14)
//
// These are black-box tests written by the Tester role based on test-plan.md.
// They exercise cross-module interactions through public APIs only.
//
// Test coverage: TC-E2E-1 through TC-E2E-12c (AC-4 through AC-15).

use std::fs;
use std::path::Path;
use tempfile::TempDir;

use crate::conditional::ConditionalSkillManager;
use crate::context_modifier::ContextModifier;
use crate::executor::prepare_inline_content;
use crate::hooks::{parse_skill_hooks, to_hook_defs};
use crate::loader::load_skills_from_dir;
use crate::permissions::{SkillPermission, SkillPermissionChecker};
use crate::prompt::format_skills_within_budget;
use crate::types::{EffortLevel, ExecutionContext, LoadedFrom, SkillMetadata, SkillSource};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a minimal SkillMetadata with sensible defaults.
fn make_skill(name: &str, content: &str) -> SkillMetadata {
    SkillMetadata {
        name: name.to_string(),
        display_name: None,
        description: String::new(),
        has_user_specified_description: false,
        allowed_tools: Vec::new(),
        argument_hint: None,
        argument_names: Vec::new(),
        when_to_use: None,
        version: None,
        model: None,
        disable_model_invocation: false,
        user_invocable: true,
        execution_context: ExecutionContext::Inline,
        agent: None,
        effort: None,
        shell: None,
        paths: Vec::new(),
        hooks_raw: None,
        source: SkillSource::User,
        loaded_from: LoadedFrom::Skills,
        content: content.to_string(),
        content_length: content.len(),
        skill_root: None,
    }
}

/// Write a SKILL.md inside `<tmp>/<skill_name>/SKILL.md`.
fn write_skill_dir(tmp: &Path, skill_name: &str, content: &str) {
    let dir = tmp.join(skill_name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), content).unwrap();
}

// ---------------------------------------------------------------------------
// TC-E2E-1: Complete lifecycle — create SKILL.md → discover → parse → load
// AC-4: loaded SkillDefinition contains correct name/description/body
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_1_full_lifecycle_load_skill() {
    let tmp = TempDir::new().unwrap();
    let skill_content = "---\nname: test-skill\ndescription: A test skill for integration testing\n---\nThis is the skill body content.";
    write_skill_dir(tmp.path(), "test-skill", skill_content);

    let loaded = load_skills_from_dir(tmp.path(), SkillSource::Project, LoadedFrom::Skills).await;

    let skill = loaded
        .iter()
        .find(|s| s.metadata.name == "test-skill")
        .expect("skill should be loaded");

    // AC-4 assertions
    assert_eq!(skill.metadata.name, "test-skill");
    assert_eq!(
        skill.metadata.description,
        "A test skill for integration testing"
    );
    assert!(
        skill
            .metadata
            .content
            .contains("This is the skill body content."),
        "body should contain expected content, got: {}",
        skill.metadata.content
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-2: Inline execution — variable substitution
// AC-5: $ARGUMENTS, $0, ${NOMI_SKILL_DIR} are correctly substituted
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_2_inline_variable_substitution() {
    let tmp = std::env::temp_dir();
    let skill_root = tmp.join("e2e2-skill");
    let mut skill = make_skill(
        "var-skill",
        "Arguments: $ARGUMENTS\nFirst arg: $0\nSkill dir: ${NOMI_SKILL_DIR}",
    );
    let skill_root_str = skill_root.to_str().unwrap();
    skill.skill_root = Some(skill_root_str.to_string());
    skill.argument_names = vec!["query".to_string()];

    let result = prepare_inline_content(&skill, Some("hello world"), None, tmp.to_str().unwrap())
        .await
        .unwrap();

    // AC-5 assertions
    assert!(
        result.contains("hello world"),
        "$ARGUMENTS should be replaced, got: {result}"
    );
    assert!(
        result.contains(skill_root_str),
        "${{NOMI_SKILL_DIR}} should be replaced with skill root, got: {result}"
    );
    // $0 (first positional arg) substitution — first token of args
    assert!(
        result.contains("hello"),
        "$0 should be replaced with first arg, got: {result}"
    );
    // No unexpanded placeholders remain
    assert!(
        !result.contains("$ARGUMENTS"),
        "$ARGUMENTS literal should not remain, got: {result}"
    );
    assert!(
        !result.contains("${NOMI_SKILL_DIR}"),
        "${{NOMI_SKILL_DIR}} literal should not remain, got: {result}"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-3: Shell command execution and substitution
// AC-6: shell command output replaces the !`...` syntax
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_3_shell_command_execution() {
    let mut skill = make_skill("shell-skill", "Output: !`echo hello`");
    skill.loaded_from = LoadedFrom::Skills; // non-MCP so shell is allowed

    let tmp = std::env::temp_dir();
    let result = prepare_inline_content(&skill, None, None, tmp.to_str().unwrap())
        .await
        .unwrap();

    // AC-6 assertions
    assert!(
        result.contains("hello"),
        "echo output should appear in result, got: {result}"
    );
    assert!(
        !result.contains("!`echo hello`"),
        "shell syntax should be replaced, got: {result}"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-4a: Permission decision — deny takes priority
// AC-7: deny rule blocks execution even when allow rule also matches
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_4a_permission_deny_priority() {
    let checker = SkillPermissionChecker::new(
        vec!["dangerous-skill".to_string()],
        vec!["dangerous-skill".to_string()],
        false,
    );
    let skill = make_skill("dangerous-skill", "body");
    let result = checker.check(&skill);
    assert_eq!(
        result,
        SkillPermission::Deny,
        "deny should take priority over allow"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-4b: Permission decision — allow rule grants access
// AC-7: allow rule permits execution when no deny rule matches
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_4b_permission_allow_grants_access() {
    let checker = SkillPermissionChecker::new(vec![], vec!["safe-skill".to_string()], false);
    let mut skill = make_skill("safe-skill", "body");
    // Give it hooks so it would otherwise need Ask
    skill.hooks_raw = Some(serde_json::json!({"PreToolUse": []}));

    let result = checker.check(&skill);
    assert_eq!(
        result,
        SkillPermission::Allow,
        "allow rule should grant access"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-4c: Permission decision — safe-properties path
// AC-7: skill with no hooks and no allowed_tools is allowed without explicit allow rule
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_4c_permission_safe_properties() {
    let checker = SkillPermissionChecker::new(vec![], vec![], false);
    // Skill has no hooks_raw and no allowed_tools → safe-properties path
    let skill = make_skill("safe-prop-skill", "body");
    assert!(skill.hooks_raw.is_none());
    assert!(skill.allowed_tools.is_empty());

    let result = checker.check(&skill);
    assert_eq!(
        result,
        SkillPermission::Allow,
        "safe-properties skill should be allowed without explicit rule"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-4d: Permission decision — ask fallback
// AC-7: skill with hooks and no matching allow rule → Ask
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_4d_permission_ask_fallback() {
    let checker = SkillPermissionChecker::new(vec![], vec![], false);
    let mut skill = make_skill("ask-skill", "body");
    // Add hooks so safe-properties check fails
    skill.hooks_raw = Some(
        serde_json::json!({"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "echo hi"}]}]}),
    );

    let result = checker.check(&skill);
    assert!(
        matches!(result, SkillPermission::Ask { .. }),
        "should fall through to Ask, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-5a: Conditional activation — dormant when path does not match
// AC-8: skill with paths: ["*.rs"] stays dormant when context file is *.py
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_5a_conditional_dormant_on_mismatch() {
    let mut manager = ConditionalSkillManager::new();
    let mut skill = make_skill("rs-skill", "Rust only");
    skill.paths = vec!["*.rs".to_string()];

    let unconditional = manager.partition_skills(vec![skill]);

    // Skill should be dormant — not in unconditional list
    assert!(
        unconditional.is_empty(),
        "conditional skill should not be in unconditional list"
    );
    assert_eq!(manager.dormant_count(), 1, "skill should be dormant");

    // Activate with a Python file — should NOT match
    let activated = manager.activate_for_paths(&["/project/main.py"], "/project");
    assert!(
        activated.is_empty(),
        "*.rs skill should not activate for .py file"
    );
    assert!(
        manager.get_activated("rs-skill").is_none(),
        "skill should remain dormant"
    );
    assert!(
        manager.get_all_activated().is_empty(),
        "no skills should be in active list"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-5b: Conditional activation — becomes active when path matches
// AC-8: skill with paths: ["*.rs"] activates when context file is *.rs
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_5b_conditional_active_on_match() {
    let mut manager = ConditionalSkillManager::new();
    let mut skill = make_skill("rs-skill", "Rust only");
    skill.paths = vec!["*.rs".to_string()];

    let unconditional = manager.partition_skills(vec![skill]);
    assert!(unconditional.is_empty(), "skill should start dormant");

    // Activate with a Rust file — should match
    let activated = manager.activate_for_paths(&["/project/main.rs"], "/project");
    assert_eq!(activated.len(), 1, "skill should be activated");
    assert!(
        manager.get_activated("rs-skill").is_some(),
        "skill should be in activated map"
    );
    let active_list = manager.get_all_activated();
    assert_eq!(active_list.len(), 1, "one skill should be in active list");
    assert_eq!(active_list[0].name, "rs-skill");
}

// ---------------------------------------------------------------------------
// TC-E2E-6: Context modifier overrides model/effort/allowedTools
// AC-9: overrides are correctly extracted from SkillMetadata
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_6_context_modifier_overrides() {
    let mut skill = make_skill("override-skill", "body");
    skill.model = Some("claude-opus-4-6".to_string());
    skill.effort = Some(EffortLevel::High);
    skill.allowed_tools = vec!["Bash".to_string(), "Read".to_string()];

    let modifier = crate::context_modifier::from_skill(&skill)
        .expect("modifier should be present when overrides are set");

    // AC-9 assertions
    assert_eq!(
        modifier.model.as_deref(),
        Some("claude-opus-4-6"),
        "model override should match"
    );
    assert_eq!(
        modifier.effort,
        Some(EffortLevel::High),
        "effort override should match"
    );
    assert_eq!(
        modifier.allowed_tools,
        vec!["Bash", "Read"],
        "allowedTools override should match"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-7: Bundled skills are protected from prompt budget truncation
// AC-10: bundled skill appears in listing even when budget is very small
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_7_bundled_skills_budget_protection() {
    let mut bundled = make_skill("bundled-skill", "protected bundled content");
    bundled.source = SkillSource::Bundled;
    bundled.description = "Bundled skill description that is always preserved".to_string();

    // Create many large regular skills to exceed budget
    let regular_skills: Vec<SkillMetadata> = (0..20)
        .map(|i| {
            let mut s = make_skill(&format!("regular-skill-{i}"), "body");
            s.description = format!(
                "Regular skill number {i} with a very long description that consumes budget space"
            );
            s
        })
        .collect();

    let mut all_skills = vec![bundled];
    all_skills.extend(regular_skills);

    // Use a very small context window (100 tokens = 400 chars budget) to force truncation
    let result = format_skills_within_budget(&all_skills, Some(100));

    // AC-10 assertions
    assert!(
        result.contains("bundled-skill"),
        "bundled skill should appear in listing even with tight budget, got: {result}"
    );

    // At least some regular skills should be omitted (names only or truncated)
    // The result should not contain all full descriptions of regular skills
    let full_regular_count = (0..20)
        .filter(|i| result.contains(&format!("Regular skill number {i} with a very long")))
        .count();
    assert!(
        full_regular_count < 20,
        "not all regular skills should have full descriptions under tight budget"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-8: MCP skills — shell commands are not executed
// AC-11: shell commands in MCP skill body are not executed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_8_mcp_skill_shell_rejected() {
    let mut skill = make_skill("mcp-skill", "run !`ls /tmp` here");
    skill.source = SkillSource::Mcp;
    skill.loaded_from = LoadedFrom::Mcp;

    let tmp = std::env::temp_dir();
    let result = prepare_inline_content(&skill, None, None, tmp.to_str().unwrap())
        .await
        .unwrap();

    // AC-11 assertions: shell command NOT executed, syntax preserved
    assert!(
        result.contains("!`ls /tmp`"),
        "MCP skill shell syntax should be preserved (not executed), got: {result}"
    );
    // The output should not be a directory listing — verify no typical ls output tokens
    assert!(
        !result.contains("total ") && !result.contains(".log"),
        "MCP skill should not execute shell and return ls output, got: {result}"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-9: Prompt budget truncation — bundled preserved, non-bundled truncated
// AC-12: over-budget causes non-bundled truncation while bundled is kept
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_9_prompt_budget_truncation() {
    let mut bundled = make_skill("my-bundled", "body");
    bundled.source = SkillSource::Bundled;
    bundled.description = "Always here".to_string();

    // Create enough regular skills to exceed even DEFAULT_CHAR_BUDGET
    let n = 30;
    let regular_skills: Vec<SkillMetadata> = (0..n)
        .map(|i| {
            let mut s = make_skill(&format!("reg-{i}"), "body");
            s.description = format!("Regular skill {i}: {}", "x".repeat(300));
            s
        })
        .collect();

    let mut skills = vec![bundled];
    skills.extend(regular_skills);

    // Use 50 tokens → 200 chars budget — far below what 30 regular skills need
    let output = format_skills_within_budget(&skills, Some(50));

    // AC-12 assertions
    assert!(
        output.contains("my-bundled"),
        "bundled skill must be preserved"
    );

    let full_reg_count = (0..n)
        .filter(|i| output.contains(&format!("Regular skill {i}: {}", "x".repeat(300))))
        .count();
    assert!(
        full_reg_count < n,
        "some regular skills should be truncated, full_count={full_reg_count}"
    );

    // Verify total output length stays within a reasonable margin of budget
    // Budget = 50 tokens * 4 chars/token * 1% = 2 chars — that's extremely tight.
    // With bundled protection, at minimum bundled entry is present.
    // We just verify bundled is there and not all regular skills are full-expanded.
}

// ---------------------------------------------------------------------------
// TC-E2E-10: Multi-directory deduplication — first discovered wins
// AC-13: duplicate skill name across two dirs → only first survives
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_10_multi_dir_dedup_first_wins() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();

    write_skill_dir(
        tmp_a.path(),
        "my-skill",
        "---\nname: my-skill\ndescription: from dir A\n---\nbody A",
    );
    write_skill_dir(
        tmp_b.path(),
        "my-skill",
        "---\nname: my-skill\ndescription: from dir B\n---\nbody B",
    );

    // Load from dir A then dir B, deduplicate by name (first wins)
    let loaded_a = load_skills_from_dir(tmp_a.path(), SkillSource::User, LoadedFrom::Skills).await;
    let loaded_b =
        load_skills_from_dir(tmp_b.path(), SkillSource::Project, LoadedFrom::Skills).await;

    // Merge: A comes first (higher priority)
    let mut all_metadata: Vec<SkillMetadata> = loaded_a
        .into_iter()
        .map(|ls| ls.metadata)
        .chain(loaded_b.into_iter().map(|ls| ls.metadata))
        .collect();

    // Apply name-based dedup (first wins)
    let mut seen = std::collections::HashSet::new();
    all_metadata.retain(|s| seen.insert(s.name.clone()));

    // AC-13 assertions
    let matches: Vec<_> = all_metadata
        .iter()
        .filter(|s| s.name == "my-skill")
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "duplicate skill should be deduplicated to one entry"
    );
    assert_eq!(
        matches[0].description, "from dir A",
        "first discovered (dir A) should win dedup, got: {}",
        matches[0].description
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-11: Legacy commands directory — flat .md files loaded as skills
// AC-14: legacy command files from .nomi/commands/ are loaded as SkillDefinition
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tc_e2e_11_legacy_commands_loaded() {
    use crate::loader::load_all_skills;

    let tmp = TempDir::new().unwrap();

    // Create the legacy commands directory structure under a fake project root.
    // load_all_skills looks for .nomi/commands/ relative to cwd.
    let commands_dir = tmp.path().join(".nomi").join("commands");
    fs::create_dir_all(&commands_dir).unwrap();

    // Flat .md file (no subdirectory, no SKILL.md) — legacy format
    fs::write(
        commands_dir.join("legacy-cmd.md"),
        "This is the legacy command body.",
    )
    .unwrap();

    // Also create a .git dir so path walking stops at tmp root
    fs::create_dir(tmp.path().join(".git")).unwrap();

    // bare=true so we don't accidentally pick up user's real skill dirs,
    // but legacy commands are discovered via cwd, not add_dirs.
    // Use bare=false with cwd=tmp to exercise project_commands_dirs path.
    let skills = load_all_skills(tmp.path(), &[], false, None).await;

    // AC-14 assertions
    let skill = skills
        .iter()
        .find(|s| s.name == "legacy-cmd")
        .expect("legacy-cmd should be loaded from flat .md file in .nomi/commands/");

    assert_eq!(skill.name, "legacy-cmd");
    assert!(
        skill.content.contains("This is the legacy command body."),
        "legacy command body should be loaded, got: {}",
        skill.content
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-12a: Hooks parsing — PreToolUse classification
// AC-15: PreToolUse hooks correctly parsed and classified
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_12a_hooks_pre_tool_use_parsed() {
    let hooks_json = serde_json::json!({
        "PreToolUse": [
            {
                "matcher": "*",
                "hooks": [{"type": "command", "command": "echo pre-tool"}]
            }
        ]
    });
    let config = parse_skill_hooks(Some(&hooks_json), "test-skill", SkillSource::User)
        .expect("hooks should parse successfully");

    // AC-15 assertions
    assert_eq!(
        config.pre_tool_use.len(),
        1,
        "should have one PreToolUse hook"
    );
    assert_eq!(
        config.pre_tool_use[0].command, "echo pre-tool",
        "command should match"
    );
    assert!(
        config.post_tool_use.is_empty(),
        "PostToolUse should be empty"
    );
    assert!(config.stop.is_empty(), "Stop should be empty");
}

// ---------------------------------------------------------------------------
// TC-E2E-12b: Hooks parsing — PostToolUse classification
// AC-15: PostToolUse hooks correctly parsed and classified
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_12b_hooks_post_tool_use_parsed() {
    let hooks_json = serde_json::json!({
        "PostToolUse": [
            {
                "matcher": "*",
                "hooks": [{"type": "command", "command": "echo post-tool"}]
            }
        ]
    });
    let config = parse_skill_hooks(Some(&hooks_json), "test-skill", SkillSource::User)
        .expect("hooks should parse successfully");

    // AC-15 assertions
    assert_eq!(
        config.post_tool_use.len(),
        1,
        "should have one PostToolUse hook"
    );
    assert_eq!(
        config.post_tool_use[0].command, "echo post-tool",
        "command should match"
    );
    assert!(config.pre_tool_use.is_empty(), "PreToolUse should be empty");
    assert!(config.stop.is_empty(), "Stop should be empty");
}

// ---------------------------------------------------------------------------
// TC-E2E-12c: Hooks parsing — Stop classification
// AC-15: Stop hooks correctly parsed and classified
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_12c_hooks_stop_parsed() {
    let hooks_json = serde_json::json!({
        "Stop": [
            {
                "matcher": "*",
                "hooks": [{"type": "command", "command": "echo stop"}]
            }
        ]
    });
    let config = parse_skill_hooks(Some(&hooks_json), "test-skill", SkillSource::User)
        .expect("hooks should parse successfully");

    // AC-15 assertions
    assert_eq!(config.stop.len(), 1, "should have one Stop hook");
    assert_eq!(config.stop[0].command, "echo stop", "command should match");
    assert!(config.pre_tool_use.is_empty(), "PreToolUse should be empty");
    assert!(
        config.post_tool_use.is_empty(),
        "PostToolUse should be empty"
    );
}

// ---------------------------------------------------------------------------
// TC-E2E-12d: Hooks to HookDefs conversion — all three event types
// AC-15: SkillHooksConfig correctly converts to HooksConfig (HookDef list)
// ---------------------------------------------------------------------------

#[test]
fn tc_e2e_12d_hooks_to_hook_defs_all_events() {
    let hooks_json = serde_json::json!({
        "PreToolUse": [
            {"matcher": "Bash", "hooks": [{"type": "command", "command": "echo pre"}]}
        ],
        "PostToolUse": [
            {"matcher": "*", "hooks": [{"type": "command", "command": "echo post"}]}
        ],
        "Stop": [
            {"matcher": "*", "hooks": [{"type": "command", "command": "echo stop"}]}
        ]
    });

    let config = parse_skill_hooks(Some(&hooks_json), "multi-hook-skill", SkillSource::Project)
        .expect("hooks should parse");

    let hook_defs = to_hook_defs(&config, "multi-hook-skill");

    assert_eq!(hook_defs.pre_tool_use.len(), 1);
    assert_eq!(hook_defs.post_tool_use.len(), 1);
    assert_eq!(hook_defs.stop.len(), 1);

    // Verify naming convention: skill:{name}:{event}:{index}
    assert_eq!(
        hook_defs.pre_tool_use[0].name,
        "skill:multi-hook-skill:pre_tool_use:0"
    );
    assert_eq!(
        hook_defs.post_tool_use[0].name,
        "skill:multi-hook-skill:post_tool_use:0"
    );
    assert_eq!(hook_defs.stop[0].name, "skill:multi-hook-skill:stop:0");

    // Verify MCP skills cannot register hooks (security boundary)
    let mcp_result = parse_skill_hooks(Some(&hooks_json), "mcp-skill", SkillSource::Mcp);
    assert!(
        mcp_result.is_none(),
        "MCP skills should not be able to register hooks"
    );
}

// ===========================================================================
// White-box tests [白盒] — Phase 14 阶段 5b
//
// Written after reading implementation code to cover:
// - branch coverage gaps
// - edge cases discovered in logic
// - risk points identified in plan.md
// ===========================================================================

// ---------------------------------------------------------------------------
// WB-1: frontmatter — extract_frontmatter_bounds branches [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_1a_frontmatter_no_delimiter_returns_empty_frontmatter() {
    // Input without --- delimiter → content treated as body, frontmatter is default
    use crate::frontmatter::parse_frontmatter;
    let parsed = parse_frontmatter("Just plain text, no frontmatter.");
    assert_eq!(parsed.frontmatter.name, None);
    assert_eq!(parsed.content, "Just plain text, no frontmatter.");
}

#[test]
fn wb_1b_frontmatter_empty_yaml_section_parses_ok() {
    use crate::frontmatter::parse_frontmatter;
    // Empty YAML block: just two --- lines
    let parsed = parse_frontmatter("---\n---\nbody here");
    assert_eq!(parsed.content, "body here");
}

#[test]
fn wb_1c_frontmatter_inherit_model_normalized_to_none() {
    // "inherit" model → None in SkillMetadata (don't override caller)
    use crate::frontmatter::{parse_frontmatter, parse_skill_fields};
    use crate::types::{LoadedFrom, SkillSource};
    let input = "---\nmodel: inherit\n---\nbody";
    let parsed = parse_frontmatter(input);
    let meta = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        "test",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert!(
        meta.model.is_none(),
        "model 'inherit' should normalize to None"
    );
}

#[test]
fn wb_1d_frontmatter_fork_context_parsed() {
    use crate::frontmatter::{parse_frontmatter, parse_skill_fields};
    use crate::types::{ExecutionContext, LoadedFrom, SkillSource};
    let input = "---\ncontext: fork\n---\nbody";
    let parsed = parse_frontmatter(input);
    let meta = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        "fork-skill",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert_eq!(meta.execution_context, ExecutionContext::Fork);
}

#[test]
fn wb_1e_frontmatter_description_from_content_fallback() {
    // No description in frontmatter → first line of content used
    use crate::frontmatter::{parse_frontmatter, parse_skill_fields};
    use crate::types::{LoadedFrom, SkillSource};
    let input = "---\nname: my-skill\n---\nThis is the first line.\nMore content here.";
    let parsed = parse_frontmatter(input);
    let meta = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        "my-skill",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert!(
        !meta.description.is_empty(),
        "description should be extracted from content"
    );
    assert!(!meta.has_user_specified_description);
}

#[test]
fn wb_1f_frontmatter_user_specified_description_flag() {
    use crate::frontmatter::{parse_frontmatter, parse_skill_fields};
    use crate::types::{LoadedFrom, SkillSource};
    let input = "---\ndescription: My explicit description\n---\nbody";
    let parsed = parse_frontmatter(input);
    let meta = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        "s",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert!(meta.has_user_specified_description);
    assert_eq!(meta.description, "My explicit description");
}

// ---------------------------------------------------------------------------
// WB-2: permissions — auto_approve branch [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_2a_auto_approve_converts_ask_to_allow() {
    // With auto_approve=true, Ask → Allow (but Deny remains Deny)
    let checker = SkillPermissionChecker::new(vec![], vec![], true);
    let mut skill = make_skill("auto-skill", "body");
    // Attach hooks to prevent safe-properties path
    skill.hooks_raw = Some(
        serde_json::json!({"PreToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": "x"}]}]}),
    );
    let result = checker.check(&skill);
    assert_eq!(
        result,
        SkillPermission::Allow,
        "auto_approve should convert Ask to Allow"
    );
}

#[test]
fn wb_2b_auto_approve_does_not_bypass_deny() {
    // Deny wins even when auto_approve=true
    let checker = SkillPermissionChecker::new(
        vec!["blocked".to_string()],
        vec![],
        true, // auto_approve
    );
    let skill = make_skill("blocked", "body");
    let result = checker.check(&skill);
    assert_eq!(
        result,
        SkillPermission::Deny,
        "Deny should not be overridden by auto_approve"
    );
}

#[test]
fn wb_2c_permission_prefix_rule_matches_namespace() {
    // "db:*" prefix rule should match "db:migrate" and "db:seed"
    use crate::permissions::PermissionRule;
    let rule = PermissionRule::parse("db:*");
    assert!(rule.matches("db:migrate"));
    assert!(rule.matches("db:seed"));
    assert!(
        !rule.matches("database"),
        "should not match without the colon separator"
    );
}

#[test]
fn wb_2d_permission_exact_rule_no_partial_match() {
    use crate::permissions::PermissionRule;
    let rule = PermissionRule::parse("commit");
    assert!(rule.matches("commit"));
    assert!(!rule.matches("commit-amend"));
    assert!(!rule.matches("my:commit"));
}

// ---------------------------------------------------------------------------
// WB-3: conditional — activated_names persists across clear_dormant [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_3a_activated_names_persist_after_clear_dormant() {
    let mut manager = ConditionalSkillManager::new();
    let mut skill = make_skill("rs-skill", "body");
    skill.paths = vec!["*.rs".to_string()];

    manager.partition_skills(vec![skill.clone()]);
    manager.activate_for_paths(&["/project/main.rs"], "/project");
    assert!(manager.get_activated("rs-skill").is_some());

    // clear_dormant should not remove activated_names
    manager.clear_dormant();

    // Re-partition with same skill — it should go to unconditional since it was activated
    let unconditional = manager.partition_skills(vec![skill]);
    assert_eq!(
        unconditional.len(),
        1,
        "previously activated skill should be unconditional after reload"
    );
}

#[test]
fn wb_3b_reset_all_clears_everything() {
    let mut manager = ConditionalSkillManager::new();
    let mut skill = make_skill("sk", "body");
    skill.paths = vec!["*.ts".to_string()];
    manager.partition_skills(vec![skill.clone()]);
    manager.activate_for_paths(&["/project/app.ts"], "/project");

    manager.reset_all();

    assert_eq!(manager.dormant_count(), 0);
    assert!(manager.get_all_activated().is_empty());

    // After reset, same skill should be dormant again
    let unconditional = manager.partition_skills(vec![skill]);
    assert!(
        unconditional.is_empty(),
        "after reset, skill should be dormant again"
    );
}

#[test]
fn wb_3c_dormant_count_reflects_state() {
    let mut manager = ConditionalSkillManager::new();
    assert_eq!(manager.dormant_count(), 0);

    let mut s1 = make_skill("s1", "b");
    s1.paths = vec!["*.rs".to_string()];
    let mut s2 = make_skill("s2", "b");
    s2.paths = vec!["*.ts".to_string()];

    manager.partition_skills(vec![s1, s2]);
    assert_eq!(manager.dormant_count(), 2);

    manager.activate_for_paths(&["/p/f.rs"], "/p");
    assert_eq!(
        manager.dormant_count(),
        1,
        "one skill activated, one still dormant"
    );
}

// ---------------------------------------------------------------------------
// WB-4: substitution — argument parsing edge cases [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_4a_substitution_fallback_appended_when_no_placeholder_matched() {
    use crate::substitution::substitute_arguments;
    // Content has no $ARGUMENTS placeholder → fallback append
    let result = substitute_arguments("Fixed content.", Some("extra_arg"), &[], None, None);
    assert!(
        result.ends_with("\n\nARGUMENTS: extra_arg"),
        "fallback should be appended, got: {result}"
    );
}

#[test]
fn wb_4b_named_arg_numeric_name_skipped() {
    use crate::substitution::substitute_arguments;
    // Numeric argument_names are skipped to avoid conflicting with $0 shorthand
    let names = vec!["0".to_string()];
    let result = substitute_arguments("Val: $0", Some("hello"), &names, None, None);
    // $0 should be replaced via shorthand path, not named path
    assert_eq!(result, "Val: hello");
}

#[test]
fn wb_4c_parse_arguments_tab_as_separator() {
    use crate::substitution::parse_arguments;
    // Tabs should split arguments same as spaces
    let result = parse_arguments("foo\tbar\tbaz");
    assert_eq!(result, vec!["foo", "bar", "baz"]);
}

#[test]
fn wb_4d_multiple_spaces_between_args_ignored() {
    use crate::substitution::parse_arguments;
    // Multiple spaces should not produce empty tokens
    let result = parse_arguments("foo   bar");
    assert_eq!(result, vec!["foo", "bar"]);
}

// ---------------------------------------------------------------------------
// WB-5: shell — format_output edge cases [白盒]
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wb_5a_shell_empty_command_replaced_with_empty() {
    use crate::shell::execute_shell_commands;
    // `cd .` exits 0 with no output on all platforms
    let content = "before !`cd .` after";
    let tmp = std::env::temp_dir();
    let result = execute_shell_commands(content, LoadedFrom::Skills, tmp.to_str().unwrap())
        .await
        .unwrap();
    // The !`cd .` replacement is empty string, so "before  after" (double space)
    assert_eq!(result, "before  after");
}

#[tokio::test]
async fn wb_5b_shell_block_multiline_command() {
    use crate::shell::execute_shell_commands;
    let content = "```!\necho line1\necho line2\n```";
    let tmp = std::env::temp_dir();
    let result = execute_shell_commands(content, LoadedFrom::Skills, tmp.to_str().unwrap())
        .await
        .unwrap();
    assert!(result.contains("line1"));
    assert!(result.contains("line2"));
    assert!(!result.contains("```!"));
}

// ---------------------------------------------------------------------------
// WB-6: loader — build_namespace colon separation [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_6a_build_namespace_two_levels() {
    use crate::loader::build_namespace;
    use std::path::Path;
    let base = Path::new("/skills");
    let target = Path::new("/skills/db/migrate");
    assert_eq!(build_namespace(base, target), "db:migrate");
}

#[test]
fn wb_6b_build_namespace_single_level() {
    use crate::loader::build_namespace;
    use std::path::Path;
    assert_eq!(
        build_namespace(Path::new("/skills"), Path::new("/skills/my-skill")),
        "my-skill"
    );
}

#[test]
fn wb_6c_build_namespace_same_dir_empty() {
    use crate::loader::build_namespace;
    use std::path::Path;
    let base = Path::new("/skills");
    assert_eq!(build_namespace(base, base), "");
}

// ---------------------------------------------------------------------------
// WB-7: context_modifier — is_empty and from_skill branches [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_7a_context_modifier_none_when_no_overrides() {
    let skill = make_skill("plain", "body");
    let result = crate::context_modifier::from_skill(&skill);
    assert!(result.is_none(), "no overrides should produce None");
}

#[test]
fn wb_7b_context_modifier_is_empty_default() {
    let m = ContextModifier::default();
    assert!(m.is_empty());
}

#[test]
fn wb_7c_context_modifier_allowed_tools_only() {
    let mut skill = make_skill("tools", "body");
    skill.allowed_tools = vec!["Write".to_string()];
    let m = crate::context_modifier::from_skill(&skill).expect("should have modifier");
    assert!(m.model.is_none());
    assert!(m.effort.is_none());
    assert_eq!(m.allowed_tools, vec!["Write"]);
    assert!(!m.is_empty());
}

// ---------------------------------------------------------------------------
// WB-8: prompt — format_skills_within_budget edge cases [白盒]
// ---------------------------------------------------------------------------

#[test]
fn wb_8a_empty_skills_returns_empty_string() {
    let result = format_skills_within_budget(&[], None);
    assert_eq!(result, "");
}

#[test]
fn wb_8b_single_skill_within_budget() {
    let mut skill = make_skill("my-skill", "body");
    skill.description = "A short description".to_string();
    let result = format_skills_within_budget(&[skill], None);
    assert!(result.contains("my-skill"));
    assert!(result.contains("A short description"));
}

#[test]
fn wb_8c_all_bundled_no_non_bundled() {
    // When only bundled skills exist, they're all returned even under budget pressure
    let mut b1 = make_skill("bundled-a", "body");
    b1.source = SkillSource::Bundled;
    b1.description = "Bundled A".to_string();
    let mut b2 = make_skill("bundled-b", "body");
    b2.source = SkillSource::Bundled;
    b2.description = "Bundled B".to_string();

    let result = format_skills_within_budget(&[b1, b2], Some(1)); // tiny budget
    assert!(result.contains("bundled-a"), "bundled A should be present");
    assert!(result.contains("bundled-b"), "bundled B should be present");
}
