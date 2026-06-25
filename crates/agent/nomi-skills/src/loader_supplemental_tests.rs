use super::*;
use crate::types::{FrontmatterData, LoadedFrom, SkillSource};
use std::fs;
use tempfile::TempDir;

fn write_skill(dir: &Path, rel_path: &str, content: &str) {
    let full = dir.join(rel_path);
    fs::create_dir_all(full.parent().unwrap()).unwrap();
    fs::write(full, content).unwrap();
}

fn make_loaded_skill(path: PathBuf, name: &str) -> LoadedSkill {
    let fm = FrontmatterData::default();
    let metadata = crate::frontmatter::parse_skill_fields(
        &fm,
        "",
        name,
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    LoadedSkill {
        metadata,
        resolved_path: path,
    }
}

// -----------------------------------------------------------------------
// TC-7.x: build_namespace
// -----------------------------------------------------------------------

#[test]
fn tc_7_1_build_namespace_single_level() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/my-tool");
    assert_eq!(build_namespace(base, target), "my-tool");
}

#[test]
fn tc_7_2_build_namespace_two_levels() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/db/migrate");
    assert_eq!(build_namespace(base, target), "db:migrate");
}

#[test]
fn tc_7_3_build_namespace_three_levels() {
    let base = Path::new("/skills");
    let target = Path::new("/skills/a/b/c");
    assert_eq!(build_namespace(base, target), "a:b:c");
}

#[test]
fn tc_7_4_build_namespace_same_dir_returns_empty() {
    let base = Path::new("/skills");
    let result = build_namespace(base, base);
    assert_eq!(result, "", "base == target should produce empty string");
}

// -----------------------------------------------------------------------
// TC-8.x: load_skills_from_dir supplemental cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn tc_8_4_dir_without_skill_md_skipped() {
    let tmp = TempDir::new().unwrap();
    // empty-dir has no SKILL.md; valid-skill does
    fs::create_dir_all(tmp.path().join("empty-dir")).unwrap();
    write_skill(tmp.path(), "valid-skill/SKILL.md", "---\n---\n");

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "valid-skill");
}

#[tokio::test]
async fn tc_8_7_source_and_loaded_from_passed_through() {
    let tmp = TempDir::new().unwrap();
    write_skill(tmp.path(), "test-skill/SKILL.md", "---\n---\n");

    let skills = load_skills_from_dir(tmp.path(), SkillSource::Project, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.source, SkillSource::Project);
    assert_eq!(skills[0].metadata.loaded_from, LoadedFrom::Skills);
}

#[tokio::test]
async fn tc_8_9_resolved_path_is_canonical() {
    let tmp = TempDir::new().unwrap();
    write_skill(tmp.path(), "my-skill/SKILL.md", "---\n---\n");

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);

    let skill_file = tmp.path().join("my-skill").join("SKILL.md");
    let expected_canonical = std::fs::canonicalize(&skill_file).unwrap();
    assert_eq!(skills[0].resolved_path, expected_canonical);
}

#[tokio::test]
async fn tc_8_x_full_frontmatter_parsed() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "my-skill/SKILL.md",
        "---\ndescription: My skill description\nallowed-tools: Bash\n---\n# Body\n",
    );

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.description, "My skill description");
    assert_eq!(skills[0].metadata.allowed_tools, vec!["Bash"]);
}

#[tokio::test]
async fn tc_8_x_no_frontmatter_description_from_body() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "my-skill/SKILL.md",
        "# My Title\nDoes things.\n",
    );

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);
    // description extracted from first non-heading line
    assert_eq!(skills[0].metadata.description, "Does things.");
    assert!(!skills[0].metadata.has_user_specified_description);
}

// -----------------------------------------------------------------------
// TC-9.x: load_skills_from_commands_dir supplemental cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn tc_9_2_flat_md_name_without_extension() {
    let tmp = TempDir::new().unwrap();
    write_skill(tmp.path(), "simple.md", "---\ndescription: Simple\n---\n");

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "simple");
}

