//! Black-box integration tests for skill & rule management (test-plan SM, RM, CP).
//!
//! These tests exercise the public API surface of `nomifun_extension::skill_service`
//! and `nomifun_extension::external_paths` against the functional requirements in
//! `08-file-workspace.md` §B.

use std::path::Path;

use nomifun_extension::external_paths::ExternalPathsManager;
use nomifun_extension::skill_service::{
    NamedPath, SkillPaths, delete_assistant_rule, delete_assistant_skill, delete_skill,
    detect_and_count_external_skills, export_skill_with_symlink, import_skill, import_skill_with_symlink,
    list_available_skills, read_assistant_rule, read_assistant_skill, read_builtin_rule, read_builtin_skill,
    read_skill_info, resolve_skill_paths, scan_for_skills, write_assistant_rule, write_assistant_skill,
};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

const SKILL_MD: &str = "SKILL.md";

fn make_paths(base: &Path) -> SkillPaths {
    SkillPaths {
        data_dir: base.to_path_buf(),
        user_skills_dir: base.join("skills"),
        cron_skills_dir: base.join("cron").join("skills"),
        builtin_skills_dir: base.join("builtin-skills"),
        builtin_rules_dir: base.join("builtin-rules"),
        assistant_rules_dir: base.join("assistant-rules"),
        assistant_skills_dir: base.join("assistant-skills"),
    }
}

fn builtin_dir(paths: &SkillPaths) -> &Path {
    &paths.builtin_skills_dir
}

fn create_skill(base: &Path, name: &str, desc: &str) {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(SKILL_MD),
        format!("---\nname: {name}\ndescription: {desc}\n---\nBody of {name}."),
    )
    .unwrap();
}

fn create_builtin_rule(base: &Path, name: &str, content: &str) {
    std::fs::create_dir_all(base).unwrap();
    std::fs::write(base.join(name), content).unwrap();
}

fn create_builtin_skill(base: &Path, name: &str, content: &str) {
    std::fs::create_dir_all(base).unwrap();
    std::fs::write(base.join(name), content).unwrap();
}

// ===========================================================================
// SM — Skill Management
// ===========================================================================

/// SM-1: List available skills (builtin + custom, deduplication).
#[tokio::test]
async fn sm1_list_available_skills_deduplication() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    // 3 built-in, 2 custom (1 overlaps)
    create_skill(builtin_dir(&paths), "review", "Built-in review");
    create_skill(builtin_dir(&paths), "debug", "Built-in debug");
    create_skill(builtin_dir(&paths), "test", "Built-in test");
    create_skill(&paths.user_skills_dir, "review", "Custom review override");
    create_skill(&paths.user_skills_dir, "my-tool", "My custom tool");

    let skills = list_available_skills(&paths).await.unwrap();

    // 4 total: debug(builtin) + my-tool(custom) + review(custom) + test(builtin)
    assert_eq!(skills.len(), 4);

    // Verify deduplication: review should be the custom version
    let review = skills.iter().find(|s| s.name == "review").unwrap();
    assert!(review.is_custom);
    assert_eq!(review.description, "Custom review override");

    // Verify each has required fields
    for s in &skills {
        assert!(!s.name.is_empty());
        assert!(!s.description.is_empty());
        assert!(!s.location.is_empty());
    }
}

/// SM-2: Read skill info from path.
#[tokio::test]
async fn sm2_read_skill_info() {
    let tmp = TempDir::new().unwrap();
    let skill_dir = tmp.path().join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join(SKILL_MD),
        "---\nname: my-skill\ndescription: A test skill\n---\nBody content.",
    )
    .unwrap();

    let (name, desc) = read_skill_info(&skill_dir).await.unwrap();
    assert_eq!(name, "my-skill");
    assert_eq!(desc, "A test skill");
}

/// SM-2 error: Path does not exist → error.
#[tokio::test]
async fn sm2_read_skill_info_not_found() {
    let result = read_skill_info(Path::new("/nonexistent/skill")).await;
    assert!(result.is_err());
}

