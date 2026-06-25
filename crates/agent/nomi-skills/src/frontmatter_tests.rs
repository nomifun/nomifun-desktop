// Supplemental tests for frontmatter.rs — covers test-plan.md cases not in impl tests.
// Included from frontmatter.rs as: #[cfg(test)] mod supplemental_tests;
// `use super::*` gives access to private functions in frontmatter.rs.

use super::*;
use crate::types::{
    BoolOrString, EffortLevel, ExecutionContext, FrontmatterData, LoadedFrom, SkillSource,
    StringOrNumber, StringOrVec,
};

// -----------------------------------------------------------------------
// TC-1.x: FrontmatterData deserialization
// -----------------------------------------------------------------------

#[test]
fn tc_1_3_serde_default_missing_fields_are_none() {
    let yaml = "name: test";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(data.name.as_deref(), Some("test"));
    assert!(data.description.is_none());
    assert!(data.allowed_tools.is_none());
    assert!(data.argument_hint.is_none());
    assert!(data.arguments.is_none());
    assert!(data.model.is_none());
    assert!(data.effort.is_none());
    assert!(data.context.is_none());
    assert!(data.user_invocable.is_none());
}

#[test]
fn tc_1_4_allowed_tools_array() {
    let yaml = "allowed-tools:\n  - Bash\n  - Read\n  - Write";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.allowed_tools.unwrap() {
        StringOrVec::Multiple(v) => assert_eq!(v, vec!["Bash", "Read", "Write"]),
        other => panic!("expected Multiple, got {:?}", other),
    }
}

#[test]
fn tc_1_5_allowed_tools_single_string() {
    let yaml = "allowed-tools: Bash";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.allowed_tools.unwrap() {
        StringOrVec::Single(s) => assert_eq!(s, "Bash"),
        other => panic!("expected Single, got {:?}", other),
    }
}

#[test]
fn tc_1_6_effort_as_string() {
    let yaml = "effort: high";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.effort.unwrap() {
        StringOrNumber::Str(s) => assert_eq!(s, "high"),
        other => panic!("expected Str, got {:?}", other),
    }
}

#[test]
fn tc_1_7_effort_as_number() {
    let yaml = "effort: 3";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.effort.unwrap() {
        StringOrNumber::Num(n) => assert_eq!(n, 3),
        other => panic!("expected Num, got {:?}", other),
    }
}

#[test]
fn tc_1_8_user_invocable_bool() {
    let yaml = "user-invocable: false";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.user_invocable.unwrap() {
        BoolOrString::Bool(b) => assert!(!b),
        other => panic!("expected Bool, got {:?}", other),
    }
}

#[test]
fn tc_1_9_user_invocable_string() {
    let yaml = "user-invocable: \"true\"";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    match data.user_invocable.unwrap() {
        BoolOrString::Str(s) => assert_eq!(s, "true"),
        other => panic!("expected Str, got {:?}", other),
    }
}

#[test]
fn tc_1_10_hooks_field_preserved() {
    let yaml = "hooks:\n  PostToolUse:\n    - command: echo done";
    let data: FrontmatterData = serde_yaml::from_str(yaml).unwrap();
    assert!(data.hooks.is_some());
}

// -----------------------------------------------------------------------
// TC-2.x: Two-pass parsing strategy
// -----------------------------------------------------------------------

#[test]
fn tc_2_3_square_bracket_in_value() {
    let input = "---\nargument-hint: [optional]\n---\nbody";
    let parsed = parse_frontmatter(input);
    assert_eq!(
        parsed.frontmatter.argument_hint.as_deref(),
        Some("[optional]")
    );
}

#[test]
fn tc_2_4_asterisk_in_value() {
    let input = "---\ndescription: Match *.rs files\n---\nbody";
    let parsed = parse_frontmatter(input);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Match *.rs files")
    );
}

