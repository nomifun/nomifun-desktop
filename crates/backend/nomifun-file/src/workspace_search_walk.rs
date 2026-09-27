//! Bounded workspace traversal. Ignore files are data read through workspace
//! authority, never ambient inputs from the host's parent directories.
use std::{collections::BTreeSet, path::{Path, PathBuf}, time::{Duration, Instant}};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use nomifun_common::AppError;
use crate::{PathAuthority, workspace_read_dir::{self, EntryKind, Entries}};
use crate::agent_text_search::{MAX_FILE_BYTES, MAX_SOURCE_BYTES};

const MAX_RULE_LINES: usize = 4096;
const MAX_RULE_LINE_BYTES: usize = 4096;

pub(crate) struct Entry { path: PathBuf, kind: EntryKind, depth: usize }
impl Entry {
    pub(crate) fn path(&self) -> &Path { &self.path }
    pub(crate) fn file_type(&self) -> Option<EntryKind> { Some(self.kind) }
    pub(crate) fn depth(&self) -> usize { self.depth }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WalkMode { Search, Inventory }

struct Frame { entries: Entries, ignore: Gitignore, gitignore: Gitignore, git_exclude: Gitignore, has_git: bool, depth: usize }

pub(crate) struct SearchWalk<F> {
    root: PathBuf, authority: PathAuthority, frames: Vec<Frame>, pending: Option<Entry>,
    inherited_git: bool, entries_seen: usize, rule_lines: usize, started: Instant,
    mode: WalkMode,
    before_directory: F,
}

impl<F: FnMut(&Path)> SearchWalk<F> {
    pub(crate) fn new(root: &Path, target: &Path, authority: PathAuthority, started: Instant, reasons: &mut BTreeSet<String>, before_directory: F) -> Result<Self, AppError> {
        Self::with_mode(root, target, authority, started, reasons, before_directory, WalkMode::Search)
    }

    /// Inventory includes hidden files and applies workspace Git rules without
    /// requiring a repository marker. Its root has already passed admission.
    pub(crate) fn inventory(root: &Path, started: Instant, reasons: &mut BTreeSet<String>, before_directory: F) -> Result<Self, AppError> {
        Self::with_mode(root, root, PathAuthority::Workspace(root.to_owned()), started, reasons, before_directory, WalkMode::Inventory)
    }

    fn with_mode(root: &Path, target: &Path, authority: PathAuthority, started: Instant, reasons: &mut BTreeSet<String>, before_directory: F, mode: WalkMode) -> Result<Self, AppError> {
        let metadata = workspace_read_dir::metadata(target, &authority)?
            .ok_or_else(|| AppError::Conflict("search target disappeared before traversal".into()))?;
        let kind = EntryKind::from_metadata(&metadata.metadata);
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            return Err(AppError::Conflict("search target changed to a link or unsupported entry".into()));
        }
        if mode == WalkMode::Inventory && !kind.is_dir() {
            return Err(AppError::BadRequest("workspace inventory requires a directory root".into()));
        }
        let mut inherited_git = false;
        let mut exhausted = false;
        if kind.is_dir() {
            let mut ancestor = target.parent();
            while let Some(path) = ancestor.filter(|path| path.starts_with(root)) {
                if started.elapsed() >= Duration::from_secs(5) { reasons.insert("scan_budget".into()); exhausted = true; break; }
                if has_git(path, &authority)? { inherited_git = true; break; }
                ancestor = path.parent();
            }
        }
        Ok(Self { root: root.to_owned(), authority, frames: Vec::new(), pending: (!exhausted).then(|| Entry { path: target.to_owned(), kind, depth: 0 }),
            inherited_git, entries_seen: 0, rule_lines: 0, started, mode, before_directory })
    }