/// SM-3: Import skill (copy).
#[tokio::test]
async fn sm3_import_skill_copy() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    let source = tmp.path().join("external-skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join(SKILL_MD),
        "---\nname: ext-tool\ndescription: External tool\n---\nContent.",
    )
    .unwrap();
    std::fs::write(source.join("helper.py"), "print('hello')").unwrap();

    let name = import_skill(&paths, &source).await.unwrap();
    assert_eq!(name, "ext-tool");

    // Verify files were copied
    let imported = paths.user_skills_dir.join("ext-tool");
    assert!(imported.join(SKILL_MD).exists());
    assert!(imported.join("helper.py").exists());
}

/// SM-4: Import skill (symlink).
#[tokio::test]
async fn sm4_import_skill_symlink() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    let source = tmp.path().join("linked-skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join(SKILL_MD),
        "---\nname: linked\ndescription: Linked skill\n---\nBody.",
    )
    .unwrap();

    let name = import_skill_with_symlink(&paths, &source).await.unwrap();
    assert_eq!(name, "linked");

    let link = paths.user_skills_dir.join("linked");
    assert!(link.is_symlink());

    // Verify content is accessible through the symlink
    let content = std::fs::read_to_string(link.join(SKILL_MD)).unwrap();
    assert!(content.contains("Linked skill"));
}

/// SM-5: Export skill (symlink).
#[tokio::test]
async fn sm5_export_skill_symlink() {
    let tmp = TempDir::new().unwrap();

    let source = tmp.path().join("source-skill");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        source.join(SKILL_MD),
        "---\nname: source\ndescription: Source\n---\nBody.",
    )
    .unwrap();

    let export_dir = tmp.path().join("exports");
    export_skill_with_symlink(&source, &export_dir).await.unwrap();

    let link = export_dir.join("source-skill");
    assert!(link.is_symlink());
}

/// SM-6: Delete user custom skill.
#[tokio::test]
async fn sm6_delete_custom_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    create_skill(&paths.user_skills_dir, "deletable", "Will be deleted");
    assert!(paths.user_skills_dir.join("deletable").exists());

    delete_skill(&paths, "deletable").await.unwrap();
    assert!(!paths.user_skills_dir.join("deletable").exists());
}

/// SM-7: Delete built-in skill → rejected.
#[tokio::test]
async fn sm7_delete_builtin_skill_rejected() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    create_skill(builtin_dir(&paths), "protected", "Cannot delete");

    let result = delete_skill(&paths, "protected").await;
    assert!(result.is_err());

    // Verify it still exists
    assert!(builtin_dir(&paths).join("protected").exists());
}

/// SM-8: Scan for skills in a directory.
#[tokio::test]
async fn sm8_scan_for_skills() {
    let tmp = TempDir::new().unwrap();
    let scan_dir = tmp.path().join("scan-target");

    create_skill(&scan_dir, "alpha", "Alpha skill");
    create_skill(&scan_dir, "beta", "Beta skill");
    // Directory without SKILL.md
    std::fs::create_dir_all(scan_dir.join("not-a-skill")).unwrap();

    let skills = scan_for_skills(&scan_dir).await.unwrap();
    assert_eq!(skills.len(), 2);
    assert!(skills.iter().any(|s| s.name == "alpha"));
    assert!(skills.iter().any(|s| s.name == "beta"));
}

/// SM-11: Get skill directory paths.
///
/// Production mode: no `NOMIFUN_BUILTIN_SKILLS_PATH` set — the built-in
/// skills tree lives at `{data_dir}/builtin-skills/`, populated at
/// startup by `startup_materialize::materialize_if_needed`. The user
/// skills directory is derived from `data_dir`, not `resource_dir`.
#[tokio::test]
async fn sm11_get_skill_paths() {
    // Ensure the env var is unset for a deterministic assertion.
    // Safe: the test runs single-threaded w.r.t. this env var.
    // (SAFETY: `remove_var` is unsafe in 2024 edition due to process-wide
    // side-effects.)
    unsafe {
        std::env::remove_var("NOMIFUN_BUILTIN_SKILLS_PATH");
    }

    let resource_dir = Path::new("/app/resources");
    let data_dir = Path::new("/home/user/.nomifun");
    let paths = resolve_skill_paths(resource_dir, data_dir);

    assert!(paths.user_skills_dir.to_string_lossy().contains("skills"));
    assert_eq!(
        paths.builtin_skills_dir,
        data_dir.join("builtin-skills"),
        "production mode must resolve builtin_skills_dir under data_dir"
    );
}

