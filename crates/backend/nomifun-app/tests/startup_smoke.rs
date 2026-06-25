//! Smoke test: starting the app twice with the same binary version
//! should be a no-op on the second run (version gate skips rewrite).

use tempfile::TempDir;

#[tokio::test]
async fn second_start_with_same_version_is_noop() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(first, "first call should materialize");

    let second =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(!second, "second call with same version should skip");
}

#[tokio::test]
async fn version_bump_triggers_rewrite() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path();

    let first =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-1.0.0")
            .await
            .unwrap();
    assert!(first);

    let second =
        nomifun_extension::materialize_if_needed(data_dir, nomifun_extension::builtin_skills_corpus(), "test-2.0.0")
            .await
            .unwrap();
    assert!(second, "version change should trigger a fresh materialize");

    let version = std::fs::read_to_string(data_dir.join("builtin-skills").join(".version")).unwrap();
    assert_eq!(version, "test-2.0.0");
}