#[test]
fn tc_2_5_hash_in_value() {
    // YAML treats " #..." as an inline comment. serde_yaml first pass "succeeds"
    // but silently strips the comment portion: "See issue #123" → "See issue".
    // The two-pass rescue only triggers when the first pass errors — it does not
    // detect silent value truncation caused by inline comments.
    // Known limitation: values containing " #" are not rescued by quote_problematic_values.
    let input = "---\ndescription: See issue #123\n---\nbody";
    let parsed = parse_frontmatter(input);
    assert_eq!(parsed.frontmatter.description.as_deref(), Some("See issue"));
}

#[test]
fn tc_2_6_pipe_in_value() {
    let input = "---\ndescription: Use cmd | grep pattern\n---\nbody";
    let parsed = parse_frontmatter(input);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Use cmd | grep pattern")
    );
}

#[test]
fn tc_2_7_both_passes_fail_returns_empty_frontmatter() {
    // Deeply malformed YAML that cannot be rescued by quoting
    let input = "---\n: {unclosed\n  bad: : : yaml:\n---\n# Real Content\n";
    let parsed = parse_frontmatter(input);
    // Must not panic; all fields should be None (empty FrontmatterData)
    assert!(parsed.frontmatter.name.is_none());
    assert!(parsed.frontmatter.description.is_none());
}

#[test]
fn tc_2_8_multiple_special_char_fields() {
    let input =
        "---\ndescription: Handle {a} and [b] patterns\nargument-hint: <file> [options]\n---\n";
    let parsed = parse_frontmatter(input);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Handle {a} and [b] patterns")
    );
    assert_eq!(
        parsed.frontmatter.argument_hint.as_deref(),
        Some("<file> [options]")
    );
}

// -----------------------------------------------------------------------
// TC-3.x: Edge cases for parse_frontmatter
// -----------------------------------------------------------------------

#[test]
fn tc_3_1_no_frontmatter_plain_markdown() {
    let input = "# Just a heading\nSome content";
    let parsed = parse_frontmatter(input);
    assert!(parsed.frontmatter.name.is_none());
    assert!(parsed.frontmatter.description.is_none());
    assert_eq!(parsed.content, input);
}

#[test]
fn tc_3_2_empty_string_input() {
    let parsed = parse_frontmatter("");
    assert!(parsed.frontmatter.name.is_none());
    assert_eq!(parsed.content, "");
}

#[test]
fn tc_3_3_only_frontmatter_no_body() {
    let input = "---\nname: test\n---\n";
    let parsed = parse_frontmatter(input);
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("test"));
    assert_eq!(parsed.content, "");
}

#[test]
fn tc_3_5_yaml_comment_in_frontmatter() {
    let input = "---\n# This is a comment\nname: test\n---\n";
    let parsed = parse_frontmatter(input);
    assert_eq!(parsed.frontmatter.name.as_deref(), Some("test"));
}

#[test]
fn tc_3_6_frontmatter_not_at_start() {
    // --- not at line 0 — should NOT be treated as frontmatter
    let input = "Some text\n---\nname: test\n---\n";
    let parsed = parse_frontmatter(input);
    assert!(parsed.frontmatter.name.is_none());
    assert_eq!(parsed.content, input);
}

#[test]
fn tc_3_7_only_opening_fence_no_close() {
    let input = "---\nname: test\n# No closing fence";
    let parsed = parse_frontmatter(input);
    assert!(parsed.frontmatter.name.is_none());
    assert_eq!(parsed.content, input);
}

// -----------------------------------------------------------------------
// TC-4.x: parse_skill_fields normalization
// -----------------------------------------------------------------------

#[test]
fn tc_4_1_user_invocable_defaults_to_true() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert!(meta.user_invocable);
}