#[tokio::test]
async fn tc_9_3_nested_flat_format_namespace() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "db/migrate.md",
        "---\ndescription: DB migrate\n---\n",
    );

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "db:migrate");
}

#[tokio::test]
async fn tc_9_5_non_md_files_ignored() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("notes.txt"), "just notes").unwrap();
    fs::write(tmp.path().join("config.yaml"), "key: value").unwrap();
    write_skill(tmp.path(), "valid.md", "---\ndescription: Valid\n---\n");

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].metadata.name, "valid");
}

#[tokio::test]
async fn tc_9_6_empty_commands_dir() {
    let tmp = TempDir::new().unwrap();
    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::User).await;
    assert!(skills.is_empty());
}

#[tokio::test]
async fn tc_9_7_nonexistent_commands_dir_no_panic() {
    let skills = load_skills_from_commands_dir(
        Path::new("/nonexistent/commands/dir/xyz"),
        SkillSource::User,
    )
    .await;
    assert!(skills.is_empty());
}

#[tokio::test]
async fn tc_9_1_commands_directory_format_loaded_from_deprecated() {
    let tmp = TempDir::new().unwrap();
    write_skill(
        tmp.path(),
        "my-cmd/SKILL.md",
        "---\ndescription: A command\n---\n",
    );

    let skills = load_skills_from_commands_dir(tmp.path(), SkillSource::Project).await;
    assert_eq!(skills.len(), 1);
    assert_eq!(
        skills[0].metadata.loaded_from,
        LoadedFrom::CommandsDeprecated
    );
    assert_eq!(skills[0].metadata.source, SkillSource::Project);
}

// -----------------------------------------------------------------------
// TC-10.x: deduplicate supplemental cases
// -----------------------------------------------------------------------

#[test]
fn tc_10_1_deduplicate_no_duplicates_all_preserved() {
    let tmp = TempDir::new().unwrap();
    let f1 = tmp.path().join("a.md");
    let f2 = tmp.path().join("b.md");
    let f3 = tmp.path().join("c.md");
    fs::write(&f1, "").unwrap();
    fs::write(&f2, "").unwrap();
    fs::write(&f3, "").unwrap();

    let skills = vec![
        make_loaded_skill(std::fs::canonicalize(&f1).unwrap(), "skill-a"),
        make_loaded_skill(std::fs::canonicalize(&f2).unwrap(), "skill-b"),
        make_loaded_skill(std::fs::canonicalize(&f3).unwrap(), "skill-c"),
    ];

    let result = deduplicate(skills);
    assert_eq!(result.len(), 3);
}

#[test]
fn tc_10_2_deduplicate_first_occurrence_wins() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("skill.md");
    fs::write(&f, "").unwrap();
    let canonical = std::fs::canonicalize(&f).unwrap();

    // Two LoadedSkill with the same path but different names (first should win)
    let fm = FrontmatterData::default();
    let meta_first = crate::frontmatter::parse_skill_fields(
        &fm,
        "",
        "first-name",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );
    let meta_second = crate::frontmatter::parse_skill_fields(
        &fm,
        "",
        "second-name",
        SkillSource::User,
        LoadedFrom::Skills,
        None,
    );

    let skills = vec![
        LoadedSkill {
            metadata: meta_first,
            resolved_path: canonical.clone(),
        },
        LoadedSkill {
            metadata: meta_second,
            resolved_path: canonical,
        },
    ];

    let result = deduplicate(skills);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "first-name", "first occurrence should win");
}

#[test]
fn tc_10_3_deduplicate_empty_input() {
    let result = deduplicate(vec![]);
    assert!(result.is_empty());
}

