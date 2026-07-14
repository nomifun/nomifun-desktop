//! Git worktree isolation for parallel mutation-capable Agents.
//!
//! When several delegated `implementer` Agents edit files concurrently they can
//! clobber one another in the shared tree. Running each in its own detached git
//! worktree isolates their edits; the parent collects each one's diff (returned
//! as a unified patch) and decides what to apply — no auto-merge, so there is no
//! merge-conflict resolution to get wrong.
//!
//! A fan-out captures one immutable source snapshot first, including staged,
//! unstaged, and untracked (non-ignored) files without modifying the user's real
//! index. Every isolated sibling starts from that same snapshot, so patches
//! contain only the sibling's own delta. The worktree is removed on drop.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
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

/// One immutable view of the source workspace used by every isolated writer in
/// a fan-out. Capturing it writes only temporary Git objects and a private
/// temporary index; it never stages, resets, or otherwise mutates the source
/// workspace or its real index.
#[derive(Clone)]
pub struct WorktreeBaseline {
    inner: Arc<WorktreeBaselineInner>,
}

struct WorktreeBaselineInner {
    _storage: tempfile::TempDir,
    git_dir: PathBuf,
    worktrees_dir: PathBuf,
    source_root: PathBuf,
    revision: String,
    worktree_admin: Mutex<()>,
}

impl WorktreeBaseline {
    pub fn capture(repo: &Path) -> Result<Self, String> {
        let repo = repo
            .canonicalize()
            .map_err(|error| format!("could not canonicalize repository path: {error}"))?;
        if !is_git_repo(&repo) {
            return Err(format!("{} is not a Git working tree", repo.display()));
        }
        let source_root = git_path(&repo, "--show-toplevel")?
            .canonicalize()
            .map_err(|error| format!("could not canonicalize repository root: {error}"))?;
        let storage = tempfile::Builder::new()
            .prefix("nomi-agent-worktrees-")
            .tempdir()
            .map_err(|error| format!("could not create private worktree storage: {error}"))?;
        let git_dir = storage.path().join("repo.git");
        let worktrees_dir = storage.path().join("worktrees");
        initialize_private_repository(&git_dir, &source_root)?;
        let revision = capture_source_snapshot(&git_dir, &source_root)?;
        std::fs::create_dir(&worktrees_dir)
            .map_err(|error| format!("could not create private worktree directory: {error}"))?;
        Ok(Self {
            inner: Arc::new(WorktreeBaselineInner {
                _storage: storage,
                git_dir,
                worktrees_dir,
                source_root,
                revision,
                worktree_admin: Mutex::new(()),
            }),
        })
    }

    /// Create one detached worktree from this exact source snapshot.
    pub fn create(&self) -> Result<Worktree, String> {
        let _admin = self
            .inner
            .worktree_admin
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir_name = format!(".nomi-worktree-{}-{}", std::process::id(), seq);
        let path = self.inner.worktrees_dir.join(dir_name);

        let out = Command::new("git")
            .arg("--git-dir")
            .arg(&self.inner.git_dir)
            .args(["worktree", "add", "--detach"])
            .arg(&path)
            .arg(&self.inner.revision)
            .output()
            .map_err(|error| format!("git worktree add failed to spawn: {error}"))?;
        if !out.status.success() {
            return Err(format!(
                "git worktree add failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(Worktree {
            baseline: self.clone(),
            path,
        })
    }
}

/// A detached git worktree of `repo`, removed on drop.
pub struct Worktree {
    baseline: WorktreeBaseline,
    path: PathBuf,
}

impl Worktree {
    /// Convenience constructor for a one-worktree fan-out. Multi-writer callers
    /// should capture one [`WorktreeBaseline`] and call its `create` method for
    /// every sibling so all siblings share the exact same source view.
    pub fn create(repo: &Path) -> Result<Self, String> {
        WorktreeBaseline::capture(repo)?.create()
    }

