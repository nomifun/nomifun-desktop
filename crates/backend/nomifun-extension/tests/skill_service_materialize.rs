use nomifun_extension::{resolve_skill_paths, skill_service};
use tempfile::TempDir;

/// `BUILTIN_SKILLS_ENV_VAR` is process-global; this test mutates it, so
/// it must not run in parallel with other `skill_service` tests that
/// touch the same env var. Vitest-style serialization inside a single
/// test is sufficient here.
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

    // SAFETY: single-threaded test harness.
    unsafe {
        std::env::set_var(nomifun_extension::BUILTIN_SKILLS_ENV_VAR, &builtin_root);
    }
    let paths = resolve_skill_paths(tmp.path(), tmp.path());

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

    unsafe {
        std::env::remove_var(nomifun_extension::BUILTIN_SKILLS_ENV_VAR);
    }
}