// ===========================================================================
// RM — Rule Management
// ===========================================================================

/// RM-1: Read built-in rule.
#[tokio::test]
async fn rm1_read_builtin_rule() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    create_builtin_rule(&paths.builtin_rules_dir, "code-review.md", "# Review Rules");

    let content = read_builtin_rule(&paths, "code-review.md").await.unwrap();
    assert_eq!(content, "# Review Rules");
}

/// RM-1 error: File not found → empty string.
#[tokio::test]
async fn rm1_read_builtin_rule_not_found() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    let content = read_builtin_rule(&paths, "nonexistent.md").await.unwrap();
    assert!(content.is_empty());
}

/// RM-1 variant: Read built-in skill.
#[tokio::test]
async fn rm1_read_builtin_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    create_builtin_skill(builtin_dir(&paths), "tdd.md", "# TDD Workflow");

    let content = read_builtin_skill(&paths, "tdd.md").await.unwrap();
    assert_eq!(content, "# TDD Workflow");
}

/// RM-2: Read assistant rule with locale fallback.
#[tokio::test]
async fn rm2_assistant_rule_locale_fallback() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    // Write both default and locale-specific
    write_assistant_rule(&paths, "abc123", "Default rule", None)
        .await
        .unwrap();
    write_assistant_rule(&paths, "abc123", "中文规则", Some("zh-CN"))
        .await
        .unwrap();

    // 1. Matching locale → locale-specific content
    let content = read_assistant_rule(&paths, "abc123", Some("zh-CN")).await.unwrap();
    assert_eq!(content, "中文规则");

    // 2. Non-matching locale → fallback to default
    let content = read_assistant_rule(&paths, "abc123", Some("en-US")).await.unwrap();
    assert_eq!(content, "Default rule");

    // 3. No locale → default
    let content = read_assistant_rule(&paths, "abc123", None).await.unwrap();
    assert_eq!(content, "Default rule");

    // 4. Not found → empty string
    let content = read_assistant_rule(&paths, "missing", None).await.unwrap();
    assert!(content.is_empty());
}

/// RM-3: Write assistant rule.
#[tokio::test]
async fn rm3_write_assistant_rule() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    let result = write_assistant_rule(&paths, "abc123", "New rule content", Some("en-US"))
        .await
        .unwrap();
    assert!(result);

    // Verify file created
    let file = paths.assistant_rules_dir.join("abc123.en-US.md");
    assert!(file.exists());
    let content = std::fs::read_to_string(file).unwrap();
    assert_eq!(content, "New rule content");
}

/// RM-4: Delete assistant rule (all locales).
#[tokio::test]
async fn rm4_delete_assistant_rule_all_locales() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    write_assistant_rule(&paths, "abc123", "Default", None).await.unwrap();
    write_assistant_rule(&paths, "abc123", "Chinese", Some("zh-CN"))
        .await
        .unwrap();
    write_assistant_rule(&paths, "abc123", "English", Some("en-US"))
        .await
        .unwrap();

    let deleted = delete_assistant_rule(&paths, "abc123").await.unwrap();
    assert!(deleted);

    // Verify all versions removed
    let content = read_assistant_rule(&paths, "abc123", None).await.unwrap();
    assert!(content.is_empty());
    let content = read_assistant_rule(&paths, "abc123", Some("zh-CN")).await.unwrap();
    assert!(content.is_empty());
    let content = read_assistant_rule(&paths, "abc123", Some("en-US")).await.unwrap();
    assert!(content.is_empty());
}