    /// The worktree's path (use as the delegated Agent's cwd).
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
        let _admin = self
            .baseline
            .inner
            .worktree_admin
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let repository = self
            .baseline
            .inner
            .git_dir
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
        let relative = source_cwd
            .strip_prefix(&self.baseline.inner.source_root)
            .map_err(|_| {
            format!(
                "source cwd {} is outside repository root {}",
                source_cwd.display(),
                self.baseline.inner.source_root.display()
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
            .args([
                "diff",
                "--cached",
                "--binary",
                "--full-index",
                "--no-ext-diff",
                "--no-textconv",
            ])
            .arg(&self.baseline.inner.revision)
            .arg("--")
            .output()
            .map_err(|e| format!("git diff failed to spawn: {e}"))?;
        if !diff.status.success() {
            return Err(format!("git diff failed: {}", String::from_utf8_lossy(&diff.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&diff.stdout).into_owned())
    }
}

fn initialize_private_repository(git_dir: &Path, source_root: &Path) -> Result<(), String> {
    let object_format = git_text(
        source_root,
        &["rev-parse", "--show-object-format"],
        "resolve Git object format",
    )?;
    let init = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(format!("--object-format={object_format}"))
        .arg(git_dir)
        .output()
        .map_err(|error| format!("git init --bare failed to spawn: {error}"))?;
    if !init.status.success() {
        return Err(format!(
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr).trim()
        ));
    }

    let common_dir = git_path(source_root, "--git-common-dir")?;
    let source_objects = resolve_git_path(source_root, &common_dir)
        .join("objects")
        .canonicalize()
        .map_err(|error| format!("could not resolve source Git object store: {error}"))?;
    let source_objects = git_compatible_path(&source_objects);
    let alternates = git_dir.join("objects").join("info").join("alternates");
    std::fs::write(&alternates, format!("{}\n", source_objects.display())).map_err(|error| {
        format!(
            "could not configure private Git object alternates {}: {error}",
            alternates.display()
        )
    })?;
    project_snapshot_git_semantics(git_dir, source_root)?;
    Ok(())
}

fn project_snapshot_git_semantics(git_dir: &Path, source_root: &Path) -> Result<(), String> {
    // Project only Git content semantics. Remote/auth, hooks, aliases, signing,
    // fsmonitor, and other host behavior intentionally stay outside the
    // private repository.
    let pattern = concat!(
        "^(core\\.(autocrlf|safecrlf|filemode|symlinks|ignorecase|",
        "precomposeunicode|protecthfs|protectntfs|sparsecheckout|",
        "sparsecheckoutcone)|filter\\.)"
    );
    let output = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["config", "--null", "--get-regexp", pattern])
        .output()
        .map_err(|error| format!("git read snapshot config failed to spawn: {error}"))?;
    if !output.status.success() && output.status.code() != Some(1) {
        return Err(format!(
            "git read snapshot config failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    for record in output.stdout.split(|byte| *byte == 0).filter(|record| !record.is_empty()) {
        let Some(separator) = record.iter().position(|byte| *byte == b'\n') else {
            return Err("git snapshot config returned a malformed record".to_owned());
        };
        let key = std::str::from_utf8(&record[..separator])
            .map_err(|error| format!("git snapshot config key was non-UTF-8: {error}"))?;
        let value = std::str::from_utf8(&record[separator + 1..])
            .map_err(|error| format!("git snapshot config value was non-UTF-8: {error}"))?;
        set_private_config(git_dir, key, value)?;
    }

    copy_git_metadata_file(source_root, git_dir, "info/exclude", "info/exclude")?;
    copy_git_metadata_file(source_root, git_dir, "info/attributes", "info/attributes")?;
    copy_git_metadata_file(
        source_root,
        git_dir,
        "info/sparse-checkout",
        "info/sparse-checkout",
    )?;
    freeze_config_path(git_dir, source_root, "core.excludesFile", "source-excludes")?;
    freeze_config_path(
        git_dir,
        source_root,
        "core.attributesFile",
        "source-attributes",
    )?;
    let private_lfs = git_dir.join("lfs");
    set_private_config(git_dir, "lfs.storage", &private_lfs.to_string_lossy())?;
    Ok(())
}

fn set_private_config(git_dir: &Path, key: &str, value: &str) -> Result<(), String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["config", "--local", "--replace-all", key, value])
        .output()
        .map_err(|error| format!("git project config '{key}' failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git project config '{key}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn copy_git_metadata_file(
    source_root: &Path,
    git_dir: &Path,
    source_name: &str,
    destination_name: &str,
) -> Result<(), String> {
    let source = source_git_path(source_root, source_name)?;
    if !source.is_file() {
        return Ok(());
    }
    let destination = git_dir.join(destination_name);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create private Git metadata directory: {error}"))?;
    }
    std::fs::copy(&source, &destination).map_err(|error| {
        format!(
            "could not copy Git metadata {} into {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn freeze_config_path(
    git_dir: &Path,
    source_root: &Path,
    key: &str,
    destination_name: &str,
) -> Result<(), String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["config", "--path", "--get", key])
        .output()
        .map_err(|error| format!("git resolve config path '{key}' failed to spawn: {error}"))?;
    if !output.status.success() {
        if output.status.code() == Some(1) {
            return Ok(());
        }
        return Err(format!(
            "git resolve config path '{key}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("git config path '{key}' was non-UTF-8: {error}"))?;
    let source = PathBuf::from(value.trim());
    if !source.is_file() {
        return Ok(());
    }
    let destination = git_dir.join(destination_name);
    std::fs::copy(&source, &destination).map_err(|error| {
        format!(
            "could not freeze Git config path {} into {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    set_private_config(git_dir, key, &destination.to_string_lossy())
}

fn capture_source_snapshot(git_dir: &Path, source_root: &Path) -> Result<String, String> {
    let private_index = git_dir.join("index");
    let tree = capture_stable_source_tree(git_dir, source_root, &private_index)?;

    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["commit-tree", tree.trim()])
        .args(["-m", "Nomi isolated Agent workspace baseline"])
        .env("GIT_AUTHOR_NAME", "Nomi")
        .env("GIT_AUTHOR_EMAIL", "agent@localhost")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "Nomi")
        .env("GIT_COMMITTER_EMAIL", "agent@localhost")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .output()
        .map_err(|error| format!("git commit-tree failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git commit-tree failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let revision = String::from_utf8(output.stdout)
        .map_err(|error| format!("git commit-tree returned non-UTF-8 output: {error}"))?;
    let revision = revision.trim();
    if revision.is_empty() {
        return Err("git commit-tree returned an empty revision".to_owned());
    }
    materialize_private_snapshot(git_dir, revision)?;
    Ok(revision.to_owned())
}

fn capture_stable_source_tree(
    git_dir: &Path,
    source_root: &Path,
    private_index: &Path,
) -> Result<String, String> {
    select_stable_tree(|| capture_source_tree_once(git_dir, source_root, private_index))
}

fn select_stable_tree<F>(mut capture: F) -> Result<String, String>
where
    F: FnMut() -> Result<String, String>,
{
    const MAX_CAPTURES: usize = 3;
    let mut previous = capture()?;
    for _ in 1..MAX_CAPTURES {
        let next = capture()?;
        if next == previous {
            return Ok(next);
        }
        previous = next;
    }
    Err(format!(
        "workspace changed while the Agent isolation baseline was being captured (no stable tree after {MAX_CAPTURES} attempts)"
    ))
}

fn capture_source_tree_once(
    git_dir: &Path,
    source_root: &Path,
    private_index: &Path,
) -> Result<String, String> {
    reset_private_index(git_dir, source_root, private_index)?;
    // Renormalizing every present tracked file makes the private object store
    // self-sufficient for configured clean/smudge filters such as Git LFS.
    git_in_private_repository(
        git_dir,
        source_root,
        private_index,
        &["add", "--renormalize", "-u", "--", "."],
        "snapshot tracked source files",
    )?;
    let untracked = source_untracked_paths(source_root)?;
    if !untracked.is_empty() {
        git_in_private_repository_with_input(
            git_dir,
            source_root,
            private_index,
            &[
                "add",
                "-f",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            &untracked,
            "snapshot exact non-ignored untracked files",
        )?;
    }
    git_in_private_repository(
        git_dir,
        source_root,
        private_index,
        &["write-tree"],
        "write source snapshot tree",
    )
}

fn reset_private_index(
    git_dir: &Path,
    source_root: &Path,
    private_index: &Path,
) -> Result<(), String> {
    let source_index = source_index_path(source_root)?;
    if source_index.is_file() {
        copy_index_state(&source_index, private_index)?;
    } else {
        git_in_private_repository(
            git_dir,
            source_root,
            private_index,
            &["read-tree", "--empty"],
            "initialize empty source snapshot index",
        )?;
    }
    Ok(())
}

fn source_untracked_paths(source_root: &Path) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(source_root)
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .output()
        .map_err(|error| format!("git list untracked files failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git list untracked files failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

fn materialize_private_snapshot(git_dir: &Path, revision: &str) -> Result<(), String> {
    let update_ref = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["update-ref", "refs/nomi/baseline", revision])
        .output()
        .map_err(|error| format!("git update-ref for private baseline failed to spawn: {error}"))?;
    if !update_ref.status.success() {
        return Err(format!(
            "git update-ref for private baseline failed: {}",
            String::from_utf8_lossy(&update_ref.stderr).trim()
        ));
    }
    let repack = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["repack", "-a", "-d", "--quiet"])
        .output()
        .map_err(|error| format!("git repack private baseline failed to spawn: {error}"))?;
    if !repack.status.success() {
        return Err(format!(
            "git repack private baseline failed: {}",
            String::from_utf8_lossy(&repack.stderr).trim()
        ));
    }
    let alternates = git_dir.join("objects").join("info").join("alternates");
    std::fs::remove_file(&alternates).map_err(|error| {
        format!(
            "could not detach private baseline from source objects {}: {error}",
            alternates.display()
        )
    })?;
    let verify = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .args(["cat-file", "-e"])
        .arg(format!("{revision}^{{tree}}"))
        .output()
        .map_err(|error| format!("git verify private baseline failed to spawn: {error}"))?;
    if !verify.status.success() {
        return Err(format!(
            "private baseline is not self-contained: {}",
            String::from_utf8_lossy(&verify.stderr).trim()
        ));
    }
    Ok(())
}

fn source_index_path(cwd: &Path) -> Result<PathBuf, String> {
    source_git_path(cwd, "index")
}

fn source_git_path(cwd: &Path, name: &str) -> Result<PathBuf, String> {
    git_text(
        cwd,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            name,
        ],
        "resolve Git metadata path",
    )
    .map(PathBuf::from)
}

fn copy_index_state(source: &Path, destination: &Path) -> Result<(), String> {
    std::fs::copy(source, destination).map_err(|error| {
        format!(
            "could not copy Git index {} into private storage {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    let (Some(source_dir), Some(destination_dir)) = (source.parent(), destination.parent()) else {
        return Ok(());
    };
    for entry in std::fs::read_dir(source_dir)
        .map_err(|error| format!("could not inspect Git index directory: {error}"))?
    {
        let entry = entry.map_err(|error| format!("could not inspect Git index entry: {error}"))?;
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with("sharedindex.") || !entry.path().is_file() {
            continue;
        }
        std::fs::copy(entry.path(), destination_dir.join(name))
            .map_err(|error| format!("could not copy split Git index state: {error}"))?;
    }
    Ok(())
}

fn git_in_private_repository(
    git_dir: &Path,
    work_tree: &Path,
    index: &Path,
    arguments: &[&str],
    operation: &str,
) -> Result<String, String> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work_tree)
        .args(arguments)
        .env("GIT_INDEX_FILE", index)
        .current_dir(work_tree)
        .output()
        .map_err(|error| format!("git {operation} failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git {operation} returned non-UTF-8 output: {error}"))
}

fn git_in_private_repository_with_input(
    git_dir: &Path,
    work_tree: &Path,
    index: &Path,
    arguments: &[&str],
    input: &[u8],
    operation: &str,
) -> Result<(), String> {
    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(git_dir)
        .arg("--work-tree")
        .arg(work_tree)
        .args(arguments)
        .env("GIT_INDEX_FILE", index)
        .current_dir(work_tree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("git {operation} failed to spawn: {error}"))?;
    child
        .stdin
        .take()
        .ok_or_else(|| format!("git {operation} stdin was unavailable"))?
        .write_all(input)
        .map_err(|error| format!("git {operation} stdin failed: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("git {operation} wait failed: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

fn git_text(cwd: &Path, arguments: &[&str], operation: &str) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(arguments)
        .output()
        .map_err(|error| format!("git {operation} failed to spawn: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {operation} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout)
        .map_err(|error| format!("git {operation} returned non-UTF-8 output: {error}"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("git {operation} returned empty output"));
    }
    Ok(value.to_owned())
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
        let _admin = self
            .baseline
            .inner
            .worktree_admin
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let _ = Command::new("git")
            .arg("--git-dir")
            .arg(&self.baseline.inner.git_dir)
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

    fn git_stdout(args: &[&str], cwd: &Path) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
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
    fn shared_baseline_preserves_dirty_source_without_polluting_child_diffs() {
        let repo = init_repo();
        std::fs::write(repo.path().join("a.txt"), "staged\n").unwrap();
        git(&["add", "a.txt"], repo.path());
        std::fs::write(repo.path().join("a.txt"), "unstaged\n").unwrap();
        std::fs::write(repo.path().join("untracked.txt"), "source context\n").unwrap();
        let status_before = git_stdout(&["status", "--porcelain=v1"], repo.path());
        let cached_before = git_stdout(&["diff", "--cached"], repo.path());

        let baseline = WorktreeBaseline::capture(repo.path()).expect("capture dirty source");
        let same_baseline =
            WorktreeBaseline::capture(repo.path()).expect("repeat deterministic capture");
        assert_eq!(
            baseline.inner.revision, same_baseline.inner.revision,
            "an unchanged source view must produce one deterministic snapshot revision"
        );
        let first = baseline.create().expect("first sibling worktree");
        let second = baseline.create().expect("second sibling worktree");

        for worktree in [&first, &second] {
            assert_eq!(
                std::fs::read_to_string(worktree.path().join("a.txt")).unwrap(),
                "unstaged\n"
            );
            assert_eq!(
                std::fs::read_to_string(worktree.path().join("untracked.txt")).unwrap(),
                "source context\n"
            );
        }

        std::fs::write(first.path().join("a.txt"), "agent change\n").unwrap();
        let diff = first.capture_diff().expect("capture only first sibling delta");
        assert!(diff.contains("agent change"), "{diff}");
        assert!(diff.contains("unstaged"), "{diff}");
        assert!(
            !diff.contains("untracked.txt"),
            "source untracked files belong to the baseline, not the Agent delta:\n{diff}"
        );

        assert_eq!(
            git_stdout(&["status", "--porcelain=v1"], repo.path()),
            status_before,
            "snapshot capture must not alter source working-tree or index state"
        );
        assert_eq!(
            git_stdout(&["diff", "--cached"], repo.path()),
            cached_before,
            "snapshot capture must not alter the user's staged content"
        );
    }

    #[test]
    fn private_baseline_does_not_write_objects_refs_or_worktrees_into_source_repo() {
        let repo = init_repo();
        std::fs::write(repo.path().join("secret-untracked.txt"), "not persistent\n").unwrap();
        let objects_before = git_stdout(&["count-objects", "-v"], repo.path());
        let refs_before = git_stdout(&["show-ref"], repo.path());
        let worktrees_before = git_stdout(&["worktree", "list", "--porcelain"], repo.path());

        let baseline = WorktreeBaseline::capture(repo.path()).expect("private baseline");
        let worktree = baseline.create().expect("private worktree");
        assert_eq!(
            std::fs::read_to_string(worktree.path().join("secret-untracked.txt")).unwrap(),
            "not persistent\n"
        );

        assert_eq!(git_stdout(&["count-objects", "-v"], repo.path()), objects_before);
        assert_eq!(git_stdout(&["show-ref"], repo.path()), refs_before);
        assert_eq!(
            git_stdout(&["worktree", "list", "--porcelain"], repo.path()),
            worktrees_before
        );
    }

    #[test]
    fn private_baseline_is_self_contained_after_source_repository_disappears() {
        let repo = init_repo();
        std::fs::write(repo.path().join("untracked.txt"), "private snapshot\n").unwrap();
        let baseline = WorktreeBaseline::capture(repo.path()).expect("capture source");

        std::fs::remove_dir_all(repo.path()).expect("remove source repository");
        let first = baseline.create().expect("first self-contained worktree");
        let second = baseline.create().expect("second self-contained worktree");

        for worktree in [&first, &second] {
            assert_eq!(
                std::fs::read_to_string(worktree.path().join("a.txt")).unwrap(),
                "original\n"
            );
            assert_eq!(
                std::fs::read_to_string(worktree.path().join("untracked.txt")).unwrap(),
                "private snapshot\n"
            );
        }
    }

    #[test]
    fn source_ignore_attributes_and_safe_git_config_are_projected() {
        let repo = init_repo();
        std::fs::write(repo.path().join(".gitignore"), "ignored-by-tree\n").unwrap();
        let info_exclude = source_git_path(repo.path(), "info/exclude").unwrap();
        std::fs::write(&info_exclude, "ignored-by-info\n").unwrap();
        let external_excludes = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(external_excludes.path(), "ignored-by-core\n").unwrap();
        git(
            &[
                "config",
                "core.excludesFile",
                external_excludes.path().to_str().unwrap(),
            ],
            repo.path(),
        );
        git(&["config", "core.autocrlf", "input"], repo.path());
        git(&["config", "filter.audit.clean", "must-not-be-lost"], repo.path());
        let info_attributes = source_git_path(repo.path(), "info/attributes").unwrap();
        std::fs::write(&info_attributes, "*.crlf text eol=lf\n").unwrap();
        for name in ["ignored-by-tree", "ignored-by-info", "ignored-by-core"] {
            std::fs::write(repo.path().join(name), "secret\n").unwrap();
        }
        std::fs::write(repo.path().join("visible.crlf"), b"line-one\r\nline-two\r\n").unwrap();

        let baseline = WorktreeBaseline::capture(repo.path()).expect("semantic projection");
        let worktree = baseline.create().expect("projected worktree");

        for name in ["ignored-by-tree", "ignored-by-info", "ignored-by-core"] {
            assert!(!worktree.path().join(name).exists(), "{name} leaked into snapshot");
        }
        assert_eq!(
            std::fs::read(worktree.path().join("visible.crlf")).unwrap(),
            b"line-one\nline-two\n"
        );
        assert_eq!(
            git_stdout(&["config", "--get", "filter.audit.clean"], worktree.path()).trim(),
            "must-not-be-lost"
        );
    }

    #[test]
    fn capture_diff_includes_committed_and_binary_agent_changes() {
        let repo = init_repo();
        let baseline = WorktreeBaseline::capture(repo.path()).expect("baseline");
        let worktree = baseline.create().expect("create worktree");
        let apply_target = baseline.create().expect("patch application target");
        git(&["config", "user.email", "agent@localhost"], worktree.path());
        git(&["config", "user.name", "Agent"], worktree.path());
        std::fs::write(worktree.path().join("a.txt"), "committed by agent\n").unwrap();
        let binary = [0_u8, 1, 2, 0, 255, 42, 17];
        std::fs::write(worktree.path().join("image.bin"), binary).unwrap();
        git(&["add", "-A"], worktree.path());
        git(&["commit", "-q", "-m", "agent change"], worktree.path());
        std::fs::write(worktree.path().join("after-commit.txt"), "unstaged delta\n").unwrap();

        let diff = worktree
            .capture_diff()
            .expect("diff against immutable baseline");
        assert!(diff.contains("committed by agent"), "{diff}");
        assert!(diff.contains("image.bin"), "{diff}");
        assert!(diff.contains("GIT binary patch"), "{diff}");
        assert!(diff.contains("after-commit.txt"), "{diff}");

        let patch = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(patch.path(), diff).unwrap();
        git(
            &["apply", "--index", patch.path().to_str().unwrap()],
            apply_target.path(),
        );
        assert_eq!(
            std::fs::read_to_string(apply_target.path().join("a.txt")).unwrap(),
            "committed by agent\n"
        );
        assert_eq!(
            std::fs::read(apply_target.path().join("image.bin")).unwrap(),
            binary
        );
        assert_eq!(
            std::fs::read_to_string(apply_target.path().join("after-commit.txt")).unwrap(),
            "unstaged delta\n"
        );
    }

    #[test]
    fn unstable_source_tree_is_retried_and_fails_closed() {
        let mut changing = ["tree-a", "tree-b", "tree-b"].into_iter();
        assert_eq!(
            select_stable_tree(|| Ok(changing.next().unwrap().to_owned())).unwrap(),
            "tree-b"
        );

        let mut never_stable = ["tree-a", "tree-b", "tree-c"].into_iter();
        let error = select_stable_tree(|| Ok(never_stable.next().unwrap().to_owned()))
            .expect_err("three different captures must fail closed");
        assert!(error.contains("workspace changed"), "{error}");
    }

    #[test]
    fn one_baseline_serializes_concurrent_worktree_registration() {
        use std::sync::{Arc, Barrier};

        let repo = init_repo();
        let baseline = Arc::new(WorktreeBaseline::capture(repo.path()).expect("baseline"));
        let barrier = Arc::new(Barrier::new(8));
        let threads = (0..8)
            .map(|_| {
                let baseline = Arc::clone(&baseline);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    let worktree = baseline.create().expect("concurrent worktree");
                    worktree.verified_path().expect("registered worktree");
                    assert_eq!(
                        std::fs::read_to_string(worktree.path().join("a.txt")).unwrap(),
                        "original\n"
                    );
                })
            })
            .collect::<Vec<_>>();
        for thread in threads {
            thread.join().expect("worktree thread");
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_file_mode_and_symlink_semantics_survive_the_private_snapshot() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let repo = init_repo();
        let executable = repo.path().join("run.sh");
        std::fs::write(&executable, "#!/bin/sh\n").unwrap();
        let mut permissions = std::fs::metadata(&executable).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).unwrap();
        symlink("a.txt", repo.path().join("a-link")).unwrap();

        let worktree = Worktree::create(repo.path()).expect("unix semantics");

        assert_ne!(
            std::fs::metadata(worktree.path().join("run.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );
        assert!(
            std::fs::symlink_metadata(worktree.path().join("a-link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_link(worktree.path().join("a-link")).unwrap(),
            PathBuf::from("a.txt")
        );
    }

    #[test]
    fn split_index_and_linked_worktree_sources_are_supported() {
        let repo = init_repo();
        git(&["update-index", "--split-index"], repo.path());
        let linked_parent = tempfile::tempdir().unwrap();
        let linked = linked_parent.path().join("linked");
        let output = Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["worktree", "add", "--detach"])
            .arg(&linked)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        std::fs::write(linked.join("a.txt"), "linked dirty state\n").unwrap();
        std::fs::write(linked.join("linked-untracked.txt"), "linked context\n").unwrap();

        let worktree = Worktree::create(&linked).expect("linked split-index source");

        assert_eq!(
            std::fs::read_to_string(worktree.path().join("a.txt")).unwrap(),
            "linked dirty state\n"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.path().join("linked-untracked.txt")).unwrap(),
            "linked context\n"
        );
    }

    #[test]
    fn unborn_git_repository_can_still_form_an_isolation_baseline() {
        let repo = tempfile::tempdir().unwrap();
        git(&["init", "-q"], repo.path());
        std::fs::write(repo.path().join("first.txt"), "unborn source\n").unwrap();
        let source_index = source_index_path(repo.path()).unwrap();
        assert!(!source_index.exists());

        let worktree = Worktree::create(repo.path()).expect("capture unborn repository");
        assert_eq!(
            std::fs::read_to_string(worktree.path().join("first.txt")).unwrap(),
            "unborn source\n"
        );
        std::fs::write(worktree.path().join("first.txt"), "agent delta\n").unwrap();
        let diff = worktree.capture_diff().expect("unborn baseline diff");
        assert!(diff.contains("agent delta"), "{diff}");
        assert!(
            !source_index.exists(),
            "private baseline must not create the source repository index"
        );
    }

    #[test]
    fn worktree_is_removed_on_drop() {
        let repo = init_repo();
        let (path, private_storage) = {
            let wt = Worktree::create(repo.path()).expect("create");
            (
                wt.path().to_path_buf(),
                wt.baseline.inner._storage.path().to_path_buf(),
            )
        }; // dropped here
        assert!(!path.exists(), "worktree dir must be removed on drop");
        assert!(
            !private_storage.exists(),
            "private objects and indexes must be removed with the baseline"
        );
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
            .arg("--git-dir")
            .arg(&wt.baseline.inner.git_dir)
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
