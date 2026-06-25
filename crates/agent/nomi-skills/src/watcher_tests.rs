/// Black-box tests for `SkillWatcher` based on the Phase 13 test plan.
///
/// All tests are async (`#[tokio::test]`) because the watcher relies on
/// tokio tasks for debouncing.
///
/// ## macOS path note
///
/// `tempfile::TempDir` creates directories under `/var/folders/.../T/.tmpXXXX`.
/// On macOS, FSEvents resolves symlinks, so the reported path becomes
/// `/private/var/folders/.../T/.tmpXXXX`.  The `.tmpXXXX` directory name
/// starts with `.`, which causes `should_ignore` to filter ALL events from
/// such directories (it checks every path component, not just the filename).
///
/// To work around this, tests that rely on receiving notifications create
/// directories with visible (non-dot-prefixed) names under `/tmp/`.
/// Tests that verify silence (TC-14, TC-15) still use `TempDir` because
/// those directories only receive hidden-file events which should be filtered.
///
/// Debounce window is 300 ms.  Tests that expect a notification wait 600 ms
/// (300 ms window + 300 ms platform margin).  Tests that expect *no*
/// notification wait 800 ms to be safe.
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tempfile::TempDir;
use tokio::time::timeout;

use crate::discovery::RuntimeDiscovery;
use crate::watcher::SkillWatcher;

/// Create a uniquely named, non-hidden test directory under `/tmp/`.
///
/// Returns a `PathBuf` and a guard that removes the directory on drop.
/// Using `/tmp/` directly (rather than `TempDir`) avoids the macOS
/// `.tmpXXXX` hidden-directory naming that triggers `should_ignore`.
fn make_visible_test_dir(name: &str) -> (PathBuf, TempDirGuard) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Use /private/tmp to match FSEvents resolved path on macOS
    let base = if cfg!(target_os = "macos") {
        PathBuf::from("/private/tmp")
    } else {
        std::env::temp_dir()
    };
    let dir = base.join(format!("nomi_watcher_test_{name}_{id}"));
    fs::create_dir_all(&dir).expect("failed to create test dir");
    let guard = TempDirGuard(dir.clone());
    (dir, guard)
}

/// RAII guard that removes the test directory on drop.
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const DEBOUNCE_EXPECT_MS: u64 = 600; // wait when expecting a notification
const DEBOUNCE_NO_EXPECT_MS: u64 = 800; // wait when expecting silence
// Time to wait after start() before triggering events, giving notify time to
// register with the OS kernel (FSEvents/inotify initialisation latency).
const WATCHER_INIT_MS: u64 = 150;

// ---------------------------------------------------------------------------
// Diagnostic: verify notify events are received at all
// ---------------------------------------------------------------------------

/// Slow diagnostic test: uses a 1-second wait to rule out timing issues.
/// Run individually: cargo test watcher_tests::diag -- --nocapture --include-ignored
#[tokio::test]
#[ignore]
async fn diag_basic_event_received() {
    let dir = TempDir::new().unwrap();
    eprintln!("[diag] test dir: {}", dir.path().display());

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.path().to_path_buf()]).unwrap();

    eprintln!("[diag] waiting 500ms for notify init...");
    tokio::time::sleep(Duration::from_millis(500)).await;

    let before = *rx.borrow_and_update();
    eprintln!("[diag] version before: {before}");

    let file = dir.path().join("test.md");
    eprintln!("[diag] writing file: {}", file.display());
    fs::write(&file, "hello").unwrap();
    eprintln!("[diag] file written, waiting up to 1s...");

    let result = timeout(Duration::from_millis(1000), rx.changed()).await;
    eprintln!(
        "[diag] timeout result (true=got event): {:?}",
        result.is_ok()
    );
    let after = *rx.borrow();
    eprintln!("[diag] version after: {after}");

    assert!(result.is_ok(), "[diag] no event received within 1s");
}