    pub(crate) fn next(&mut self, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Option<Result<Entry, AppError>> {
        loop {
            let entry_budget = if self.mode == WalkMode::Inventory { 100_000 } else { 20_000 };
            if self.entries_seen >= entry_budget || self.started.elapsed() >= Duration::from_secs(5) {
                reasons.insert("scan_budget".into()); self.frames.clear(); self.pending = None; return None;
            }
            if let Some(entry) = self.pending.take() {
                if entry.kind.is_dir() && entry.depth < 64 {
                    match self.frame(&entry.path, entry.depth, source_bytes, reasons) {
                        Ok(frame) => self.frames.push(frame),
                        Err(error) => return Some(Err(error)),
                    }
                }
                return Some(Ok(entry));
            }
            let frame = self.frames.last_mut()?;
            let depth = frame.depth + 1;
            let entry = match frame.entries.next() {
                None => { self.frames.pop(); continue; }
                Some(Err(error)) => return Some(Err(AppError::BadRequest(format!("cannot enumerate search entry: {error}")))),
                Some(Ok(entry)) => entry,
            };
            self.entries_seen += 1;
            let path = entry.path();
            if path.strip_prefix(&self.root).ok().and_then(|path| path.components().next())
                .is_none_or(|part| crate::artifact_store::is_workspace_owner_component(part.as_os_str())) { continue; }
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(error) => return Some(Err(AppError::BadRequest(format!("cannot classify search entry: {error}")))),
            };
            if self.ignored(&path, kind.is_dir(), entry.is_hidden()) { continue; }
            self.pending = Some(Entry { path, kind, depth });
        }
    }

    fn ignored(&self, path: &Path, directory: bool, hidden: bool) -> bool {
        // .ignore outranks .gitignore even when the Git rule is nearer. Within
        // one class, the nearest matcher wins and a whitelist overrides hidden.
        for frame in self.frames.iter().rev() {
            let matched = frame.ignore.matched(path, directory);
            if !matched.is_none() { return matched.is_ignore(); }
        }
        if self.mode == WalkMode::Inventory || self.inherited_git || self.frames.iter().any(|frame| frame.has_git) {
            for frame in self.frames.iter().rev() {
                let matched = frame.gitignore.matched(path, directory);
                if !matched.is_none() { return matched.is_ignore(); }
                if frame.has_git { break; }
            }
            for frame in self.frames.iter().rev() {
                let matched = frame.git_exclude.matched(path, directory);
                if !matched.is_none() { return matched.is_ignore(); }
                if frame.has_git { break; }
            }
        }
        self.mode == WalkMode::Search && hidden
    }

