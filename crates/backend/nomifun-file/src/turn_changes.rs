//! Read-only, bounded workspace observations for one Turn's display summary.
//!
//! This module neither owns Session facts nor proves tool effects. The caller
//! coordinates the two observations with its exact Turn lifecycle. Only net
//! regular-file changes in the same retained workspace identity are returned.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

use cap_fs_ext::{DirExt as _, FollowSymlinks, OpenOptionsFollowExt as _};
use cap_std::fs::{Dir, OpenOptions};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use nomifun_common::FileChangeOperation;
use same_file::Handle;
use sha2::{Digest, Sha256};

const READ_CHUNK_BYTES: usize = 64 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_RULE_BYTES: u64 = 256 * 1024;
const MAX_RULE_LINES: usize = 4096;
const MAX_RULE_LINE_BYTES: usize = 4096;
const EXCLUDED_DIRECTORIES: &[&str] = &[
    ".git", ".nomifun", "node_modules", "dist", "build", "target", ".venv",
    "__pycache__", ".next", ".nuxt", ".output",
];

/// Cooperative limits apply to enumeration, rule reads and streamed hashing.
/// A blocking filesystem syscall itself cannot be interrupted; no unbounded
/// worker is left running behind an async timeout wrapper.
#[derive(Clone, Debug)]
pub struct TurnChangeLimits {
    pub max_files: usize,
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    pub max_duration: Duration,
    pub cancellation: Option<Arc<AtomicBool>>,
}