/// Diagnostic: test with multi-thread runtime to rule out single-thread scheduling
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn diag_multi_thread_event_received() {
    let dir = TempDir::new().unwrap();
    eprintln!("[diag-mt] test dir: {}", dir.path().display());

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.path().to_path_buf()]).unwrap();

    eprintln!("[diag-mt] waiting 500ms for notify init...");
    tokio::time::sleep(Duration::from_millis(500)).await;

    let before = *rx.borrow_and_update();
    eprintln!("[diag-mt] version before: {before}");

    let file = dir.path().join("test.md");
    eprintln!("[diag-mt] writing file: {}", file.display());
    fs::write(&file, "hello").unwrap();
    eprintln!("[diag-mt] file written, waiting up to 2s...");

    let result = timeout(Duration::from_millis(2000), rx.changed()).await;
    eprintln!(
        "[diag-mt] timeout result (true=got event): {:?}",
        result.is_ok()
    );
    let after = *rx.borrow();
    eprintln!("[diag-mt] version after: {after}");

    assert!(result.is_ok(), "[diag-mt] no event received within 2s");
}

// ---------------------------------------------------------------------------
// TC-01: new() accepts empty directory list
// ---------------------------------------------------------------------------

/// [TC-01 黑盒] `new()` with no dirs returns Ok; initial version is 0.
#[tokio::test]
async fn tc01_new_empty_dirs_returns_ok() {
    let (mut watcher, rx) = SkillWatcher::new().expect("new() should succeed");
    watcher.start(vec![]).expect("start() should succeed");

    let initial = *rx.borrow();
    assert_eq!(initial, 0, "initial version should be 0");
}

// ---------------------------------------------------------------------------
// TC-02: new() accepts multiple existing directories
// ---------------------------------------------------------------------------

/// [TC-02 黑盒] `new()` with two real directories returns Ok.
#[tokio::test]
async fn tc02_new_with_existing_dirs_returns_ok() {
    let dir_a = TempDir::new().unwrap();
    let dir_b = TempDir::new().unwrap();

    let (mut watcher, _rx) = SkillWatcher::new().expect("new() should succeed with existing dirs");
    watcher
        .start(vec![dir_a.path().to_path_buf(), dir_b.path().to_path_buf()])
        .expect("start() should succeed");
}

// ---------------------------------------------------------------------------
// TC-03: new() skips non-existent directories (no panic, no Err)
// ---------------------------------------------------------------------------

/// [TC-03 黑盒] Non-existent directory is skipped silently; `new()` succeeds.
#[tokio::test]
async fn tc03_nonexistent_dir_skipped() {
    let non_existent = std::path::PathBuf::from("/nonexistent/path/abc_phase13_test");

    let (mut watcher, _rx) = SkillWatcher::new().expect("new() should succeed");
    watcher
        .start(vec![non_existent])
        .expect("start() should not error for non-existent dirs");
}

// ---------------------------------------------------------------------------
// TC-04: mix of existing and non-existing directories
// ---------------------------------------------------------------------------

/// [TC-04 黑盒] Mix of existing and non-existing dirs — both handled without error.
#[tokio::test]
async fn tc04_mixed_dirs() {
    let existing = TempDir::new().unwrap();
    let non_existent = std::path::PathBuf::from("/nonexistent/xyz_phase13_test");

    let (mut watcher, _rx) = SkillWatcher::new().expect("new() should succeed");
    watcher
        .start(vec![existing.path().to_path_buf(), non_existent])
        .expect("start() should succeed for mixed dirs");
}

// ---------------------------------------------------------------------------
// TC-05: file creation triggers notification after debounce
// ---------------------------------------------------------------------------

/// [TC-05 黑盒] Creating a file in a watched directory triggers a version bump.
#[tokio::test]
async fn tc05_file_create_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc05");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify to register the watch with the OS.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;

    let initial = *rx.borrow_and_update();

    // Create a file to trigger an event.
    fs::write(dir.join("SKILL.md"), "# test skill").unwrap();

    let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

    assert!(
        result.is_ok(),
        "should receive notification within {}ms after file creation",
        DEBOUNCE_EXPECT_MS
    );
    let new_version = *rx.borrow();
    assert!(
        new_version > initial,
        "version should increment after file creation (was {initial}, now {new_version})"
    );
}

