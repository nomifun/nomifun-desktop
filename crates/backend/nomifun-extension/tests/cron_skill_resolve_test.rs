use nomifun_extension::{resolve_skill_paths, skill_service};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("nomifun-extension-{label}-{}-{nanos}", std::process::id()))
}

#[tokio::test]
async fn resolve_skill_paths_includes_cron_skills_dir() {
    let base = unique_temp_dir("cron-paths");
    std::fs::create_dir_all(&base).unwrap();

    let paths = resolve_skill_paths(&base, &base);
    assert_eq!(paths.cron_skills_dir, base.join("cron").join("skills"));

    std::fs::remove_dir_all(&base).unwrap();
}

#[tokio::test]
async fn materialize_resolves_saved_cron_skill() {
    let base = unique_temp_dir("cron-materialize");
    let skill_dir = base.join("cron").join("skills").join("cron-job-123");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: cron-job-123\ndescription: Saved cron skill\n---\nUse the saved steps.",
    )
    .unwrap();

    let paths = resolve_skill_paths(&base, &base);
    let resolved = skill_service::materialize_skills_for_agent(&paths, "conv-1", &["cron-job-123".to_owned()])
        .await
        .unwrap();

    assert_eq!(resolved.len(), 1);
    assert_eq!(resolved[0].name, "cron-job-123");
    assert_eq!(resolved[0].source_path, skill_dir);
    assert!(resolved[0].source_path.join("SKILL.md").exists());

    std::fs::remove_dir_all(&base).unwrap();
}