impl Default for TurnChangeLimits {
    fn default() -> Self {
        Self {
            max_files: 20_000,
            max_entries: 100_000,
            max_file_bytes: 32 * 1024 * 1024,
            max_total_bytes: 128 * 1024 * 1024,
            max_duration: Duration::from_secs(2),
            cancellation: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileFingerprint { size_bytes: u64, sha256: String }

/// Ephemeral baseline, intentionally not serializable or a second file ledger.
/// The retained root identity detects path replacement between observations.
#[derive(Debug)]
pub struct WorkspaceFileSnapshot {
    pub root_sha256: String,
    pub complete: bool,
    pub reasons: Vec<String>,
    root: PathBuf,
    root_identity: Option<Handle>,
    files: BTreeMap<String, FileFingerprint>,
    present: BTreeSet<String>,
    enumeration_complete: bool,
    rules: BTreeMap<String, String>,
    scopes: BTreeMap<PathBuf, Rules>,
    skipped: BTreeSet<String>,
}

impl WorkspaceFileSnapshot {
    /// Canonical root used by this observation; no mutable path authority.
    pub fn root_path(&self) -> &Path { &self.root }

    /// Digest verified during capture. Excluded, absent and unreadable paths
    /// return None; this accessor never inspects current filesystem state.
    pub fn file_sha256(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(|file| file.sha256.as_str())
    }

    fn includes(&self, path: &str) -> bool {
        let relative = Path::new(path);
        if portable(relative).ok().as_deref() != Some(path)
            || relative.components().count() > MAX_DEPTH + 1
            || relative.components().any(|part| excluded_name(part.as_os_str())) {
            return false;
        }
        let mut candidate = PathBuf::new();
        let mut key = String::new();
        let count = relative.components().count();
        for (index, part) in relative.components().enumerate() {
            candidate.push(part.as_os_str());
            if index > 0 { key.push('/'); }
            key.push_str(part.as_os_str().to_str().expect("portable path verified"));
            let directory = index + 1 < count;
            if self.skipped.contains(&key) || (directory && self.present.contains(&key))
                || ignored_in_scope(&self.scopes, &candidate, directory) { return false; }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceFileChange {
    /// Exact workspace-relative path, using forward slashes.
    pub path: String,
    pub operation: FileChangeOperation,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug)]
pub struct WorkspaceFileChanges {
    pub root_sha256: String,
    pub complete: bool,
    pub files: Vec<WorkspaceFileChange>,
}

/// Capture through retained directory handles without following descendants'
/// symlinks, mutating Git state, or reading ambient ignore files above root.
pub fn capture_workspace_files(root: &Path, limits: TurnChangeLimits) -> WorkspaceFileSnapshot {
    capture_with_hook(root, limits, |_, _| {})
}

/// Missing entries prove create/delete only after complete enumeration and
/// inclusion in both captured scopes. Changed ignore rules mark the summary
/// incomplete without hiding unrelated, confidently observed file changes.
pub fn compare_workspace_files(before: &WorkspaceFileSnapshot, after: &WorkspaceFileSnapshot) -> WorkspaceFileChanges {
    let same_root = before.root == after.root && before.root_identity.is_some()
        && before.root_identity == after.root_identity;
    let complete = same_root && before.complete && after.complete && before.rules == after.rules;
    let known_absence = same_root && before.enumeration_complete && after.enumeration_complete;
    let mut result = WorkspaceFileChanges { root_sha256: after.root_sha256.clone(), complete, files: Vec::new() };
    if !same_root { return result; }
    let paths = before.files.keys().chain(after.files.keys()).collect::<BTreeSet<_>>();
    for path in paths {
        let (operation, source) = match (before.files.get(path), after.files.get(path)) {
            (Some(old), Some(new)) if old != new => (FileChangeOperation::Modify, Some(new)),
            (None, Some(new)) if known_absence && !before.present.contains(path) && before.includes(path) && after.includes(path) => (FileChangeOperation::Create, Some(new)),
            (Some(_), None) if known_absence && !after.present.contains(path) && before.includes(path) && after.includes(path) => (FileChangeOperation::Delete, None),
            _ => continue,
        };
        result.files.push(WorkspaceFileChange {
            path: path.clone(), operation,
            size_bytes: source.map(|value| value.size_bytes),
            sha256: source.map(|value| value.sha256.clone()),
        });
    }
    result
}

/// Receipt paths can be suppressed as net-zero only when both observations
/// actually read the same content, or both completely enumerated observations
/// prove absence inside their captured included scope. Excluded, linked and
/// unreadable paths are never guessed unchanged from an empty hash lookup.
pub fn unchanged_workspace_paths(before: &WorkspaceFileSnapshot, after: &WorkspaceFileSnapshot, paths: &[String]) -> Vec<String> {
    if before.root != after.root || before.root_identity.is_none() || before.root_identity != after.root_identity { return Vec::new(); }
    let known_absence = before.enumeration_complete && after.enumeration_complete;
    paths.iter().filter(|path| {
        before.files.get(*path).is_some_and(|old| after.files.get(*path) == Some(old))
            || (known_absence && !before.present.contains(*path) && !after.present.contains(*path)
                && !before.scopes.contains_key(Path::new(path)) && !after.scopes.contains_key(Path::new(path))
                && before.includes(path) && after.includes(path))
    }).cloned().collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapturePhase { DirectoryOpened, FileOpened, ChunkRead }

#[derive(Clone, Debug)]
struct Rules { ignore: Gitignore, gitignore: Gitignore }

struct Capture<F> {
    limits: TurnChangeLimits,
    started: Instant,
    entries: usize,
    bytes: u64,
    rule_lines: usize,
    files: BTreeMap<String, FileFingerprint>,
    present: BTreeSet<String>,
    enumeration_complete: bool,
    rules: BTreeMap<String, String>,
    scopes: BTreeMap<PathBuf, Rules>,
    skipped: BTreeSet<String>,
    reasons: BTreeSet<String>,
    hook: F,
}

fn capture_with_hook(root: &Path, limits: TurnChangeLimits, hook: impl FnMut(CapturePhase, &Path)) -> WorkspaceFileSnapshot {
    let mut scan = Capture { limits, started: Instant::now(), entries: 0, bytes: 0, rule_lines: 0,
        files: BTreeMap::new(), present: BTreeSet::new(), enumeration_complete: true,
        rules: BTreeMap::new(), scopes: BTreeMap::new(), skipped: BTreeSet::new(), reasons: BTreeSet::new(), hook };
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let root_sha256 = format!("{:x}", Sha256::digest(canonical.as_os_str().as_encoded_bytes()));
    let mut root_identity = None;
    if scan.running() {
        match open_root(&canonical).and_then(|directory| Ok((identity(&directory)?, directory))) {
            Ok((pinned, directory)) => {
                root_identity = Some(pinned);
                scan.directory(&directory, Path::new(""), &mut Vec::new(), 0);
                // A replaced root invalidates every result, even if the retained
                // old directory was still readable somewhere else.
                if open_root(&canonical).and_then(|directory| identity(&directory)).ok().as_ref() != root_identity.as_ref() {
                    scan.walk_failure("workspace_identity_changed");
                    scan.files.clear();
                    root_identity = None;
                }
            }
            Err(_) => { scan.walk_failure("workspace_unavailable"); }
        }
    }
    WorkspaceFileSnapshot { root_sha256, complete: scan.reasons.is_empty(), reasons: scan.reasons.into_iter().collect(),
        root: canonical, root_identity, files: scan.files, present: scan.present,
        enumeration_complete: scan.enumeration_complete, rules: scan.rules, scopes: scan.scopes, skipped: scan.skipped }
}

impl<F: FnMut(CapturePhase, &Path)> Capture<F> {
    fn running(&mut self) -> bool {
        let reason = if self.limits.cancellation.as_ref().is_some_and(|flag| flag.load(Ordering::Acquire)) {
            Some("cancelled")
        } else if self.started.elapsed() >= self.limits.max_duration {
            Some("time_budget")
        } else { None };
        if let Some(reason) = reason { self.walk_failure(reason); false } else { true }
    }

    fn walk_failure(&mut self, reason: &str) {
        self.enumeration_complete = false;
        self.reasons.insert(reason.into());
    }

    fn directory(&mut self, directory: &Dir, relative: &Path, inherited: &mut Vec<Rules>, depth: usize) {
        if !self.running() { return; }
        (self.hook)(CapturePhase::DirectoryOpened, relative);
        let rules = match self.load_rules(directory, relative) {
            Ok(rules) => rules,
            Err(reason) => { self.walk_failure(reason); return; }
        };
        self.scopes.insert(relative.to_owned(), rules.clone());
        inherited.push(rules);
        let entries = match directory.entries() {
            Ok(entries) => entries,
            Err(_) => { self.walk_failure("unreadable_directory"); inherited.pop(); return; }
        };
        for entry in entries {
            if !self.running() { break; }
            if self.entries >= self.limits.max_entries { self.walk_failure("entry_budget"); break; }
            self.entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => { self.walk_failure("unreadable_entry"); continue; }
            };
            let name = entry.file_name();
            // Apply private-directory exclusions before even inspecting them.
            if excluded_name(&name) { continue; }
            let path = relative.join(&name);
            let metadata = match directory.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(_) => { self.walk_failure("changed_or_unreadable_entry"); continue; }
            };
            if metadata.file_type().is_symlink() || (!metadata.is_file() && !metadata.is_dir()) {
                if let Ok(path) = portable(&path) { self.skipped.insert(path); }
                continue;
            }
            if ignored(inherited, &path, metadata.is_dir()) {
                if let Ok(path) = portable(&path) { self.skipped.insert(path); }
                continue;
            }
            if metadata.is_dir() {
                if depth >= MAX_DEPTH { self.walk_failure("depth_budget"); continue; }
                let child = match directory.open_dir_nofollow(&name) {
                    Ok(child) => child,
                    Err(_) => { self.walk_failure("changed_or_unreadable_directory"); continue; }
                };
                let before = identity(&child).ok();
                self.directory(&child, &path, inherited, depth + 1);
                let after = directory.open_dir_nofollow(&name).and_then(|child| identity(&child)).ok();
                if before.is_none() || before != after {
                    self.walk_failure("directory_identity_changed");
                    // Never retain observations under a detached name.
                    if let Ok(prefix) = portable(&path) { self.files.retain(|key, _| !key.starts_with(&format!("{prefix}/"))); }
                }
            } else {
                if self.present.len() >= self.limits.max_files { self.walk_failure("file_budget"); break; }
                let path_key = match portable(&path) {
                    Ok(path) => path,
                    Err(_) => { self.walk_failure("unrepresentable_path"); continue; }
                };
                self.present.insert(path_key.clone());
                match self.read_file(directory, &name, &path, self.limits.max_file_bytes, false) {
                    Ok((fingerprint, _)) => { self.files.insert(path_key, fingerprint); }
                    Err(reason) => { self.reasons.insert(reason.into()); }
                }
            }
        }
        inherited.pop();
    }

    fn load_rules(&mut self, directory: &Dir, relative: &Path) -> Result<Rules, &'static str> {
        let mut matchers = Vec::new();
        for name in [".ignore", ".gitignore"] {
            if !self.running() { return Err("time_or_cancel_budget"); }
            let metadata = match directory.symlink_metadata(name) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == io::ErrorKind::NotFound => { matchers.push(Gitignore::empty()); continue; }
                Err(_) => return Err("unreadable_ignore_source"),
            };
            if !metadata.is_file() || metadata.file_type().is_symlink() { return Err("linked_or_invalid_ignore_source"); }
            let path = relative.join(name);
            let (fingerprint, bytes) = self.read_file(directory, OsStr::new(name), &path, MAX_RULE_BYTES, true)?;
            self.rules.insert(portable(&path).map_err(|_| "unrepresentable_path")?, fingerprint.sha256);
            let text = std::str::from_utf8(&bytes).map_err(|_| "invalid_ignore_source")?;
            let mut builder = GitignoreBuilder::new(relative);
            for (index, line) in text.lines().enumerate() {
                if !self.running() { return Err("time_or_cancel_budget"); }
                if self.rule_lines >= MAX_RULE_LINES || line.len() > MAX_RULE_LINE_BYTES { return Err("ignore_rule_budget"); }
                self.rule_lines += 1;
                let line = if index == 0 { line.trim_start_matches('\u{feff}') } else { line };
                builder.add_line(None, line).map_err(|_| "invalid_ignore_pattern")?;
            }
            matchers.push(builder.build().map_err(|_| "invalid_ignore_pattern")?);
        }
        Ok(Rules { ignore: matchers.remove(0), gitignore: matchers.remove(0) })
    }

    fn read_file(&mut self, directory: &Dir, name: &OsStr, relative: &Path, max_bytes: u64, retain: bool) -> Result<(FileFingerprint, Vec<u8>), &'static str> {
        if !self.running() { return Err("time_or_cancel_budget"); }
        let mut file = directory.open_with(name, &read_options()).map_err(|_| "changed_or_unreadable_file")?.into_std();
        let metadata = file.metadata().map_err(|_| "unreadable_file_metadata")?;
        if !metadata.is_file() { return Err("changed_file_type"); }
        if metadata.len() > max_bytes { return Err("file_byte_budget"); }
        if metadata.len() > self.limits.max_total_bytes.saturating_sub(self.bytes) { return Err("total_byte_budget"); }
        let before = Handle::from_file(file.try_clone().map_err(|_| "unreadable_file_identity")?).map_err(|_| "unreadable_file_identity")?;
        (self.hook)(CapturePhase::FileOpened, relative);
        let mut digest = Sha256::new();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; READ_CHUNK_BYTES];
        let mut total = 0_u64;
        loop {
            if !self.running() { return Err("time_or_cancel_budget"); }
            let allowed = max_bytes.saturating_sub(total).min(self.limits.max_total_bytes.saturating_sub(self.bytes));
            // A one-byte probe distinguishes exact budget EOF from growth.
            let limit = allowed.saturating_add(1).min(buffer.len() as u64) as usize;
            let count = file.read(&mut buffer[..limit]).map_err(|_| "unreadable_file")?;
            self.bytes = self.bytes.saturating_add(count as u64);
            total = total.saturating_add(count as u64);
            if total > max_bytes || self.bytes > self.limits.max_total_bytes { return Err("file_or_total_byte_budget"); }
            if count == 0 { break; }
            digest.update(&buffer[..count]);
            if retain { bytes.extend_from_slice(&buffer[..count]); }
            (self.hook)(CapturePhase::ChunkRead, relative);
        }
        let after = file.metadata().map_err(|_| "unreadable_file_metadata")?;
        let current = directory.open_with(name, &read_options()).map_err(|_| "file_identity_changed")?.into_std();
        let current = Handle::from_file(current).map_err(|_| "unreadable_file_identity")?;
        if before != current || total != metadata.len() || metadata.len() != after.len()
            || metadata.modified().ok() != after.modified().ok() {
            return Err("file_changed_during_read");
        }
        Ok((FileFingerprint { size_bytes: total, sha256: format!("{:x}", digest.finalize()) }, bytes))
    }
}