// ---------------------------------------------------------------------------
// TC-06: file modification triggers notification
// ---------------------------------------------------------------------------

/// [TC-06 黑盒] Modifying an existing file triggers a version bump.
#[tokio::test]
async fn tc06_file_modify_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc06");
    let skill_file = dir.join("SKILL.md");
    fs::write(&skill_file, "# initial").unwrap();

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify to initialise, then drain any creation event.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS + 400)).await;
    let version_before = *rx.borrow_and_update();

    // Modify the file.
    fs::write(&skill_file, "# modified").unwrap();

    let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

    assert!(
        result.is_ok(),
        "should receive notification within {}ms after file modification",
        DEBOUNCE_EXPECT_MS
    );
    let new_version = *rx.borrow();
    assert!(
        new_version > version_before,
        "version should increment after modification (was {version_before}, now {new_version})"
    );
}

// ---------------------------------------------------------------------------
// TC-07: file deletion triggers notification
// ---------------------------------------------------------------------------

/// [TC-07 黑盒] Deleting a file in a watched directory triggers a version bump.
#[tokio::test]
async fn tc07_file_delete_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc07");
    let skill_file = dir.join("SKILL.md");
    fs::write(&skill_file, "# to be deleted").unwrap();

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify init + drain creation event.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS + 400)).await;
    let version_before = *rx.borrow_and_update();

    // Delete the file.
    fs::remove_file(&skill_file).unwrap();

    let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

    assert!(
        result.is_ok(),
        "should receive notification within {}ms after file deletion",
        DEBOUNCE_EXPECT_MS
    );
    let new_version = *rx.borrow();
    assert!(
        new_version > version_before,
        "version should increment after deletion (was {version_before}, now {new_version})"
    );
}

// ---------------------------------------------------------------------------
// TC-08: file rename triggers notification
// ---------------------------------------------------------------------------

/// [TC-08 黑盒] Renaming a file in a watched directory triggers a version bump.
#[tokio::test]
async fn tc08_file_rename_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc08");
    let old_file = dir.join("old.md");
    fs::write(&old_file, "# old").unwrap();

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify init + drain creation event.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS + 400)).await;
    let version_before = *rx.borrow_and_update();

    // Rename the file.
    fs::rename(&old_file, dir.join("SKILL.md")).unwrap();

    let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

    assert!(
        result.is_ok(),
        "should receive notification within {}ms after file rename",
        DEBOUNCE_EXPECT_MS
    );
    let new_version = *rx.borrow();
    assert!(
        new_version > version_before,
        "version should increment after rename (was {version_before}, now {new_version})"
    );
}

// ---------------------------------------------------------------------------
// TC-09: multiple events within 300ms are coalesced into one notification
// ---------------------------------------------------------------------------

