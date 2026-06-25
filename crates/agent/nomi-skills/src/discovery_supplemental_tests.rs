// Supplemental tests for Phase 8 — RuntimeDiscovery.
// Covers test-plan.md TC-21 through TC-37.
//
// These tests use `tempfile::TempDir` to create real filesystem structures.
// Async tests use `#[tokio::test]`.

#[cfg(test)]
mod discovery_supplemental_tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use crate::discovery::RuntimeDiscovery;

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    /// Create a `.nomi/skills/` directory inside `parent`.
    fn create_skill_dir(parent: &Path) -> PathBuf {
        let dir = parent.join(".nomi").join("skills");
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write a minimal valid skill in directory format (`<name>/SKILL.md`).
    ///
    /// `load_skills_from_dir` only supports the directory format (each
    /// subdirectory containing a `SKILL.md` file), so flat `.md` files
    /// are not loaded by that function.
    fn write_skill_file(dir: &Path, name: &str) {
        let skill_subdir = dir.join(name);
        fs::create_dir_all(&skill_subdir).unwrap();
        let content = format!(
            "---\ndescription: test skill {}\n---\n\nSkill content for {}.",
            name, name
        );
        fs::write(skill_subdir.join("SKILL.md"), content).unwrap();
    }

    // ---------------------------------------------------------------------------
    // TC-21: new_creates_empty_discovery
    // ---------------------------------------------------------------------------

    // TC-21: new() yields an empty manager.
    #[test]
    fn tc21_new_creates_empty_discovery() {
        let mgr = RuntimeDiscovery::new();
        assert!(mgr.get_dynamic_skills().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-22: discover_dirs_finds_skill_dir_in_subdir
    // ---------------------------------------------------------------------------

    // TC-22: discovers `.nomi/skills/` inside a direct subdirectory of cwd.
    #[tokio::test]
    async fn tc22_discover_dirs_finds_nomi_skills_in_subdir() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        // Create /tmp/proj/module/.nomi/skills/
        let module = tmp.path().join("module");
        fs::create_dir_all(&module).unwrap();
        create_skill_dir(&module);

        let mut mgr = RuntimeDiscovery::new();
        let file_path = module.join("foo.rs");
        fs::write(&file_path, "").unwrap();

        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        assert_eq!(found.len(), 1);
        assert!(found[0].ends_with(".nomi/skills"));
    }

    // ---------------------------------------------------------------------------
    // TC-23: cwd-level skill dir not re-discovered
    // ---------------------------------------------------------------------------

    // TC-23: `.nomi/skills/` at cwd level is not returned (loaded at startup).
    #[tokio::test]
    async fn tc23_discover_dirs_does_not_return_cwd_level() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        // Create skill dir directly in cwd
        create_skill_dir(tmp.path());

        let mut mgr = RuntimeDiscovery::new();
        let file_path = tmp.path().join("foo.rs");
        fs::write(&file_path, "").unwrap();

        // file parent IS cwd — no subdirectory to walk
        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        assert!(found.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-24: already-checked dirs are not re-stat'd
    // ---------------------------------------------------------------------------

    // TC-24: second call for the same file path returns empty (already checked).
    #[tokio::test]
    async fn tc24_discover_dirs_dedup_checked_dirs() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        let module = tmp.path().join("a");
        fs::create_dir_all(&module).unwrap();
        create_skill_dir(&module);

        let file_path = module.join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();

        // First call discovers the dir
        let first = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert_eq!(first.len(), 1);

        // Second call: already in checked_dirs → returns empty
        let second = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert!(second.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-25: miss dirs are also recorded in checked_dirs
    // ---------------------------------------------------------------------------

    // TC-25: directories without `.nomi/skills/` are still recorded to avoid
    // repeated stat calls.
    #[tokio::test]
    async fn tc25_discover_dirs_records_miss_dirs_in_checked() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        // `b` does NOT have .nomi/skills/
        let subdir = tmp.path().join("b");
        fs::create_dir_all(&subdir).unwrap();
        let file_path = subdir.join("bar.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();

        let first = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert!(first.is_empty());

        // Second call: the miss is cached — still empty, no crash
        let second = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert!(second.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-26 & TC-27: gitignore integration
    // ---------------------------------------------------------------------------

    // TC-26: gitignored directory is skipped.
    // Uses a real git repo with `.gitignore` to trigger `git check-ignore`.
    #[tokio::test]
    async fn tc26_discover_dirs_skips_gitignored_dir() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        // Init git repo
        let status = std::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .status();

        // If git is not available skip gracefully
        if status.is_err() || !status.unwrap().success() {
            return;
        }

        // Create subdirectory and write to .gitignore
        let ignored = tmp.path().join("ignored");
        fs::create_dir_all(&ignored).unwrap();
        create_skill_dir(&ignored);

        // Add `ignored/` to .gitignore
        fs::write(tmp.path().join(".gitignore"), "ignored/\n").unwrap();

        let file_path = ignored.join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        assert!(found.is_empty(), "gitignored dir should be skipped");
    }

    // TC-27: when git fails (non-git dir), path is not filtered (fail-open).
    #[tokio::test]
    async fn tc27_discover_dirs_not_filtered_when_git_unavailable() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        // NOT a git repo — git check-ignore will fail
        let normal = tmp.path().join("normal");
        fs::create_dir_all(&normal).unwrap();
        create_skill_dir(&normal);

        let file_path = normal.join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        // fail-open: non-git dir → dir is NOT filtered
        assert_eq!(found.len(), 1);
    }

    // ---------------------------------------------------------------------------
    // TC-28: deepest-first sort
    // ---------------------------------------------------------------------------

    // TC-28: returned directories are sorted deepest-first.
    #[tokio::test]
    async fn tc28_discover_dirs_sorted_deepest_first() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        let a = tmp.path().join("a");
        let ab = a.join("b");
        fs::create_dir_all(&ab).unwrap();
        create_skill_dir(&a);
        create_skill_dir(&ab);

        let file_path = ab.join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        assert_eq!(found.len(), 2);
        // Deepest path has more components
        let depth = |p: &PathBuf| p.components().count();
        assert!(depth(&found[0]) >= depth(&found[1]));
    }

    // ---------------------------------------------------------------------------
    // TC-29: multiple file paths discover multiple directories
    // ---------------------------------------------------------------------------

    // TC-29: two separate file paths each with a skill dir are both discovered.
    #[tokio::test]
    async fn tc29_discover_dirs_multiple_file_paths() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        let x = tmp.path().join("x");
        let y = tmp.path().join("y");
        fs::create_dir_all(&x).unwrap();
        fs::create_dir_all(&y).unwrap();
        create_skill_dir(&x);
        create_skill_dir(&y);

        let fx = x.join("a.rs");
        let fy = y.join("b.rs");
        fs::write(&fx, "").unwrap();
        fs::write(&fy, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr
            .discover_dirs_for_paths(&[fx.to_str().unwrap(), fy.to_str().unwrap()], &cwd)
            .await;

        assert_eq!(found.len(), 2);
    }

    // ---------------------------------------------------------------------------
    // TC-30: empty file_paths returns empty
    // ---------------------------------------------------------------------------

    // TC-30: passing no file paths returns empty list without panic.
    #[tokio::test]
    async fn tc30_discover_dirs_empty_file_paths() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr.discover_dirs_for_paths(&[], &cwd).await;
        assert!(found.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-31: add_skill_directories loads skills
    // ---------------------------------------------------------------------------

    // TC-31: skills are loaded from a discovered directory.
    #[tokio::test]
    async fn tc31_add_skill_directories_loads_skills() {
        let tmp = TempDir::new().unwrap();
        let module = tmp.path().join("module");
        fs::create_dir_all(&module).unwrap();
        let skill_dir = create_skill_dir(&module);
        write_skill_file(&skill_dir, "my-skill");

        let mut mgr = RuntimeDiscovery::new();
        let count = mgr.add_skill_directories(&[skill_dir]).await;

        assert!(count > 0);
        assert!(!mgr.get_dynamic_skills().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-32: deeper directory wins on same-name skill
    // ---------------------------------------------------------------------------

    // TC-32: when the same skill name appears in both shallow and deep dirs,
    // the deeper directory's version takes precedence.
    #[tokio::test]
    async fn tc32_add_skill_directories_deeper_wins() {
        let tmp = TempDir::new().unwrap();

        let shallow = tmp.path().join("a");
        let deep = tmp.path().join("a").join("b");
        fs::create_dir_all(&shallow).unwrap();
        fs::create_dir_all(&deep).unwrap();

        let shallow_skills = create_skill_dir(&shallow);
        let deep_skills = create_skill_dir(&deep);

        // Both dirs have a skill named "shared" with different content.
        // Use directory format (<name>/SKILL.md) — required by load_skills_from_dir.
        let shallow_content = "---\ndescription: shallow version\n---\n\nShallow skill.";
        let deep_content = "---\ndescription: deep version\n---\n\nDeep skill.";
        let shallow_skill_dir = shallow_skills.join("shared");
        let deep_skill_dir = deep_skills.join("shared");
        fs::create_dir_all(&shallow_skill_dir).unwrap();
        fs::create_dir_all(&deep_skill_dir).unwrap();
        fs::write(shallow_skill_dir.join("SKILL.md"), shallow_content).unwrap();
        fs::write(deep_skill_dir.join("SKILL.md"), deep_content).unwrap();

        let mut mgr = RuntimeDiscovery::new();
        // Pass deepest first (as discover_dirs_for_paths would return)
        mgr.add_skill_directories(&[deep_skills, shallow_skills])
            .await;

        let skills = mgr.get_dynamic_skills();
        assert_eq!(skills.len(), 1);
        // Deeper version should have "deep version" description
        assert_eq!(skills[0].description, "deep version");
    }

    // ---------------------------------------------------------------------------
    // TC-33: add_skill_directories with empty dir
    // ---------------------------------------------------------------------------

    // TC-33: empty skill directory returns 0 and get_dynamic_skills stays empty.
    #[tokio::test]
    async fn tc33_add_skill_directories_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let empty_skills = create_skill_dir(tmp.path());

        let mut mgr = RuntimeDiscovery::new();
        let count = mgr.add_skill_directories(&[empty_skills]).await;

        assert_eq!(count, 0);
        assert!(mgr.get_dynamic_skills().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-34: get_dynamic_skills returns all loaded skills
    // ---------------------------------------------------------------------------

    // TC-34: skills from two separate directories are all returned.
    #[tokio::test]
    async fn tc34_get_dynamic_skills_returns_all_loaded() {
        let tmp = TempDir::new().unwrap();

        let dir_a = tmp.path().join("dir_a");
        let dir_b = tmp.path().join("dir_b");
        fs::create_dir_all(&dir_a).unwrap();
        fs::create_dir_all(&dir_b).unwrap();

        let skills_a = create_skill_dir(&dir_a);
        let skills_b = create_skill_dir(&dir_b);

        write_skill_file(&skills_a, "skill-a");
        write_skill_file(&skills_b, "skill-b");

        let mut mgr = RuntimeDiscovery::new();
        mgr.add_skill_directories(&[skills_a]).await;
        mgr.add_skill_directories(&[skills_b]).await;

        let all = mgr.get_dynamic_skills();
        assert_eq!(all.len(), 2);
        let names: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"skill-a"));
        assert!(names.contains(&"skill-b"));
    }

    // ---------------------------------------------------------------------------
    // TC-35: clear_dynamic_skills removes all skills
    // ---------------------------------------------------------------------------

    // TC-35: after clear_dynamic_skills, get_dynamic_skills returns empty.
    #[tokio::test]
    async fn tc35_clear_dynamic_skills_removes_skills() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("m");
        fs::create_dir_all(&dir).unwrap();
        let skill_dir = create_skill_dir(&dir);
        write_skill_file(&skill_dir, "some-skill");

        let mut mgr = RuntimeDiscovery::new();
        mgr.add_skill_directories(&[skill_dir]).await;
        assert!(!mgr.get_dynamic_skills().is_empty());

        mgr.clear_dynamic_skills();
        assert!(mgr.get_dynamic_skills().is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-36: clear_dynamic_skills preserves checked_dirs
    // ---------------------------------------------------------------------------

    // TC-36: checked_dirs survive clear_dynamic_skills — second discover call
    // for the same path returns empty (still cached as checked).
    #[tokio::test]
    async fn tc36_clear_dynamic_skills_preserves_checked_dirs() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_str().unwrap().to_string();

        let module = tmp.path().join("mod");
        fs::create_dir_all(&module).unwrap();
        create_skill_dir(&module);

        let file_path = module.join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        // First discover — populates checked_dirs
        let first = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert_eq!(first.len(), 1);

        mgr.clear_dynamic_skills();
        assert!(mgr.get_dynamic_skills().is_empty());

        // Second discover — checked_dirs still has the entry → returns empty
        let second = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;
        assert!(second.is_empty());
    }

    // ---------------------------------------------------------------------------
    // TC-37: file outside cwd is not traversed into cwd
    // ---------------------------------------------------------------------------

    // TC-37: a file path outside cwd does not cause traversal into cwd or beyond.
    #[tokio::test]
    async fn tc37_discover_dirs_file_outside_cwd_ignored() {
        let tmp = TempDir::new().unwrap();

        // Two separate dirs: cwd and an "other" root
        let cwd_dir = tmp.path().join("proj");
        let other_dir = tmp.path().join("other");
        fs::create_dir_all(&cwd_dir).unwrap();
        fs::create_dir_all(other_dir.join("module")).unwrap();
        create_skill_dir(&other_dir.join("module"));

        let cwd = cwd_dir.to_str().unwrap().to_string();
        let file_path = other_dir.join("module").join("file.rs");
        fs::write(&file_path, "").unwrap();

        let mut mgr = RuntimeDiscovery::new();
        let found = mgr
            .discover_dirs_for_paths(&[file_path.to_str().unwrap()], &cwd)
            .await;

        // Outside cwd — no traversal should produce results within cwd
        assert!(found.is_empty());
    }
}