#[test]
fn tc_4_2_user_invocable_false() {
    let fm = FrontmatterData {
        user_invocable: Some(BoolOrString::Bool(false)),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert!(!meta.user_invocable);
}

#[test]
fn tc_4_3_user_invocable_string_false() {
    let fm = FrontmatterData {
        user_invocable: Some(BoolOrString::Str("false".into())),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert!(!meta.user_invocable);
}

#[test]
fn tc_4_5_model_non_inherit_preserved() {
    let fm = FrontmatterData {
        model: Some("claude-opus-4-6".into()),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.model.as_deref(), Some("claude-opus-4-6"));
}

#[test]
fn tc_4_6_description_extracted_from_content_first_nonheading_line() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(
        &fm,
        "# Title\n\nFirst real paragraph.",
        "x",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert_eq!(meta.description, "First real paragraph.");
    assert!(!meta.has_user_specified_description);
}

#[test]
fn tc_4_7_description_empty_content_no_panic() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.description, "");
    assert!(!meta.has_user_specified_description);
}

#[test]
fn tc_4_8_has_user_specified_description_true_when_frontmatter_has_it() {
    let fm = FrontmatterData {
        description: Some("User provided".into()),
        ..Default::default()
    };
    let meta = parse_skill_fields(
        &fm,
        "# Title",
        "x",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    assert!(meta.has_user_specified_description);
    assert_eq!(meta.description, "User provided");
}

#[test]
fn tc_4_10_allowed_tools_single_string_to_vec() {
    let fm = FrontmatterData {
        allowed_tools: Some(StringOrVec::Single("Bash".into())),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.allowed_tools, vec!["Bash"]);
}

#[test]
fn tc_4_11_allowed_tools_comma_separated() {
    let fm = FrontmatterData {
        allowed_tools: Some(StringOrVec::Single("Bash,Read,Write".into())),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.allowed_tools, vec!["Bash", "Read", "Write"]);
}

#[test]
fn tc_4_12_allowed_tools_none_gives_empty_vec() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert!(meta.allowed_tools.is_empty());
}

#[test]
fn tc_4_13_argument_names_parsed() {
    let fm = FrontmatterData {
        arguments: Some(StringOrVec::Multiple(vec!["query".into(), "limit".into()])),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.argument_names, vec!["query", "limit"]);
}

#[test]
fn tc_4_14_execution_context_defaults_to_inline() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert_eq!(meta.execution_context, ExecutionContext::Inline);
}

#[test]
fn tc_4_17_source_and_loaded_from_passed_through() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(
        &fm,
        "",
        "x",
        SkillSource::Project,
        LoadedFrom::CommandsDeprecated,
        None,
    );
    assert_eq!(meta.source, SkillSource::Project);
    assert_eq!(meta.loaded_from, LoadedFrom::CommandsDeprecated);
}

#[test]
fn tc_4_18_skill_root_passed_through() {
    let fm = FrontmatterData::default();
    let meta = parse_skill_fields(
        &fm,
        "",
        "x",
        SkillSource::User,
        LoadedFrom::Skills,
        Some("/home/user/.claude/skills"),
    );
    assert_eq!(
        meta.skill_root.as_deref(),
        Some("/home/user/.claude/skills")
    );
}

#[test]
fn tc_4_19_disable_model_invocation_mapping() {
    let fm = FrontmatterData {
        hide_from_model_invocation: Some(BoolOrString::Bool(true)),
        ..Default::default()
    };
    let meta = parse_skill_fields(&fm, "", "x", SkillSource::User, LoadedFrom::Skills, None);
    assert!(meta.disable_model_invocation);
}

// -----------------------------------------------------------------------
// TC-5.x: parse_effort additional cases
// -----------------------------------------------------------------------

#[test]
fn tc_5_5_effort_number_mapping() {
    assert_eq!(
        parse_effort(&Some(StringOrNumber::Num(3))),
        Some(EffortLevel::Max)
    );
    assert_eq!(
        parse_effort(&Some(StringOrNumber::Num(0))),
        Some(EffortLevel::Low)
    );
    assert_eq!(
        parse_effort(&Some(StringOrNumber::Num(1))),
        Some(EffortLevel::Medium)
    );
    assert_eq!(
        parse_effort(&Some(StringOrNumber::Num(2))),
        Some(EffortLevel::High)
    );
}

#[test]
fn tc_5_7_effort_unknown_string_returns_none() {
    let result = parse_effort(&Some(StringOrNumber::Str("unknown".into())));
    assert_eq!(result, None);
}