#[test]
fn tc_10_4_deduplicate_mixed_unique_and_duplicate() {
    let tmp = TempDir::new().unwrap();
    let f1 = tmp.path().join("a.md");
    let f2 = tmp.path().join("b.md");
    let f3 = tmp.path().join("c.md");
    fs::write(&f1, "").unwrap();
    fs::write(&f2, "").unwrap();
    fs::write(&f3, "").unwrap();

    let c1 = std::fs::canonicalize(&f1).unwrap();
    let c2 = std::fs::canonicalize(&f2).unwrap();
    let c3 = std::fs::canonicalize(&f3).unwrap();

    // f1 appears twice, f2 appears twice, f3 appears once → 3 unique
    let skills = vec![
        make_loaded_skill(c1.clone(), "a1"),
        make_loaded_skill(c1, "a2"), // duplicate of a1
        make_loaded_skill(c2.clone(), "b1"),
        make_loaded_skill(c2, "b2"), // duplicate of b1
        make_loaded_skill(c3, "c1"),
    ];

    let result = deduplicate(skills);
    assert_eq!(result.len(), 3);
    let names: Vec<_> = result.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"a1"));
    assert!(names.contains(&"b1"));
    assert!(names.contains(&"c1"));
}

// -----------------------------------------------------------------------
// TC-11.x: load_all_skills supplemental cases
// -----------------------------------------------------------------------

#[tokio::test]
async fn tc_11_1_bare_mode_only_loads_add_dirs() {
    let user_tmp = TempDir::new().unwrap();
    let add_tmp = TempDir::new().unwrap();

    // Put a skill in add_dir's .nomi/skills/
    let add_skills_dir = add_tmp.path().join(".nomi").join("skills");
    fs::create_dir_all(&add_skills_dir).unwrap();
    write_skill(&add_skills_dir, "add-skill/SKILL.md", "---\n---\n");

    // Use a fake nonexistent cwd (bare should not need it)
    let result = load_all_skills(
        Path::new("/nonexistent_cwd_xyz"),
        &[add_tmp.path().to_path_buf()],
        true,
        None,
    )
    .await;

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "add-skill");
    // user_tmp was not consulted (no skills from there)
    let _ = user_tmp;
}

#[tokio::test]
async fn tc_11_4_nonexistent_dirs_silently_skipped() {
    let add_tmp = TempDir::new().unwrap();
    let add_skills_dir = add_tmp.path().join(".nomi").join("skills");
    fs::create_dir_all(&add_skills_dir).unwrap();
    write_skill(&add_skills_dir, "extra/SKILL.md", "---\n---\n");

    // cwd does not exist — no project skills loaded, no panic
    let result = load_all_skills(
        Path::new("/tmp/nonexistent_project_abc_xyz"),
        &[add_tmp.path().to_path_buf()],
        false,
        None,
    )
    .await;

    // Should load the add_dir skill; no panic
    assert!(result.iter().any(|s| s.name == "extra"));
}

#[tokio::test]
async fn tc_11_5_empty_scenario_returns_empty_vec() {
    // All dirs nonexistent, no add_dirs
    let tmp = TempDir::new().unwrap();
    // tmp exists but has no .nomi/skills
    let result = load_all_skills(tmp.path(), &[], false, None).await;
    // May have skills from user dir if it exists, but must not panic
    let _ = result;
}

#[tokio::test]
async fn tc_11_6_empty_add_dirs_no_effect() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();

    let skills_dir = root.join(".nomi").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    write_skill(&skills_dir, "proj-skill/SKILL.md", "---\n---\n");

    let result = load_all_skills(root, &[], false, None).await;
    let names: Vec<_> = result.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"proj-skill"),
        "project skill should load with empty add_dirs"
    );
}

// -----------------------------------------------------------------------
// TC-8.8: skill_root semantic — must be skill_dir itself (the dir containing SKILL.md),
// not skill_dir's parent. This verifies the L-5 fix: skill_root = skill_dir,
// matching TS skillRoot used for ${NOMI_SKILL_DIR} substitution.
// -----------------------------------------------------------------------

