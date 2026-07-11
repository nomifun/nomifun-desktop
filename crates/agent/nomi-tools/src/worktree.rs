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
    source_root: PathBuf,
    path: PathBuf,
}

impl Worktree {
    /// Create a detached worktree of `repo` at HEAD. Errors if `repo` is not a
    /// git repo or `git worktree add` fails.
    pub fn create(repo: &Path) -> Result<Self, String> {
        let source_root = git_path(repo, "--show-toplevel")?
            .canonicalize()
            .map_err(|error| format!("could not canonicalize repository root: {error}"))?;
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir_name = format!(".nomi-worktree-{}-{}", std::process::id(), seq);
        // Place the worktree as a sibling of the repo so it is not itself scanned
        // as part of the repo tree.
        let source_root_for_git = git_compatible_path(&source_root);
        let parent = source_root_for_git
            .parent()
            .unwrap_or(&source_root_for_git);
        let path = parent.join(&dir_name);

        let out = Command::new("git")
            .arg("-C")
            .arg(&source_root)
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
        Ok(Self {
            repo: repo.to_path_buf(),
            source_root,
            path,
        })
    }

    /// The worktree's path (use as the sub-agent's cwd).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Return the exact canonical path created by this live worktree handle,
    /// after verifying that Git still reports it as a worktree of `repo`.
    ///
    /// This is the authority hand-off used by child-agent capability setup. A
    /// caller cannot substitute an arbitrary sibling directory because the
    /// expected path must match both this handle and `git worktree list`.
    pub fn verified_path(&self) -> Result<PathBuf, String> {
        let repository = self
            .repo
            .canonicalize()
            .map_err(|error| format!("could not canonicalize worktree repository: {error}"))?;
        let expected = self
            .path
            .canonicalize()
            .map_err(|error| format!("could not canonicalize created worktree: {error}"))?;
        let common_dir = git_path(&repository, "--git-common-dir")?;
        let expected_common_dir = resolve_git_path(&repository, &common_dir)
            .canonicalize()
            .map_err(|error| format!("could not canonicalize repository common dir: {error}"))?;

        let output = Command::new("git")
            .arg("-C")
            .arg(&repository)
            .args(["worktree", "list", "--porcelain"])
            .output()
            .map_err(|error| format!("git worktree list failed to spawn: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "git worktree list failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }

        let listed = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.strip_prefix("worktree "))
            .map(PathBuf::from)
            .filter_map(|path| path.canonicalize().ok())
            .any(|path| path == expected);
        if !listed {
            return Err(format!(
                "created worktree is no longer registered by Git: {}",
                expected.display()
            ));
        }

        let worktree_common_dir = git_path(&expected, "--git-common-dir")?;
        let worktree_common_dir = resolve_git_path(&expected, &worktree_common_dir)
            .canonicalize()
            .map_err(|error| format!("could not canonicalize worktree common dir: {error}"))?;
        if worktree_common_dir != expected_common_dir {
            return Err(format!(
                "created worktree belongs to a different repository: {}",
                expected.display()
            ));
        }
        Ok(expected)
    }

    /// Map the source session cwd into this exact worktree without broadening a
    /// repository-subdirectory session to the whole repository root.
    pub fn verified_cwd(&self, source_cwd: &Path) -> Result<PathBuf, String> {
        let source_cwd = source_cwd
            .canonicalize()
            .map_err(|error| format!("could not canonicalize source cwd: {error}"))?;
        let relative = source_cwd.strip_prefix(&self.source_root).map_err(|_| {
            format!(
                "source cwd {} is outside repository root {}",
                source_cwd.display(),
                self.source_root.display()
            )
        })?;
        let worktree_root = self.verified_path()?;
        let mapped = worktree_root.join(relative);
        let mapped = mapped.canonicalize().map_err(|error| {
            format!(
                "could not canonicalize mapped worktree cwd {}: {error}",
                mapped.display()
            )
        })?;
        if !mapped.is_dir() || !mapped.starts_with(&worktree_root) {
            return Err(format!(
                "mapped worktree cwd is not a directory inside the verified worktree: {}",
                mapped.display()
            ));
        }
        Ok(mapped)
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

fn git_path(cwd: &Path, argument: &str) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--path-format=absolute", argument])
        .output()
        .map_err(|error| format!("git rev-parse {argument} failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git rev-parse {argument} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("git rev-parse {argument} returned non-UTF-8 output: {error}"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("git rev-parse {argument} returned an empty path"));
    }
    Ok(PathBuf::from(value))
}

fn resolve_git_path(cwd: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

#[cfg(windows)]
fn git_compatible_path(path: &Path) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.to_path_buf();
    };
    if !matches!(prefix.kind(), Prefix::VerbatimDisk(_)) {
        return path.to_path_buf();
    }
    let mut compatible = PathBuf::new();
    if let Prefix::VerbatimDisk(drive) = prefix.kind() {
        compatible.push(format!("{}:", char::from(drive)));
    }
    for component in components {
        compatible.push(component.as_os_str());
    }
    compatible
}

#[cfg(not(windows))]
fn git_compatible_path(path: &Path) -> PathBuf {
    path.to_path_buf()
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
        assert_eq!(
            wt.verified_path().expect("verify created worktree"),
            wt.path().canonicalize().unwrap()
        );

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

    #[test]
    fn verified_path_rejects_a_handle_whose_expected_path_was_replaced() {
        let repo = init_repo();
        let wt = Worktree::create(repo.path()).expect("create");
        let registered_path = wt.path().to_path_buf();

        let remove = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["worktree", "remove", "--force"])
            .arg(&registered_path)
            .output()
            .expect("remove registered worktree");
        assert!(
            remove.status.success(),
            "git worktree remove failed: {}",
            String::from_utf8_lossy(&remove.stderr)
        );
        std::fs::create_dir_all(&registered_path).expect("replace with plain sibling directory");

        let error = wt
            .verified_path()
            .expect_err("plain sibling directory must not inherit worktree authority");
        assert!(error.contains("no longer registered"), "{error}");
    }

    #[test]
    fn verified_cwd_preserves_a_repository_subdirectory_scope() {
        let repo = init_repo();
        let nested = repo.path().join("packages").join("a");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("nested.txt"), "nested\n").unwrap();
        git(&["add", "-A"], repo.path());
        git(&["commit", "-q", "-m", "nested"], repo.path());
        let wt = Worktree::create(&nested).expect("create from nested cwd");

        let mapped = wt.verified_cwd(&nested).expect("map nested cwd");

        assert_eq!(
            mapped,
            wt.verified_path()
                .unwrap()
                .join("packages")
                .join("a")
                .canonicalize()
                .unwrap()
        );
        assert_ne!(mapped, wt.verified_path().unwrap());
    }
}