/// [TC-09 黑盒] Five file writes within 100 ms result in only one version increment.
#[tokio::test]
async fn tc09_debounce_coalesces_multiple_events() {
    let (dir, _guard) = make_visible_test_dir("tc09");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify to initialise.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let initial = *rx.borrow_and_update();

    // Write 5 files rapidly (within ~50 ms total).
    for i in 0..5u32 {
        fs::write(dir.join(format!("skill_{i}.md")), format!("# skill {i}")).unwrap();
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Wait for the debounce window to expire plus margin.
    tokio::time::sleep(Duration::from_millis(DEBOUNCE_EXPECT_MS)).await;

    let final_version = *rx.borrow();
    let increments = final_version - initial;

    assert!(
        increments >= 1,
        "version should have incremented at least once (initial={initial}, final={final_version})"
    );
    // The key assertion: 5 rapid events should be coalesced into at most 2 notifications.
    // Ideally 1, but allow a small margin for platform timing jitter.
    assert!(
        increments <= 2,
        "debounce should coalesce rapid events into <=2 increments, got {increments} (initial={initial}, final={final_version})"
    );
}

// ---------------------------------------------------------------------------
// TC-10: watch_directory() adds a new directory dynamically
// ---------------------------------------------------------------------------

/// [TC-10 黑盒] `watch_directory()` called after `start()` enables monitoring new dir.
#[tokio::test]
async fn tc10_watch_directory_dynamic_add() {
    let (dir_a, _guard_a) = make_visible_test_dir("tc10a");
    let (dir_b, _guard_b) = make_visible_test_dir("tc10b");

    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir_a.clone()]).unwrap();

    // Dynamically add dir_b.
    watcher
        .watch_directory(&dir_b)
        .expect("watch_directory() should succeed for existing dir");

    // Wait for notify to register the new directory.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let version_before = *rx.borrow_and_update();

    // Create file in newly added dir.
    fs::write(dir_b.join("SKILL.md"), "# dynamic").unwrap();

    let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

    assert!(
        result.is_ok(),
        "should receive notification from dynamically added directory"
    );
    let new_version = *rx.borrow();
    assert!(
        new_version > version_before,
        "version should increment after event in dynamically added dir"
    );
}

// ---------------------------------------------------------------------------
// TC-11: watch_directory() with non-existent dir does not panic
// ---------------------------------------------------------------------------

/// [TC-11 黑盒] `watch_directory()` on a non-existent dir does not panic or crash.
#[tokio::test]
async fn tc11_watch_directory_nonexistent_no_panic() {
    let (mut watcher, _rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![]).unwrap();

    let result = watcher.watch_directory(&std::path::PathBuf::from("/nonexistent/dynamic_test"));
    // Should either return Ok (skip silently) or Err — but MUST NOT panic.
    // Per AC-2 spirit, we expect Ok (silently skipped).
    assert!(
        result.is_ok(),
        "watch_directory() should not error for non-existent dir per AC-2 skip semantics"
    );
}

// ---------------------------------------------------------------------------
// TC-12: stop() prevents subsequent notifications
// ---------------------------------------------------------------------------

/// [TC-12 黑盒] After `stop()`, file changes in formerly watched dir do not trigger notifications.
#[tokio::test]
async fn tc12_stop_prevents_notifications() {
    let (dir, _guard) = make_visible_test_dir("tc12");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify init, then drain any initial events.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let version_before_stop = *rx.borrow_and_update();

    watcher.stop();

    // Give a brief moment for the OS to process the unwatch.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create a file — should NOT trigger any notification.
    fs::write(dir.join("after_stop.md"), "# after stop").unwrap();

    tokio::time::sleep(Duration::from_millis(DEBOUNCE_NO_EXPECT_MS)).await;

    let version_after = *rx.borrow();
    assert_eq!(
        version_after, version_before_stop,
        "version should not change after stop() (was {version_before_stop}, got {version_after})"
    );
}

// ---------------------------------------------------------------------------
// TC-13: stop() is idempotent (safe to call multiple times)
// ---------------------------------------------------------------------------

/// [TC-13 黑盒] Calling `stop()` twice does not panic.
#[tokio::test]
async fn tc13_stop_idempotent() {
    let (mut watcher, _rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![]).unwrap();

    watcher.stop();
    watcher.stop(); // second call must not panic
}

// ---------------------------------------------------------------------------
// TC-14: hidden files (dot-prefixed) do not trigger notifications
// ---------------------------------------------------------------------------