/// RM-5: Read assistant skill with locale fallback (same as RM-2 pattern).
#[tokio::test]
async fn rm5_assistant_skill_locale_fallback() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    write_assistant_skill(&paths, "abc123", "Default skill", None)
        .await
        .unwrap();
    write_assistant_skill(&paths, "abc123", "English skill", Some("en-US"))
        .await
        .unwrap();

    let content = read_assistant_skill(&paths, "abc123", Some("en-US")).await.unwrap();
    assert_eq!(content, "English skill");

    let content = read_assistant_skill(&paths, "abc123", Some("fr-FR")).await.unwrap();
    assert_eq!(content, "Default skill");
}

/// RM-6: Write and delete assistant skill.
#[tokio::test]
async fn rm6_write_and_delete_assistant_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    write_assistant_skill(&paths, "abc123", "Content", None).await.unwrap();
    write_assistant_skill(&paths, "abc123", "Locale", Some("zh-CN"))
        .await
        .unwrap();

    let deleted = delete_assistant_skill(&paths, "abc123").await.unwrap();
    assert!(deleted);

    let content = read_assistant_skill(&paths, "abc123", None).await.unwrap();
    assert!(content.is_empty());
}

// ===========================================================================
// CP — Custom External Paths
// ===========================================================================

/// CP-1: Get custom paths (initially empty).
#[tokio::test]
async fn cp1_get_custom_paths_empty() {
    let tmp = TempDir::new().unwrap();
    let mgr = ExternalPathsManager::new(tmp.path()).await;

    let paths = mgr.get_custom_external_paths().await;
    assert!(paths.is_empty());
}

/// CP-2: Add custom path and verify persistence.
#[tokio::test]
async fn cp2_add_custom_path() {
    let tmp = TempDir::new().unwrap();
    let mgr = ExternalPathsManager::new(tmp.path()).await;

    mgr.add_custom_external_path("My Skills", "/home/user/skills")
        .await
        .unwrap();

    let paths = mgr.get_custom_external_paths().await;
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].name, "My Skills");
    assert_eq!(paths[0].path, "/home/user/skills");

    // Verify persistence across reload
    drop(mgr);
    let mgr2 = ExternalPathsManager::new(tmp.path()).await;
    let paths = mgr2.get_custom_external_paths().await;
    assert_eq!(paths.len(), 1);
}

/// CP-3: Remove custom path.
#[tokio::test]
async fn cp3_remove_custom_path() {
    let tmp = TempDir::new().unwrap();
    let mgr = ExternalPathsManager::new(tmp.path()).await;

    mgr.add_custom_external_path("A", "/path/a").await.unwrap();
    mgr.add_custom_external_path("B", "/path/b").await.unwrap();

    mgr.remove_custom_external_path("/path/a").await.unwrap();

    let paths = mgr.get_custom_external_paths().await;
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].path, "/path/b");
}

/// CP-4: Enable skills market.
#[tokio::test]
async fn cp4_enable_skills_market() {
    let tmp = TempDir::new().unwrap();
    let mgr = ExternalPathsManager::new(tmp.path()).await;

    mgr.enable_skills_market().await.unwrap();

    let paths = mgr.get_custom_external_paths().await;
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].name, "nomifun-skills");
}

/// CP-5: Disable skills market.
#[tokio::test]
async fn cp5_disable_skills_market() {
    let tmp = TempDir::new().unwrap();
    let mgr = ExternalPathsManager::new(tmp.path()).await;

    mgr.enable_skills_market().await.unwrap();
    mgr.disable_skills_market().await.unwrap();

    let paths = mgr.get_custom_external_paths().await;
    assert!(paths.is_empty());
}

// ===========================================================================
// External skill discovery
// ===========================================================================