#[test]
fn tc_5_8_effort_uppercase_string() {
    let result = parse_effort(&Some(StringOrNumber::Str("HIGH".into())));
    assert_eq!(result, Some(EffortLevel::High));
}

#[test]
fn tc_5_x_effort_normal_alias() {
    let result = parse_effort(&Some(StringOrNumber::Str("normal".into())));
    assert_eq!(result, Some(EffortLevel::Medium));
}

#[test]
fn tc_5_x_effort_maximum_alias() {
    let result = parse_effort(&Some(StringOrNumber::Str("maximum".into())));
    assert_eq!(result, Some(EffortLevel::Max));
}

// -----------------------------------------------------------------------
// TC-6.x: expand_braces additional cases
// -----------------------------------------------------------------------

#[test]
fn tc_6_2_path_prefix_brace_expansion() {
    let mut result = expand_braces("src/*.{ts,tsx}");
    result.sort();
    assert_eq!(result, vec!["src/*.ts", "src/*.tsx"]);
}

#[test]
fn tc_6_6_three_element_brace() {
    let mut result = expand_braces("*.{rs,toml,md}");
    result.sort();
    assert_eq!(result, vec!["*.md", "*.rs", "*.toml"]);
}

#[test]
fn tc_6_7_empty_string_no_panic() {
    // Must not panic
    let _ = expand_braces("");
}

// -----------------------------------------------------------------------
// TC-7.x: parse_string_or_vec additional cases
// -----------------------------------------------------------------------

#[test]
fn tc_7_5_comma_separated_with_spaces_trimmed() {
    let v = parse_string_or_vec(&Some(StringOrVec::Single("Bash, Read, Write".into())));
    assert_eq!(v, vec!["Bash", "Read", "Write"]);
}

// -----------------------------------------------------------------------
// TC-8.x: split_paths
// -----------------------------------------------------------------------

#[test]
fn tc_8_1_single_path_no_brace() {
    let result = split_paths(&Some(StringOrVec::Single("src/**/*.rs".into())));
    assert_eq!(result, vec!["src/**/*.rs"]);
}

#[test]
fn tc_8_2_single_path_with_brace() {
    let mut result = split_paths(&Some(StringOrVec::Single("src/*.{ts,tsx}".into())));
    result.sort();
    assert_eq!(result, vec!["src/*.ts", "src/*.tsx"]);
}

#[test]
fn tc_8_3_multiple_paths_each_brace_expanded() {
    let mut result = split_paths(&Some(StringOrVec::Multiple(vec![
        "*.{rs,toml}".into(),
        "src/**".into(),
    ])));
    result.sort();
    assert_eq!(result, vec!["*.rs", "*.toml", "src/**"]);
}

#[test]
fn tc_8_4_comma_separated_paths_string() {
    let mut result = split_paths(&Some(StringOrVec::Single("src/*.rs,tests/*.rs".into())));
    result.sort();
    assert_eq!(result, vec!["src/*.rs", "tests/*.rs"]);
}

#[test]
fn tc_8_4b_comma_in_brace_not_split() {
    let mut result = split_paths(&Some(StringOrVec::Single("src/*.{ts,tsx}".into())));
    result.sort();
    assert_eq!(result, vec!["src/*.ts", "src/*.tsx"]);
}

#[test]
fn tc_8_5_none_returns_empty_vec() {
    let result = split_paths(&None);
    assert!(result.is_empty());
}

// -----------------------------------------------------------------------
// TC-9.x: parse_bool additional cases
// -----------------------------------------------------------------------

#[test]
fn tc_9_7_unknown_string_returns_default() {
    let result = parse_bool(&Some(BoolOrString::Str("yes".into())), false);
    assert!(!result);
}

// -----------------------------------------------------------------------
// TC-10.x: extract_description_from_content
// -----------------------------------------------------------------------