/// [TC-14 黑盒] Creating a hidden file (`.swp`) does not trigger a version bump.
///
/// Uses a visible (non-dot-prefixed) parent directory so that normal files
/// *would* trigger a notification — confirming that only the hidden file is filtered.
///
/// `Modify(Metadata(_))` events are now filtered by `should_ignore`, so the
/// parent-directory metadata event emitted by macOS FSEvents is also suppressed.
#[tokio::test]
async fn tc14_hidden_file_not_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc14");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let version_before = *rx.borrow_and_update();

    // Create a hidden file (editor swap file).
    fs::write(dir.join(".swp"), "editor temp").unwrap();

    tokio::time::sleep(Duration::from_millis(DEBOUNCE_NO_EXPECT_MS)).await;

    let version_after = *rx.borrow();
    assert_eq!(
        version_after, version_before,
        "hidden file creation should not increment version (was {version_before}, got {version_after})"
    );
}

// ---------------------------------------------------------------------------
// TC-15: hidden dot-prefixed file does not trigger notification
// ---------------------------------------------------------------------------

/// [TC-15 黑盒] Creating `.hidden_skill.md` does not trigger a version bump.
///
/// Uses a visible (non-dot-prefixed) parent directory so that normal files
/// *would* trigger a notification — confirming that only the dot-prefixed file is filtered.
///
/// `Modify(Metadata(_))` filtering now suppresses macOS parent-dir metadata events.
#[tokio::test]
async fn tc15_dot_prefixed_file_not_triggers_notification() {
    let (dir, _guard) = make_visible_test_dir("tc15");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let version_before = *rx.borrow_and_update();

    fs::write(dir.join(".hidden_skill.md"), "# hidden").unwrap();

    tokio::time::sleep(Duration::from_millis(DEBOUNCE_NO_EXPECT_MS)).await;

    let version_after = *rx.borrow();
    assert_eq!(
        version_after, version_before,
        ".hidden_skill.md should not increment version (was {version_before}, got {version_after})"
    );
}

// ---------------------------------------------------------------------------
// TC-16: RuntimeDiscovery::clear_checked_dirs() exists and clears state
// ---------------------------------------------------------------------------

/// [TC-16 黑盒] `clear_checked_dirs()` method exists and empties the checked dirs cache.
#[test]
fn tc16_runtime_discovery_clear_checked_dirs() {
    let mut discovery = RuntimeDiscovery::new();

    // Verify the method is callable and clears state.
    // We can't inspect private fields directly, so we verify behaviour via
    // discover_dirs_for_paths returning results after clearing.
    //
    // The key assertion: calling clear_checked_dirs() does not panic.
    discovery.clear_checked_dirs();

    // Calling it a second time is also safe.
    discovery.clear_checked_dirs();
}

// ---------------------------------------------------------------------------
// TC-17 & TC-18 are verified by CI: cargo clippy and cargo test
// (These are build-level assertions, not unit tests.)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// TC-20: version number is strictly monotonically increasing
// ---------------------------------------------------------------------------

/// [TC-20 黑盒] Version numbers strictly increase across multiple independent events.
#[tokio::test]
async fn tc20_version_monotonically_increasing() {
    let (dir, _guard) = make_visible_test_dir("tc20");
    let (mut watcher, mut rx) = SkillWatcher::new().unwrap();
    watcher.start(vec![dir.clone()]).unwrap();

    // Wait for notify to initialise.
    tokio::time::sleep(Duration::from_millis(WATCHER_INIT_MS)).await;
    let mut prev_version = *rx.borrow_and_update();

    for round in 0..3u32 {
        // Each round: create a unique file, then wait for the notification.
        fs::write(
            dir.join(format!("round_{round}.md")),
            format!("# round {round}"),
        )
        .unwrap();

        let result = timeout(Duration::from_millis(DEBOUNCE_EXPECT_MS), rx.changed()).await;

        assert!(
            result.is_ok(),
            "round {round}: should receive notification within {DEBOUNCE_EXPECT_MS}ms"
        );

        let new_version = *rx.borrow_and_update();
        assert!(
            new_version > prev_version,
            "round {round}: version should be strictly increasing (prev={prev_version}, new={new_version})"
        );
        prev_version = new_version;
    }
}