/// Test detect_and_count_external_skills with custom paths.
#[tokio::test]
async fn detect_external_skills_from_custom_paths() {
    let tmp = TempDir::new().unwrap();
    let ext_dir = tmp.path().join("external-skills");
    create_skill(&ext_dir, "ext-a", "External A");
    create_skill(&ext_dir, "ext-b", "External B");

    let custom_paths = vec![NamedPath {
        name: "External".to_string(),
        path: ext_dir.to_string_lossy().into_owned(),
    }];

    let sources = detect_and_count_external_skills(&custom_paths).await;

    // Should have at least the custom path source
    let external = sources
        .iter()
        .find(|s| s.name == "External")
        .expect("custom external source should be found");
    assert_eq!(external.skill_count, 2);
    assert!(external.skills.iter().any(|s| s.name == "ext-a"));
    assert!(external.skills.iter().any(|s| s.name == "ext-b"));
    // `source` for custom paths is `custom-<abs-path>` — used by the renderer
    // as a React key / testid suffix, and asserted by e2e spec
    // `edge-cases.e2e.ts` (prefix `external-source-tab-custom-`).
    assert_eq!(external.source, format!("custom-{}", ext_dir.to_string_lossy()));
    assert!(external.source.starts_with("custom-"));
}

/// Custom paths with distinct filesystem locations get distinct slugs so
/// the renderer can use them as unique React keys / testid suffixes.
#[tokio::test]
async fn detect_external_skills_custom_sources_are_unique() {
    let tmp = TempDir::new().unwrap();
    let dir_a = tmp.path().join("a");
    let dir_b = tmp.path().join("b");
    create_skill(&dir_a, "a-skill", "A");
    create_skill(&dir_b, "b-skill", "B");

    let custom_paths = vec![
        NamedPath {
            name: "A".into(),
            path: dir_a.to_string_lossy().into_owned(),
        },
        NamedPath {
            name: "B".into(),
            path: dir_b.to_string_lossy().into_owned(),
        },
    ];

    let sources = detect_and_count_external_skills(&custom_paths).await;
    let slugs: Vec<&str> = sources
        .iter()
        .filter(|s| s.name == "A" || s.name == "B")
        .map(|s| s.source.as_str())
        .collect();
    assert_eq!(slugs.len(), 2);
    assert_ne!(slugs[0], slugs[1]);
}

/// Verify path traversal is blocked in skill deletion.
#[tokio::test]
async fn security_path_traversal_blocked() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    // Path traversal attempts
    assert!(delete_skill(&paths, "../escape").await.is_err());
    assert!(delete_skill(&paths, "foo/bar").await.is_err());
    assert!(delete_skill(&paths, "foo\\bar").await.is_err());
}

/// Verify built-in resource reads block path traversal.
#[tokio::test]
async fn security_builtin_read_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    assert!(read_builtin_rule(&paths, "../secret.md").await.is_err());
    assert!(read_builtin_skill(&paths, "../../etc/passwd").await.is_err());
    assert!(read_builtin_rule(&paths, "").await.is_err());
}

/// Verify assistant CRUD functions block path traversal in assistant_id.
#[tokio::test]
async fn security_assistant_crud_path_traversal_id() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    // read
    assert!(read_assistant_rule(&paths, "../escape", None).await.is_err());
    assert!(read_assistant_skill(&paths, "foo/bar", None).await.is_err());

    // write
    assert!(write_assistant_rule(&paths, "../escape", "x", None).await.is_err());
    assert!(write_assistant_skill(&paths, "foo\\bar", "x", None).await.is_err());

    // delete
    assert!(delete_assistant_rule(&paths, "../escape").await.is_err());
    assert!(delete_assistant_skill(&paths, "a/b").await.is_err());
}

/// Verify assistant read/write functions block path traversal in locale.
#[tokio::test]
async fn security_assistant_crud_path_traversal_locale() {
    let tmp = TempDir::new().unwrap();
    let paths = make_paths(tmp.path());

    assert!(read_assistant_rule(&paths, "valid", Some("../bad")).await.is_err());
    assert!(
        write_assistant_rule(&paths, "valid", "x", Some("../../evil"))
            .await
            .is_err()
    );
    assert!(read_assistant_skill(&paths, "valid", Some("a/b")).await.is_err());
    assert!(write_assistant_skill(&paths, "valid", "x", Some("a\\b")).await.is_err());
}
