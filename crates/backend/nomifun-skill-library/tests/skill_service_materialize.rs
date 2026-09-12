use nomifun_skill_library::{SkillPaths, skill_service};
use tempfile::TempDir;

#[tokio::test]
async fn materialize_returns_only_listed_skill_source_paths() {
    let tmp = TempDir::new().unwrap();
    // Stage two builtin auto-inject skills on disk.
    let builtin_root = tmp.path().join("builtin-skills");
    let auto_dir = builtin_root.join("auto-inject");
    std::fs::create_dir_all(auto_dir.join("cron")).unwrap();
    std::fs::write(
        auto_dir.join("cron").join("SKILL.md"),
        "---\nname: cron\ndescription: \n---",
    )
    .unwrap();
    std::fs::create_dir_all(auto_dir.join("todo")).unwrap();
    std::fs::write(
        auto_dir.join("todo").join("SKILL.md"),
        "---\nname: todo\ndescription: \n---",
    )
    .unwrap();

    let paths = SkillPaths {
        data_dir: tmp.path().to_path_buf(),
        user_skills_dir: tmp.path().join("skills"),
        cron_skills_dir: tmp.path().join("cron/skills"),
        builtin_skills_dir: builtin_root,
        builtin_rules_dir: tmp.path().join("builtin-rules"),
    };

    let resolved = skill_service::materialize_skills_for_agent(&paths, "conv-1", &["cron".to_owned()])
        .await
        .unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "cron");
    assert_eq!(resolved[0].source_path, auto_dir.join("cron"));
    assert!(resolved[0].source_path.is_dir());
    assert!(resolved[0].source_path.join("SKILL.md").exists());

    // Guardrail: the new contract forbids any per-conversation dir on
    // disk. Nothing under data_dir should have been created.
    assert!(!tmp.path().join("agent-skills").exists());
    assert!(!tmp.path().join("conversations").exists());
}