#[test]
fn tc_10_1_extract_first_nonheading_line_skips_h1() {
    let result = extract_description_from_content("# My Skill Title\nSome description");
    assert_eq!(result.as_deref(), Some("Some description"));
}

#[test]
fn tc_10_2_extract_plain_first_line() {
    let result = extract_description_from_content("First line of content\nSecond line");
    assert_eq!(result.as_deref(), Some("First line of content"));
}

#[test]
fn tc_10_3_empty_content_returns_none() {
    let result = extract_description_from_content("");
    assert!(result.is_none());
}

#[test]
fn tc_10_4_all_whitespace_returns_none() {
    let result = extract_description_from_content("\n\n\n");
    assert!(result.is_none());
}

#[test]
fn tc_10_5_skips_blank_lines_then_heading() {
    let result = extract_description_from_content("\n\n# Real Title\nContent");
    assert_eq!(result.as_deref(), Some("Content"));
}

// -----------------------------------------------------------------------
// TC-11.x: quote_problematic_values additional cases
// -----------------------------------------------------------------------

#[test]
fn tc_11_2_square_bracket_gets_quoted() {
    let yaml = "argument-hint: [optional]";
    let fixed = quote_problematic_values(yaml);
    assert!(fixed.contains("\"[optional]\""));
}

#[test]
fn tc_11_3_no_special_chars_unchanged() {
    let yaml = "name: simple-name";
    let fixed = quote_problematic_values(yaml);
    assert_eq!(fixed.trim(), yaml);
}

#[test]
fn tc_11_5_only_problematic_lines_requoted() {
    let yaml = "name: simple\ndescription: Use {x} to do y\nversion: \"1.0\"";
    let fixed = quote_problematic_values(yaml);
    assert!(fixed.contains("name: simple"));
    assert!(fixed.contains("version: \"1.0\""));
    assert!(fixed.contains("\"Use {x} to do y\""));
}

// -----------------------------------------------------------------------
// TC-12.x: Integration tests
// -----------------------------------------------------------------------

#[test]
fn tc_12_1_full_skill_file_standard() {
    let input = r#"---
name: test-skill
description: A test skill for integration
allowed-tools: Bash
user-invocable: true
paths: "src/*.{rs,toml}"
effort: high
---
# Test Skill
This skill does things.
"#;
    let parsed = parse_frontmatter(input);
    let meta = parse_skill_fields(
        &parsed.frontmatter,
        &parsed.content,
        "test-skill",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );

    assert_eq!(meta.name, "test-skill");
    assert_eq!(meta.allowed_tools, vec!["Bash"]);
    assert!(meta.user_invocable);
    assert_eq!(meta.effort, Some(EffortLevel::High));
    assert!(meta.content.contains("This skill does things"));

    let mut paths = meta.paths.clone();
    paths.sort();
    assert_eq!(paths, vec!["src/*.rs", "src/*.toml"]);
}

#[test]
fn tc_12_2_full_skill_file_special_chars() {
    let input = r#"---
description: Handle {input} and [output] patterns
argument-hint: <file> [options]
---
# Body
"#;
    let parsed = parse_frontmatter(input);
    assert_eq!(
        parsed.frontmatter.description.as_deref(),
        Some("Handle {input} and [output] patterns")
    );
    assert_eq!(
        parsed.frontmatter.argument_hint.as_deref(),
        Some("<file> [options]")
    );
}

#[test]
fn tc_12_3_legacy_commands_loaded_from() {
    let fm = FrontmatterData {
        description: Some("Legacy skill".into()),
        ..Default::default()
    };
    let meta = parse_skill_fields(
        &fm,
        "# Legacy\nDoes old things.",
        "legacy-cmd",
        SkillSource::Legacy,
        LoadedFrom::CommandsDeprecated,
        Some("/project/.claude/commands"),
    );
    assert_eq!(meta.loaded_from, LoadedFrom::CommandsDeprecated);
    assert_eq!(meta.source, SkillSource::Legacy);
    assert_eq!(
        meta.skill_root.as_deref(),
        Some("/project/.claude/commands")
    );
}