fn ignored(rules: &[Rules], relative: &Path, directory: bool) -> bool {
    // .ignore precedence is consistent with existing workspace search rules.
    for rules in rules.iter().rev() { let found = rules.ignore.matched(relative, directory); if !found.is_none() { return found.is_ignore(); } }
    for rules in rules.iter().rev() { let found = rules.gitignore.matched(relative, directory); if !found.is_none() { return found.is_ignore(); } }
    false
}

fn ignored_in_scope(scopes: &BTreeMap<PathBuf, Rules>, relative: &Path, directory: bool) -> bool {
    // Simulate top-down inclusion using only matchers loaded at capture time.
    // A child whitelist cannot resurrect a directory the walk had excluded.
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    for git in [false, true] {
        for ancestor in parent.ancestors() {
            let Some(rules) = scopes.get(ancestor) else { continue; };
            let matcher = if git { &rules.gitignore } else { &rules.ignore };
            let found = matcher.matched(relative, directory);
            if !found.is_none() { return found.is_ignore(); }
        }
    }
    false
}

fn excluded_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else { return false; };
    EXCLUDED_DIRECTORIES.iter().any(|excluded| name.eq_ignore_ascii_case(excluded))
        || matches!(name, ".DS_Store" | "Thumbs.db" | ".env" | ".env.local") || name.ends_with(".pyc")
}

