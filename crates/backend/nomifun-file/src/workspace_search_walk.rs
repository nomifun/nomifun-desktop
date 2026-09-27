//! Bounded search traversal. Ignore files are data read through workspace
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

struct Frame { entries: Entries, ignore: Gitignore, gitignore: Gitignore, has_git: bool, depth: usize }

pub(crate) struct SearchWalk<F> {
    root: PathBuf, authority: PathAuthority, frames: Vec<Frame>, pending: Option<Entry>,
    inherited_git: bool, entries_seen: usize, rule_lines: usize, started: Instant,
    before_directory: F,
}

impl<F: FnMut(&Path)> SearchWalk<F> {
    pub(crate) fn new(root: &Path, target: &Path, authority: PathAuthority, started: Instant, reasons: &mut BTreeSet<String>, before_directory: F) -> Result<Self, AppError> {
        let metadata = workspace_read_dir::metadata(target, &authority)?
            .ok_or_else(|| AppError::Conflict("search target disappeared before traversal".into()))?;
        let kind = EntryKind::from_metadata(&metadata.metadata);
        if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
            return Err(AppError::Conflict("search target changed to a link or unsupported entry".into()));
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
            inherited_git, entries_seen: 0, rule_lines: 0, started, before_directory })
    }

    pub(crate) fn next(&mut self, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Option<Result<Entry, AppError>> {
        loop {
            if self.entries_seen >= 20_000 || self.started.elapsed() >= Duration::from_secs(5) {
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
        if self.inherited_git || self.frames.iter().any(|frame| frame.has_git) {
            for frame in self.frames.iter().rev() {
                let matched = frame.gitignore.matched(path, directory);
                if !matched.is_none() { return matched.is_ignore(); }
                if frame.has_git { break; }
            }
        }
        hidden
    }

    fn frame(&mut self, path: &Path, depth: usize, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Frame, AppError> {
        (self.before_directory)(path);
        // Retain the directory before inspecting markers or loading rules.
        let entries = workspace_read_dir::read_directory(path, &self.authority)?;
        let has_git = has_git(path, &self.authority)?;
        let ignore = self.rules(path, ".ignore", source_bytes, reasons)?;
        let gitignore = if has_git || self.inherited_git || self.frames.iter().any(|frame| frame.has_git) {
            self.rules(path, ".gitignore", source_bytes, reasons)?
        } else { Gitignore::empty() };
        Ok(Frame { entries, ignore, gitignore, has_git, depth })
    }

    fn rules(&mut self, directory: &Path, name: &str, source_bytes: &mut usize, reasons: &mut BTreeSet<String>) -> Result<Gitignore, AppError> {
        let path = directory.join(name);
        let failure = |reason: &str, reasons: &mut BTreeSet<String>| {
            reasons.insert(reason.into());
            AppError::Conflict("Search ignore rules could not be fully verified; narrow the search before retry".into())
        };
        let metadata = workspace_read_dir::metadata(&path, &self.authority)
            .map_err(|_| failure("unreadable_ignore_source", reasons))?;
        let Some(metadata) = metadata else { return Ok(Gitignore::empty()); };
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
        let text = std::str::from_utf8(&bytes).map_err(|_| failure("invalid_ignore_source", reasons))?;
        let mut builder = GitignoreBuilder::new(directory);
        for (index, line) in text.lines().enumerate() {
            if self.rule_lines >= MAX_RULE_LINES || line.len() > MAX_RULE_LINE_BYTES || self.started.elapsed() >= Duration::from_secs(5) {
                return Err(failure("ignore_rule_budget", reasons));
            }
            self.rule_lines += 1;
            let line = if index == 0 { line.trim_start_matches('\u{feff}') } else { line };
            builder.add_line(Some(path.clone()), line).map_err(|_| failure("invalid_ignore_pattern", reasons))?;
        }
        builder.build().map_err(|_| failure("invalid_ignore_pattern", reasons))
    }
}

fn has_git(directory: &Path, authority: &PathAuthority) -> Result<bool, AppError> {
    // A worktree's .git file is a marker; its gitdir destination is not needed
    // for search. Never follow it or probe above the bound workspace root.
    Ok(workspace_read_dir::metadata(&directory.join(".git"), authority)?.is_some()
        || workspace_read_dir::metadata(&directory.join(".jj"), authority)?.is_some())
}
