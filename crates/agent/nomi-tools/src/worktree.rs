//! Git worktree isolation for parallel editing sub-agents (design §3.4
//! "worktree 隔离": 并行编辑子 agent 用临时 worktree，校验后回并).
//!
//! When several `implementer` sub-agents edit files concurrently they can
//! clobber one another in the shared tree. Running each in its own detached git
//! worktree isolates their edits; the parent collects each one's diff (returned
//! as a unified patch) and decides what to apply — no auto-merge, so there is no
//! merge-conflict resolution to get wrong.
//!
//! Opt-in and additive: only used when a Spawn fan-out requests isolation AND
//! the workspace is a git repo. The worktree is removed on drop.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Monotonic suffix so concurrent worktrees get distinct dir names without
/// needing a clock or RNG (both unavailable / nondeterministic).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// True if `root` is inside a git working tree.
pub fn is_git_repo(root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

/// A detached git worktree of `repo`, removed on drop.
pub struct Worktree {
    repo: PathBuf,
    path: PathBuf,
}

impl Worktree {
    /// Create a detached worktree of `repo` at HEAD. Errors if `repo` is not a
    /// git repo or `git worktree add` fails.
    pub fn create(repo: &Path) -> Result<Self, String> {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir_name = format!(".nomi-worktree-{}-{}", std::process::id(), seq);
        // Place the worktree as a sibling of the repo so it is not itself scanned
        // as part of the repo tree.
        let parent = repo.parent().unwrap_or(repo);
        let path = parent.join(&dir_name);

        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["worktree", "add", "--detach"])
            .arg(&path)
            .output()
            .map_err(|e| format!("git worktree add failed to spawn: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(Self { repo: repo.to_path_buf(), path })
    }

    /// The worktree's path (use as the sub-agent's cwd).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Capture all changes made in the worktree (new + modified files) as a
    /// unified diff. Stages everything first so untracked files are included.
    pub fn capture_diff(&self) -> Result<String, String> {
        let add = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["add", "-A"])
            .output()
            .map_err(|e| format!("git add failed to spawn: {e}"))?;
        if !add.status.success() {
            return Err(format!("git add failed: {}", String::from_utf8_lossy(&add.stderr).trim()));
        }
        let diff = Command::new("git")
            .arg("-C")
            .arg(&self.path)
            .args(["diff", "--cached"])
            .output()
            .map_err(|e| format!("git diff failed to spawn: {e}"))?;
        if !diff.status.success() {
            return Err(format!("git diff failed: {}", String::from_utf8_lossy(&diff.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&diff.stdout).into_owned())
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        // Best-effort removal; --force discards the worktree's uncommitted edits
        // (the parent already captured the diff it cares about).
        let _ = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .output();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(args: &[&str], cwd: &Path) {
        let out = Command::new("git").arg("-C").arg(cwd).args(args).output().unwrap();
        assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
    }

    /// Init a repo with one committed file. Returns the repo dir (kept alive by
    /// the returned TempDir).
    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(&["init", "-q"], p);
        git(&["config", "user.email", "t@t"], p);
        git(&["config", "user.name", "t"], p);
        std::fs::write(p.join("a.txt"), "original\n").unwrap();
        git(&["add", "-A"], p);
        git(&["commit", "-q", "-m", "init"], p);
        dir
    }

    #[test]
    fn is_git_repo_detects_repo_and_non_repo() {
        let repo = init_repo();
        assert!(is_git_repo(repo.path()));
        let plain = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(plain.path()));
    }

    #[test]
    fn worktree_isolates_edits_and_captures_diff() {
        let repo = init_repo();
        let wt = Worktree::create(repo.path()).expect("create worktree");
        assert!(wt.path().exists(), "worktree dir should exist");

        // Edit an existing file and add a new one inside the worktree.
        std::fs::write(wt.path().join("a.txt"), "changed\n").unwrap();
        std::fs::write(wt.path().join("new.txt"), "brand new\n").unwrap();

        // The main tree is untouched (isolation).
        assert_eq!(std::fs::read_to_string(repo.path().join("a.txt")).unwrap(), "original\n");
        assert!(!repo.path().join("new.txt").exists());

        let diff = wt.capture_diff().expect("diff");
        assert!(diff.contains("a.txt"), "diff mentions the edited file:\n{diff}");
        assert!(diff.contains("changed"), "diff shows the change:\n{diff}");
        assert!(diff.contains("new.txt"), "diff includes the new file:\n{diff}");
    }

    #[test]
    fn worktree_is_removed_on_drop() {
        let repo = init_repo();
        let path = {
            let wt = Worktree::create(repo.path()).expect("create");
            wt.path().to_path_buf()
        }; // dropped here
        assert!(!path.exists(), "worktree dir must be removed on drop");
        // And git no longer lists it.
        let list = Command::new("git").arg("-C").arg(repo.path()).args(["worktree", "list"]).output().unwrap();
        let listing = String::from_utf8_lossy(&list.stdout);
        assert!(!listing.contains(path.to_string_lossy().as_ref()), "git must not still list the worktree");
    }
}