fn portable(relative: &Path) -> io::Result<String> {
    let mut parts = Vec::new();
    for part in relative.components() {
        let std::path::Component::Normal(value) = part else { return Err(io::ErrorKind::InvalidInput.into()); };
        let value = value.to_str().ok_or(io::ErrorKind::InvalidInput)?;
        if crate::path_safety::is_unsafe_path_segment(value) || value.chars().any(char::is_control) { return Err(io::ErrorKind::InvalidInput.into()); }
        parts.push(value);
    }
    let path = parts.join("/");
    if path.len() > 4096 { return Err(io::ErrorKind::InvalidInput.into()); }
    Ok(path)
}

fn identity(directory: &Dir) -> io::Result<Handle> { Handle::from_file(directory.try_clone()?.into_std_file()) }

fn open_root(root: &Path) -> io::Result<Dir> {
    if !root.is_absolute() || std::fs::canonicalize(root)? != root {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    // Only the filesystem root is ambient. Every remaining component is
    // descriptor-relative and nofollow, including the workspace ancestors.
    // A root/ancestor swapped to a link after canonicalization cannot redirect
    // any reads into another tree, even when the final verification would fail.
    let filesystem_root = root.ancestors().last().ok_or(io::ErrorKind::InvalidInput)?;
    let mut directory = Dir::open_ambient_dir(filesystem_root, cap_std::ambient_authority())?;
    for component in root.components() {
        if let std::path::Component::Normal(name) = component { directory = directory.open_dir_nofollow(name)?; }
    }
    Ok(directory)
}

fn read_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(unix)] { use cap_std::fs::OpenOptionsExt as _; options.custom_flags(libc::O_NONBLOCK); }
    options
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn capture(root: &Path) -> WorkspaceFileSnapshot { capture_workspace_files(root, TurnChangeLimits::default()) }

    #[test]
    fn non_git_script_and_binary_document_are_both_created() {
        let root = tempfile::tempdir().unwrap();
        let before = capture(root.path());
        fs::write(root.path().join("build_docx.py"), b"generate_docx()").unwrap();
        fs::write(root.path().join("骆驼祥子读后感.docx"), [0x50, 0x4b, 0xff, 0x00, 0x80]).unwrap();
        let after = capture(root.path());
        let changes = compare_workspace_files(&before, &after);
        assert!(changes.complete, "{:?}", after.reasons);
        assert_eq!(changes.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), ["build_docx.py", "骆驼祥子读后感.docx"]);
        assert!(changes.files.iter().all(|file| file.operation == FileChangeOperation::Create && file.sha256.as_ref().is_some_and(|hash| hash.len() == 64)));
        assert!(!root.path().join(".git").exists(), "observation must not initialize a repository");
    }

    #[test]
    fn net_changes_preserve_modifications_deletes_and_duplicate_names() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        for name in ["same.txt", "nested/same.txt", "deleted.txt", "reverted.txt"] { fs::write(root.path().join(name), b"before").unwrap(); }
        let before = capture(root.path());
        fs::write(root.path().join("same.txt"), b"after!").unwrap();
        fs::write(root.path().join("nested/same.txt"), b"updated").unwrap();
        fs::remove_file(root.path().join("deleted.txt")).unwrap();
        fs::write(root.path().join("reverted.txt"), b"intermediate").unwrap();
        fs::write(root.path().join("reverted.txt"), b"before").unwrap();
        fs::write(root.path().join("temporary.txt"), b"created").unwrap();
        fs::remove_file(root.path().join("temporary.txt")).unwrap();
        let after = capture(root.path());
        assert_eq!(before.file_sha256("deleted.txt"), Some(format!("{:x}", Sha256::digest(b"before")).as_str()));
        assert_eq!(after.file_sha256("deleted.txt"), None);
        assert_eq!(after.file_sha256("missing.txt"), None);
        assert_ne!(before.file_sha256("same.txt"), after.file_sha256("same.txt"));
        // Historical access must not read bytes written after the capture.
        fs::write(root.path().join("same.txt"), b"later").unwrap();
        assert_eq!(after.file_sha256("same.txt"), Some(format!("{:x}", Sha256::digest(b"after!")).as_str()));
        let changes = compare_workspace_files(&before, &after);
        assert!(changes.complete);
        assert_eq!(changes.files.iter().map(|file| (file.path.as_str(), file.operation)).collect::<Vec<_>>(), [
            ("deleted.txt", FileChangeOperation::Delete), ("nested/same.txt", FileChangeOperation::Modify), ("same.txt", FileChangeOperation::Modify),
        ]);
        assert_eq!(changes.files[0].size_bytes, None);
        assert_eq!(changes.files[0].sha256, None);
        assert_eq!(unchanged_workspace_paths(&before, &after, &["reverted.txt".into(), "temporary.txt".into()]), ["reverted.txt", "temporary.txt"]);
    }

    #[test]
    fn exclusions_and_nested_ignore_rules_do_not_read_ambient_sources() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("workspace");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(fixture.path().join(".gitignore"), b"*\n").unwrap();
        fs::write(root.join(".gitignore"), b"*.log\nignored/\n").unwrap();
        fs::write(root.join(".ignore"), b"priority.txt\n").unwrap();
        fs::write(root.join("nested/.gitignore"), b"!keep.log\n!priority.txt\n").unwrap();
        fs::write(root.join("nested/.ignore"), b"!priority.txt\n").unwrap();
        for name in ["visible.txt", "drop.log", "priority.txt", "nested/keep.log", "nested/priority.txt", ".hidden"] { fs::write(root.join(name), b"source").unwrap(); }
        for name in EXCLUDED_DIRECTORIES.iter().copied().chain(["ignored"]) {
            fs::create_dir_all(root.join(name)).unwrap();
            fs::write(root.join(name).join("never.txt"), b"excluded").unwrap();
        }
        for name in [".env", ".env.local", "test.pyc", ".DS_Store", "Thumbs.db"] { fs::write(root.join(name), b"excluded").unwrap(); }
        let snapshot = capture(&root);
        assert!(snapshot.complete, "{:?}", snapshot.reasons);
        assert!(snapshot.files.contains_key("visible.txt"));
        assert!(snapshot.files.contains_key(".hidden"));
        assert!(snapshot.files.contains_key("nested/keep.log"));
        assert!(snapshot.files.contains_key("nested/priority.txt"));
        assert!(!snapshot.files.contains_key("priority.txt"));
        assert!(!snapshot.files.contains_key("drop.log"));
        assert!(snapshot.files.keys().all(|path| !path.ends_with("never.txt")));
    }

    #[test]
    fn incomplete_scan_cannot_turn_unread_files_into_deletes_or_creates() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), b"before").unwrap();
        fs::write(root.path().join("b.txt"), b"unchanged").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join("a.txt"), b"after!").unwrap();
        let partial = capture_workspace_files(root.path(), TurnChangeLimits { max_file_bytes: 6, ..Default::default() });
        assert!(!partial.complete);
        assert!(!partial.files.contains_key("b.txt"));
        let changes = compare_workspace_files(&before, &partial);
        assert!(!changes.complete);
        assert_eq!(changes.files.iter().map(|file| (file.path.as_str(), file.operation)).collect::<Vec<_>>(), [("a.txt", FileChangeOperation::Modify)]);
        assert!(compare_workspace_files(&partial, &before).files.iter().all(|file| file.operation == FileChangeOperation::Modify));
        assert!(unchanged_workspace_paths(&before, &partial, &["b.txt".into()]).is_empty());
    }

    #[test]
    fn changed_ignore_scope_does_not_invent_deletions() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), b"# before\n").unwrap();
        fs::write(root.path().join("kept.txt"), b"source").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join(".gitignore"), b"kept.txt\n").unwrap();
        let after = capture(root.path());
        assert!(before.complete && after.complete);
        let changes = compare_workspace_files(&before, &after);
        assert!(!changes.complete);
        assert_eq!(changes.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), [".gitignore"]);
    }

    #[test]
    fn adding_ignore_rules_keeps_unrelated_new_document_without_false_deletions() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cache.bin"), b"existing").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join(".gitignore"), b"cache.bin\n").unwrap();
        fs::write(root.path().join("output.docx"), b"PK\0\0").unwrap();
        let after = capture(root.path());
        let changes = compare_workspace_files(&before, &after);
        assert!(!changes.complete);
        assert_eq!(changes.files.iter().map(|file| (file.path.as_str(), file.operation)).collect::<Vec<_>>(), [
            (".gitignore", FileChangeOperation::Create), ("output.docx", FileChangeOperation::Create),
        ]);
        // Removing the rules cannot fabricate a creation of the old cache.
        let reverse = compare_workspace_files(&after, &before);
        assert_eq!(reverse.files.iter().map(|file| (file.path.as_str(), file.operation)).collect::<Vec<_>>(), [
            (".gitignore", FileChangeOperation::Delete), ("output.docx", FileChangeOperation::Delete),
        ]);
        assert_eq!(unchanged_workspace_paths(&before, &after, &["transient.docx".into(), "cache.bin".into()]), ["transient.docx"]);
    }

    #[test]
    fn oversized_unrelated_file_does_not_hide_confirmed_new_document() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("large.bin"), vec![5; 100]).unwrap();
        let limits = TurnChangeLimits { max_file_bytes: 10, ..Default::default() };
        let before = capture_workspace_files(root.path(), limits.clone());
        fs::write(root.path().join("output.docx"), b"PK\0\0").unwrap();
        let after = capture_workspace_files(root.path(), limits);
        assert!(!before.complete && !after.complete);
        let changes = compare_workspace_files(&before, &after);
        assert!(!changes.complete);
        assert_eq!(changes.files.iter().map(|file| (file.path.as_str(), file.operation)).collect::<Vec<_>>(), [("output.docx", FileChangeOperation::Create)]);
    }

    #[test]
    fn nominated_create_then_shell_delete_is_zero_only_in_captured_included_scope() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(".gitignore"), b"ignored/\n*.log\n").unwrap();
        fs::write(root.path().join(".ignore"), b"!rescued.docx\n").unwrap();
        fs::create_dir(root.path().join("ignored")).unwrap();
        fs::create_dir(root.path().join("nested")).unwrap();
        fs::write(root.path().join("nested/.gitignore"), b"private/\n").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join("temporary.docx"), b"PK").unwrap();
        fs::remove_file(root.path().join("temporary.docx")).unwrap();
        let after = capture(root.path());
        // Later rules are not consulted when checking the retained snapshots.
        fs::write(root.path().join(".gitignore"), b"temporary.docx\n").unwrap();
        let nominated = ["temporary.docx", "new/transient.docx", "ignored", "nested", "ignored/missing.docx", "nested/private/missing.docx",
            "missing.log", "ignored/rescued.docx", "node_modules/missing.docx", ".nomifun/missing.docx", "../outside.docx"]
            .map(String::from);
        assert_eq!(unchanged_workspace_paths(&before, &after, &nominated), ["temporary.docx", "new/transient.docx"]);
        let incomplete = capture_workspace_files(root.path(), TurnChangeLimits { max_entries: 0, ..Default::default() });
        assert!(unchanged_workspace_paths(&before, &incomplete, &nominated).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn absent_path_beneath_symlink_is_never_proven_unchanged() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("alias")).unwrap();
        let before = capture(root.path());
        let after = capture(root.path());
        assert!(before.complete && after.complete);
        assert!(unchanged_workspace_paths(&before, &after, &["alias/temporary.docx".into()]).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_display_path_marks_coverage_incomplete_without_poisoning_known_changes() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("valid.txt"), b"before").unwrap();
        fs::write(root.path().join("line\nbreak.txt"), b"source").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join("valid.txt"), b"after!").unwrap();
        let after = capture(root.path());
        assert!(!before.complete && !after.complete);
        let changes = compare_workspace_files(&before, &after);
        assert_eq!(changes.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(), ["valid.txt"]);
        assert!(changes.files.iter().all(|file| !file.path.chars().any(char::is_control)));
    }

    #[test]
    fn byte_file_entry_and_elapsed_budgets_are_cooperative() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("large.bin"), vec![5; READ_CHUNK_BYTES * 2]).unwrap();
        for limits in [
            TurnChangeLimits { max_total_bytes: 1, ..Default::default() },
            TurnChangeLimits { max_file_bytes: 1, ..Default::default() },
            TurnChangeLimits { max_files: 0, ..Default::default() },
            TurnChangeLimits { max_entries: 0, ..Default::default() },
            TurnChangeLimits { max_duration: Duration::ZERO, ..Default::default() },
        ] {
            let snapshot = capture_workspace_files(root.path(), limits);
            assert!(!snapshot.complete, "{:?}", snapshot.reasons);
            assert!(snapshot.files.is_empty());
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let hook_cancel = cancelled.clone();
        let mut chunks = 0;
        let snapshot = capture_with_hook(root.path(), TurnChangeLimits { cancellation: Some(cancelled), ..Default::default() }, |phase, _| {
            if phase == CapturePhase::ChunkRead { chunks += 1; hook_cancel.store(true, Ordering::Release); }
        });
        assert_eq!(chunks, 1, "cancellation must be checked between hashing chunks");
        assert!(!snapshot.complete);
        assert!(snapshot.reasons.iter().any(|reason| reason == "cancelled"));
        assert!(snapshot.files.is_empty());
    }

    #[test]
    fn bad_ignore_rules_stop_that_subtree_without_false_absence() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("safe.txt"), b"source").unwrap();
        let before = capture(root.path());
        fs::write(root.path().join(".gitignore"), b"[z-a]\n").unwrap();
        let after = capture(root.path());
        assert!(!after.complete);
        assert!(after.reasons.iter().any(|reason| reason == "invalid_ignore_pattern"));
        assert!(compare_workspace_files(&before, &after).files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn links_never_supply_outside_or_private_content() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(".nomifun")).unwrap();
        fs::write(root.path().join(".nomifun/secret"), b"secret").unwrap();
        fs::write(outside.path().join("sentinel"), b"outside").unwrap();
        symlink(outside.path(), root.path().join("outside")).unwrap();
        symlink(outside.path().join("sentinel"), root.path().join("file-link")).unwrap();
        symlink(".nomifun", root.path().join("private-link")).unwrap();
        let snapshot = capture(root.path());
        assert!(snapshot.complete);
        assert!(snapshot.files.is_empty());
        symlink(outside.path().join("sentinel"), root.path().join(".gitignore")).unwrap();
        let snapshot = capture(root.path());
        assert!(!snapshot.complete);
        assert!(snapshot.files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn root_replacement_invalidates_observations_and_cross_turn_comparison() {
        use std::os::unix::fs::symlink;
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let moved = fixture.path().join("moved");
        let outside = fixture.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("inside.txt"), b"inside").unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        let before = capture(&root);
        let mut replaced = false;
        let after = capture_with_hook(&root, TurnChangeLimits::default(), |phase, _| {
            if phase == CapturePhase::DirectoryOpened && !replaced {
                replaced = true;
                fs::rename(&root, &moved).unwrap();
                symlink(&outside, &root).unwrap();
            }
        });
        assert!(!after.complete);
        assert!(after.files.is_empty());
        assert!(compare_workspace_files(&before, &after).files.is_empty());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("new.txt"), b"new").unwrap();
        let new_root = capture(&root);
        let changes = compare_workspace_files(&before, &new_root);
        assert!(!changes.complete);
        assert!(changes.files.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn directory_replacement_never_traverses_outside_link() {
        use std::os::unix::fs::symlink;
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("root");
        let outside = fixture.path().join("outside");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("nested/inside.txt"), b"inside").unwrap();
        fs::write(outside.join("sentinel"), b"outside").unwrap();
        let snapshot = capture_with_hook(&root, TurnChangeLimits::default(), |phase, path| {
            if phase == CapturePhase::DirectoryOpened && path == Path::new("nested") {
                fs::rename(root.join("nested"), fixture.path().join("moved")).unwrap();
                symlink(&outside, root.join("nested")).unwrap();
            }
        });
        assert!(!snapshot.complete);
        assert!(snapshot.files.is_empty());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"outside");
    }
}
