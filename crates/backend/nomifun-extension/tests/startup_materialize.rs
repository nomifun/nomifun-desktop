//! Integration tests for startup materialization of the embedded
//! builtin-skills corpus. Uses a purpose-built in-test `include_dir`
//! tree so results are deterministic and independent of the real
//! embedded corpus's contents.

use std::path::Path;

use include_dir::{Dir, include_dir};
use nomifun_extension::startup_materialize::{materialize_embedded_builtin_skills, materialize_if_needed};
use tempfile::TempDir;

static FIXTURE_CORPUS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/builtin-skills-fixture");

fn read_version(target: &Path) -> Option<String> {
    std::fs::read_to_string(target.join(".version")).ok()
}

#[tokio::test]
async fn materialize_writes_tree_and_version_file() {
    let tmp = TempDir::new().unwrap();
    materialize_embedded_builtin_skills(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();

    let target = tmp.path().join("builtin-skills");
    assert!(target.is_dir(), "target dir should exist");
    assert_eq!(read_version(&target).as_deref(), Some("1.2.3"));
    assert!(
        target.join("example-skill").join("SKILL.md").is_file(),
        "expected fixture skill to be materialized"
    );
}

#[tokio::test]
async fn materialize_overwrites_existing_target() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("builtin-skills");
    std::fs::create_dir_all(target.join("stale-dir")).unwrap();
    std::fs::write(target.join("junk.txt"), b"old").unwrap();

    materialize_embedded_builtin_skills(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();

    assert!(!target.join("junk.txt").exists(), "stale file should be gone");
    assert!(!target.join("stale-dir").exists(), "stale dir should be gone");
    assert_eq!(read_version(&target).as_deref(), Some("1.2.3"));
}

#[tokio::test]
async fn materialize_cleans_staging_from_prior_crash() {
    let tmp = TempDir::new().unwrap();
    let staging = tmp.path().join(".builtin-skills.tmp");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("leftover.txt"), b"x").unwrap();

    materialize_embedded_builtin_skills(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();

    assert!(!staging.exists(), "staging should be cleaned after success");
}

#[tokio::test]
async fn gate_skips_when_version_matches() {
    let tmp = TempDir::new().unwrap();
    // Pre-populate target with matching version.
    let target = tmp.path().join("builtin-skills");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(".version"), "1.2.3").unwrap();
    std::fs::write(target.join("sentinel"), "do-not-delete").unwrap();

    let wrote = materialize_if_needed(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();
    assert!(!wrote, "version match should skip materialize");
    assert!(
        target.join("sentinel").is_file(),
        "sentinel must be preserved when gate says skip"
    );
}

#[tokio::test]
async fn gate_triggers_when_version_mismatches() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("builtin-skills");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(".version"), "0.0.0").unwrap();
    std::fs::write(target.join("sentinel"), "will-be-wiped").unwrap();

    let wrote = materialize_if_needed(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();
    assert!(wrote, "version mismatch should materialize");
    assert!(
        !target.join("sentinel").exists(),
        "sentinel must be wiped by fresh materialize"
    );
    assert_eq!(read_version(&target).as_deref(), Some("1.2.3"));
}

#[tokio::test]
async fn gate_triggers_when_version_file_missing() {
    let tmp = TempDir::new().unwrap();

    let wrote = materialize_if_needed(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();
    assert!(wrote, "no existing version should materialize");
    assert_eq!(
        read_version(&tmp.path().join("builtin-skills")).as_deref(),
        Some("1.2.3")
    );
}

#[tokio::test]
async fn gate_keeps_existing_tree_when_refresh_fails() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("builtin-skills");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::write(target.join(".version"), "0.0.0").unwrap();
    std::fs::write(target.join("sentinel"), "keep-existing").unwrap();
    std::fs::write(tmp.path().join(".builtin-skills.tmp"), "stale-file").unwrap();

    let wrote = materialize_if_needed(tmp.path(), &FIXTURE_CORPUS, "1.2.3")
        .await
        .unwrap();

    assert!(!wrote, "refresh failure should fall back to existing tree");
    assert_eq!(read_version(&target).as_deref(), Some("0.0.0"));
    assert!(
        target.join("sentinel").is_file(),
        "existing tree must be preserved when refresh fails"
    );
}

#[tokio::test]
async fn gate_fails_when_initial_materialize_fails_without_existing_tree() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join(".builtin-skills.tmp"), "stale-file").unwrap();

    let result = materialize_if_needed(tmp.path(), &FIXTURE_CORPUS, "1.2.3").await;

    assert!(
        result.is_err(),
        "first startup must not silently continue without a usable builtin-skills tree"
    );
}

#[tokio::test]
async fn concurrent_materialize_produces_consistent_tree() {
    // Two concurrent invocations share the same staging/target paths, so
    // at most one reliably wins the atomic rename; the other may legally
    // fail mid-staging. What matters is that after both complete, the
    // on-disk tree is in a consistent, fully-populated state driven by
    // whichever call succeeded.
    let tmp = TempDir::new().unwrap();
    let dir1 = tmp.path().to_path_buf();
    let dir2 = tmp.path().to_path_buf();

    let (a, b) = tokio::join!(
        materialize_embedded_builtin_skills(&dir1, &FIXTURE_CORPUS, "1.2.3"),
        materialize_embedded_builtin_skills(&dir2, &FIXTURE_CORPUS, "1.2.3"),
    );
    assert!(
        a.is_ok() || b.is_ok(),
        "at least one concurrent materialize must succeed; a={a:?}, b={b:?}"
    );

    let target = tmp.path().join("builtin-skills");
    assert_eq!(read_version(&target).as_deref(), Some("1.2.3"));
    assert!(target.join("example-skill").join("SKILL.md").is_file());
}

#[tokio::test]
async fn concurrent_gate_materialize_all_callers_succeed() {
    let tmp = TempDir::new().unwrap();
    let data_dir = tmp.path().to_path_buf();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let dir = data_dir.clone();
        handles.push(tokio::spawn(async move {
            materialize_if_needed(&dir, &FIXTURE_CORPUS, "1.2.3").await
        }));
    }
    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await.unwrap());
    }

    for result in &results {
        assert!(
            result.is_ok(),
            "concurrent startup materialize callers should not fail: {results:?}"
        );
    }
    assert!(
        results.iter().filter(|result| matches!(result, Ok(true))).count() >= 1,
        "at least one caller should perform the materialize write: {results:?}"
    );

    let target = tmp.path().join("builtin-skills");
    assert_eq!(read_version(&target).as_deref(), Some("1.2.3"));
    assert!(target.join("example-skill").join("SKILL.md").is_file());
}