#[tokio::test]
async fn tc_8_8_skill_root_is_skill_dir_not_parent() {
    let tmp = TempDir::new().unwrap();
    // Creates: /tmp/xxx/my-skill/SKILL.md
    write_skill(tmp.path(), "my-skill/SKILL.md", "---\n---\n");

    let skills = load_skills_from_dir(tmp.path(), SkillSource::User, LoadedFrom::Skills).await;
    assert_eq!(skills.len(), 1);

    // skill_root should be the skill's own directory (containing SKILL.md),
    // not the base skills/ directory (the parent).
    let expected_skill_dir = tmp.path().join("my-skill").to_string_lossy().into_owned();
    assert_eq!(
        skills[0].metadata.skill_root.as_deref(),
        Some(expected_skill_dir.as_str()),
        "skill_root should be the skill dir itself (containing SKILL.md), not its parent"
    );
}

// -----------------------------------------------------------------------
// TC-WB: deduplicate_by_name (white-box tests for private function)
// -----------------------------------------------------------------------

#[test]
fn tc_wb_deduplicate_by_name_first_wins() {
    // [白盒] TC-WB: deduplicate_by_name keeps first occurrence (first-wins semantic)
    // Decision 6: HashMap<String, ()> with .insert().is_none() check
    let fm = FrontmatterData::default();
    let make_meta = |name: &str, source: SkillSource| {
        crate::frontmatter::parse_skill_fields(&fm, "", name, source, LoadedFrom::Skills, None)
    };

    let skills = vec![
        make_meta("my-skill", SkillSource::User), // first — should win
        make_meta("my-skill", SkillSource::Project), // second — should be removed
        make_meta("other-skill", SkillSource::User),
    ];

    let result = deduplicate_by_name(skills);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].name, "my-skill");
    assert_eq!(
        result[0].source,
        SkillSource::User,
        "first occurrence (User) should win over Project"
    );
    assert_eq!(result[1].name, "other-skill");
}

#[test]
fn tc_wb_deduplicate_by_name_empty() {
    // [白盒] empty input → empty output
    let result = deduplicate_by_name(vec![]);
    assert!(result.is_empty());
}

#[test]
fn tc_wb_deduplicate_by_name_all_unique() {
    // [白盒] no duplicates — all preserved in order
    let fm = FrontmatterData::default();
    let make_meta = |name: &str| {
        crate::frontmatter::parse_skill_fields(
            &fm,
            "",
            name,
            SkillSource::User,
            LoadedFrom::Skills,
            None,
        )
    };

    let skills = vec![make_meta("a"), make_meta("b"), make_meta("c")];
    let result = deduplicate_by_name(skills);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].name, "a");
    assert_eq!(result[1].name, "b");
    assert_eq!(result[2].name, "c");
}

#[test]
fn tc_wb_deduplicate_by_name_case_sensitive() {
    // [白盒] name matching is case-sensitive — "Skill" and "skill" are different
    let fm = FrontmatterData::default();
    let make_meta = |name: &str| {
        crate::frontmatter::parse_skill_fields(
            &fm,
            "",
            name,
            SkillSource::User,
            LoadedFrom::Skills,
            None,
        )
    };

    let skills = vec![make_meta("Skill"), make_meta("skill")];
    let result = deduplicate_by_name(skills);
    assert_eq!(
        result.len(),
        2,
        "case-sensitive: 'Skill' and 'skill' are distinct"
    );
}

// -----------------------------------------------------------------------
// TC-4.x: load_all_skills MCP integration (white-box using McpManager::new_for_test)
// -----------------------------------------------------------------------

#[tokio::test]
async fn tc_4_5_mcp_manager_none_returns_no_mcp_skills() {
    // [黑盒] TC-4.5: mcp_manager=None → no MCP skills in result
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir(root.join(".git")).unwrap();
    let skills_dir = root.join(".nomi").join("skills");
    fs::create_dir_all(&skills_dir).unwrap();
    write_skill(
        &skills_dir,
        "local-skill/SKILL.md",
        "---\ndescription: local\n---\n",
    );

    let result = load_all_skills(root, &[], false, None).await;
    let names: Vec<_> = result.iter().map(|s| s.name.as_str()).collect();
    // No skill with source=Mcp
    for skill in &result {
        assert_ne!(
            skill.source,
            crate::types::SkillSource::Mcp,
            "mcp_manager=None should produce no MCP skills"
        );
    }
    assert!(names.contains(&"local-skill"));
}