    fn frame(&mut self, path: &Path, depth: usize, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Frame, AppError> {
        (self.before_directory)(path);
        // Retain the directory before inspecting markers or loading rules.
        let entries = workspace_read_dir::read_directory(path, &self.authority)?;
        let has_git = has_git(path, &self.authority)?;
        let ignore = self.rules(path, ".ignore", source_bytes, reasons)?;
        let gitignore = if self.mode == WalkMode::Inventory || has_git || self.inherited_git || self.frames.iter().any(|frame| frame.has_git) {
            self.rules(path, ".gitignore", source_bytes, reasons)?
        } else { Gitignore::empty() };
        let git_exclude = if self.mode == WalkMode::Inventory {
            self.git_exclude(path, source_bytes, reasons)?
        } else { Gitignore::empty() };
        Ok(Frame { entries, ignore, gitignore, git_exclude, has_git, depth })
    }

    fn rules(&mut self, directory: &Path, name: &str, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Gitignore, AppError> {
        let path = directory.join(name);
        self.rules_at(directory, &path, source_bytes, reasons)
    }

    fn rules_at(&mut self, directory: &Path, path: &Path, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Gitignore, AppError> {
        let Some((path, text)) = self.rule_source(path, source_bytes, reasons)? else { return Ok(Gitignore::empty()); };
        let mut builder = GitignoreBuilder::new(directory);
        for (index, line) in text.lines().enumerate() {
            if self.rule_lines >= MAX_RULE_LINES || line.len() > MAX_RULE_LINE_BYTES || self.started.elapsed() >= Duration::from_secs(5) {
                return Err(rule_failure("ignore_rule_budget", reasons));
            }
            self.rule_lines += 1;
            let line = if index == 0 { line.trim_start_matches('\u{feff}') } else { line };
            builder.add_line(Some(path.clone()), line).map_err(|_| rule_failure("invalid_ignore_pattern", reasons))?;
        }
        builder.build().map_err(|_| rule_failure("invalid_ignore_pattern", reasons))
    }

    fn rule_source(&self, path: &Path, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Option<(PathBuf, String)>, AppError> {
        let failure = |reason: &str, reasons: &mut BTreeSet<String>| {
            rule_failure(reason, reasons)
        };
        let metadata = workspace_read_dir::metadata(path, &self.authority)
            .map_err(|_| failure("unreadable_ignore_source", reasons))?;
        let Some(metadata) = metadata else { return Ok(None); };
        let kind = EntryKind::from_metadata(&metadata.metadata);
        if kind.is_symlink() || !kind.is_file() { return Err(failure("invalid_or_linked_ignore_source", reasons)); }
        let path = metadata.canonical;
        let remaining = MAX_SOURCE_BYTES.saturating_sub(*source_bytes);
        if remaining <= 1 { return Err(failure("source_byte_budget", reasons)); }
        let source = crate::agent_text_read::read_source_bytes_with_hooks(
            &path, &self.authority, MAX_FILE_BYTES.min(remaining - 1), source_bytes, || {}, || {},
        ).map_err(|_| failure("unreadable_changed_or_oversized_ignore_source", reasons))?;
        let Some((bytes, _, canonical)) = source else { return Err(failure("changed_ignore_source", reasons)); };
        if canonical != path { return Err(failure("changed_ignore_source", reasons)); }
        let text = String::from_utf8(bytes).map_err(|_| failure("invalid_ignore_source", reasons))?;
        Ok(Some((path, text)))
    }

    fn git_exclude(&mut self, directory: &Path, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Gitignore, AppError> {
        let marker = directory.join(".git");
        let Some(metadata) = workspace_read_dir::metadata(&marker, &self.authority)? else { return Ok(Gitignore::empty()); };
        let kind = EntryKind::from_metadata(&metadata.metadata);
        if kind.is_symlink() { return Err(rule_failure("linked_git_directory", reasons)); }
        let git_dir = if kind.is_dir() {
            metadata.canonical
        } else if kind.is_file() {
            let Some((_, text)) = self.rule_source(&marker, source_bytes, reasons)? else { return Err(rule_failure("changed_git_directory", reasons)); };
            let value = text.trim().strip_prefix("gitdir: ")
                .ok_or_else(|| rule_failure("invalid_git_directory", reasons))?;
            self.git_directory_target(directory, value, reasons)?
        } else { return Err(rule_failure("invalid_git_directory", reasons)); };
        let common = match self.rule_source(&git_dir.join("commondir"), source_bytes, reasons)? {
            Some((_, text)) => self.git_directory_target(&git_dir, text.trim(), reasons)?,
            None => git_dir,
        };
        self.rules_at(directory, &common.join("info/exclude"), source_bytes, reasons)
    }

    fn git_directory_target(&self, directory: &Path, value: &str, reasons: &mut BTreeSet<String>) -> Result<PathBuf, AppError> {
        if value.is_empty() || value.chars().any(char::is_control) {
            return Err(rule_failure("invalid_git_directory", reasons));
        }
        let target = directory.join(value).canonicalize()
            .map_err(|_| rule_failure("unreadable_git_directory", reasons))?;
        crate::path_safety::reject_workspace_owner_canonical_path(&self.root, &target)?;
        Ok(target)
    }
}

fn rule_failure(reason: &str, reasons: &mut BTreeSet<String>) -> AppError {
    reasons.insert(reason.into());
    AppError::Conflict("Workspace ignore rules could not be fully verified; narrow the directory before retry".into())
}

fn has_git(directory: &Path, authority: &PathAuthority) -> Result<bool, AppError> {
    // A worktree's .git file is a marker; its gitdir destination is not needed
    // for search. Never follow it or probe above the bound workspace root.
    Ok(workspace_read_dir::metadata(&directory.join(".git"), authority)?.is_some()
        || workspace_read_dir::metadata(&directory.join(".jj"), authority)?.is_some())
}
