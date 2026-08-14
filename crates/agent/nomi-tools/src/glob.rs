use std::collections::{HashMap, VecDeque};
use std::fs::Metadata;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::sync::{
    Mutex as AsyncMutex, OwnedMutexGuard, OwnedSemaphorePermit, Semaphore, oneshot,
};

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::Tool;

const MAX_RESULTS: usize = 100;
const MAX_SCANNED_ENTRIES: usize = 100_000;
const MAX_TRAVERSAL_DEPTH: usize = 64;
const MAX_PENDING_DIRECTORIES: usize = 4_096;
const MAX_PENDING_PATH_BYTES: usize = 4 * 1024 * 1024;
const MAX_PATTERN_BYTES: usize = 4_096;
const MAX_PATH_BYTES: usize = 32_768;
const SCAN_DEADLINE: Duration = Duration::from_secs(8);
const SCAN_CAPACITY_TIMEOUT: Duration = Duration::from_secs(2);
const SCAN_HARD_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONCURRENT_SCANS: usize = 4;

static SCAN_LIMIT: OnceLock<Arc<Semaphore>> = OnceLock::new();
static ROOT_SCAN_LOCKS: OnceLock<StdMutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> =
    OnceLock::new();

#[derive(Clone, Copy)]
struct ScanLimits {
    max_entries: usize,
    max_depth: usize,
    max_pending_dirs: usize,
    max_pending_path_bytes: usize,
    deadline: Duration,
}

impl Default for ScanLimits {
    fn default() -> Self {
        Self {
            max_entries: MAX_SCANNED_ENTRIES,
            max_depth: MAX_TRAVERSAL_DEPTH,
            max_pending_dirs: MAX_PENDING_DIRECTORIES,
            max_pending_path_bytes: MAX_PENDING_PATH_BYTES,
            deadline: SCAN_DEADLINE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanStop {
    Cancelled,
    Deadline,
    EntryLimit,
    DepthLimit,
    PendingLimit,
    IoErrors,
}

struct ScanReport {
    files: Vec<(SystemTime, String)>,
    scanned_entries: usize,
    more_matches: bool,
    stop: Option<ScanStop>,
    io_errors: usize,
}

struct ScanPlan {
    traversal_root: PathBuf,
    display_root: PathBuf,
    matcher: globset::GlobMatcher,
    pattern_depth: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum DirectoryIdentity {
    #[cfg(unix)]
    Native(u64, u64),
    Canonical(PathBuf),
}

struct DirectoryAncestry {
    identity: DirectoryIdentity,
    parent: Option<Arc<DirectoryAncestry>>,
}

impl DirectoryAncestry {
    fn contains(&self, identity: &DirectoryIdentity) -> bool {
        let mut current = Some(self);
        while let Some(ancestor) = current {
            if ancestor.identity == *identity {
                return true;
            }
            current = ancestor.parent.as_deref();
        }
        false
    }
}

struct PendingDirectory {
    path: PathBuf,
    depth: usize,
    ancestry: Arc<DirectoryAncestry>,
}

/// Cancelling/dropping the async tool future must become visible to the
/// blocking walker. Filesystem syscalls cannot be pre-empted portably, so the
/// walker checks this flag between operations.
struct CancelScanOnDrop(Arc<AtomicBool>);

impl CancelScanOnDrop {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    fn token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }

    fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Drop for CancelScanOnDrop {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn component_has_glob_meta(component: Component<'_>) -> bool {
    component
        .as_os_str()
        .to_string_lossy()
        .bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'['))
}

fn traversal_shape(pattern: &Path) -> (PathBuf, Option<usize>) {
    let mut literal_prefix = PathBuf::new();
    let mut pattern_components = 0usize;
    let mut saw_meta = false;
    let mut recursive = false;

    for component in pattern.components() {
        if !saw_meta && !component_has_glob_meta(component) {
            literal_prefix.push(component.as_os_str());
            continue;
        }

        saw_meta = true;
        pattern_components = pattern_components.saturating_add(1);
        if component.as_os_str() == "**" {
            recursive = true;
        }
    }

    let depth = if recursive {
        None
    } else {
        Some(pattern_components)
    };
    (literal_prefix, depth)
}

#[cfg(windows)]
fn ends_with_path_separator(value: &str) -> bool {
    value.ends_with(['/', '\\'])
}

#[cfg(not(windows))]
fn ends_with_path_separator(value: &str) -> bool {
    value.ends_with('/')
}

fn relative_full_pattern(root: &Path, pattern: &str) -> String {
    // The root is data, not a model-supplied pattern. Escaping it prevents a
    // real directory such as `[workspace]` from being interpreted as glob
    // syntax when the relative pattern is appended.
    let mut full = globset::escape(root.to_string_lossy().as_ref());
    if !ends_with_path_separator(&full) {
        full.push(std::path::MAIN_SEPARATOR);
    }
    full.push_str(&literalize_braces(pattern));
    full
}

fn literalize_braces(pattern: &str) -> String {
    // glob 0.3 treated braces literally. globset adds alternation syntax, so
    // escape braces outside an existing character class to retain the public
    // contract and keep traversal-depth analysis exact.
    let mut output = String::with_capacity(pattern.len());
    let mut in_class = false;
    for character in pattern.chars() {
        match character {
            '[' => {
                in_class = true;
                output.push(character);
            }
            ']' => {
                in_class = false;
                output.push(character);
            }
            '{' | '}' if !in_class => {
                output.push('[');
                output.push(character);
                output.push(']');
            }
            _ => output.push(character),
        }
    }
    output
}

fn normalized_relative_pattern(pattern: &str) -> Result<PathBuf, String> {
    if pattern.is_empty() {
        return Err("Glob pattern must not be empty".to_string());
    }
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(format!(
            "Glob pattern exceeds the {MAX_PATTERN_BYTES} byte safety limit"
        ));
    }

    let mut normalized = PathBuf::new();
    for component in Path::new(pattern).components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(
                    "Glob pattern must be relative and cannot contain parent path components; put the search root in `path`"
                        .to_string(),
                );
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err("Glob pattern must identify a file name or pattern".to_string());
    }
    Ok(normalized)
}

impl ScanPlan {
    fn new(root: PathBuf, pattern: &str) -> Result<Self, String> {
        let pattern_path = normalized_relative_pattern(pattern)?;
        let normalized_pattern = pattern_path.to_string_lossy();
        let matcher_text = relative_full_pattern(&root, &normalized_pattern);
        let mut matcher_builder = globset::GlobBuilder::new(&matcher_text);
        matcher_builder
            .literal_separator(true)
            // On Unix a backslash is a valid filename character, while on
            // Windows it remains a path separator. Disabling glob escaping
            // preserves that native path behavior and matches glob 0.3.
            .backslash_escape(false);
        let matcher = matcher_builder
            .build()
            .map_err(|error| format!("Invalid glob pattern: {error}"))?
            .compile_matcher();
        let (literal_prefix, pattern_depth) = traversal_shape(&pattern_path);
        let traversal_root = root.join(literal_prefix);

        Ok(Self {
            traversal_root,
            display_root: root,
            matcher,
            pattern_depth,
        })
    }
}

#[cfg(unix)]
fn directory_identity(
    path: &Path,
    metadata: &Metadata,
) -> std::io::Result<DirectoryIdentity> {
    use std::os::unix::fs::MetadataExt;
    let inode = metadata.ino();
    if inode == 0 {
        std::fs::canonicalize(path).map(DirectoryIdentity::Canonical)
    } else {
        Ok(DirectoryIdentity::Native(metadata.dev(), inode))
    }
}

#[cfg(windows)]
fn directory_identity(path: &Path, _metadata: &Metadata) -> std::io::Result<DirectoryIdentity> {
    // Stable Rust does not yet expose Windows volume/file IDs. Canonical paths
    // resolve directory symlinks and NTFS junctions without retaining one OS
    // handle per visited directory, which keeps the scan's descriptor use
    // constant. The depth/entry/deadline budgets remain the final backstop for
    // unusual aliasing that canonicalization cannot collapse.
    std::fs::canonicalize(path).map(DirectoryIdentity::Canonical)
}

#[cfg(not(any(unix, windows)))]
fn directory_identity(
    path: &Path,
    _metadata: &Metadata,
) -> std::io::Result<DirectoryIdentity> {
    std::fs::canonicalize(path).map(DirectoryIdentity::Canonical)
}

fn root_scan_lock(root: &Path) -> Arc<AsyncMutex<()>> {
    let locks = ROOT_SCAN_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(root).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(root.to_path_buf(), Arc::downgrade(&lock));
    lock
}

async fn acquire_scan_capacity(
    root: &Path,
    budget: Duration,
) -> Option<(OwnedSemaphorePermit, OwnedMutexGuard<()>)> {
    let limit = Arc::clone(
        SCAN_LIMIT.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_SCANS))),
    );
    let root_lock = root_scan_lock(root);
    tokio::time::timeout(budget, async move {
        // Serialize the same traversal-root key first so a timed-out NFS or
        // FUSE syscall cannot be multiplied by immediate retries. Different
        // roots can still make progress up to the small global cap.
        let root_guard = root_lock.lock_owned().await;
        let permit = limit
            .acquire_owned()
            .await
            .expect("Glob scan semaphore is never closed");
        (permit, root_guard)
    })
    .await
    .ok()
}

fn scan_stop_reason(
    started: Instant,
    limits: ScanLimits,
    cancelled: &AtomicBool,
    scanned_entries: usize,
) -> Option<ScanStop> {
    if cancelled.load(Ordering::Acquire) {
        Some(ScanStop::Cancelled)
    } else if started.elapsed() >= limits.deadline {
        Some(ScanStop::Deadline)
    } else if scanned_entries >= limits.max_entries {
        Some(ScanStop::EntryLimit)
    } else {
        None
    }
}

fn followed_metadata(entry: &std::fs::DirEntry) -> std::io::Result<Option<Metadata>> {
    let file_type = entry.file_type()?;
    if file_type.is_symlink() {
        // `DirEntry::metadata` does not consistently follow links across the
        // supported platforms. `fs::metadata` deliberately does, preserving
        // workspace skill symlinks and Windows junctions.
        // A missing target is a local broken-link miss. Permission, mount I/O,
        // and other failures must propagate so the caller cannot report a
        // falsely complete "No files matched" result.
        match std::fs::metadata(entry.path()) {
            Ok(metadata) => Ok(Some(metadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    } else {
        entry.metadata().map(Some)
    }
}

fn record_io_error(error: &std::io::Error, io_errors: &mut usize) {
    // A literal prefix that does not exist is an ordinary no-match result, and
    // files may disappear between read_dir and metadata. Other errors (notably
    // permissions and mount I/O failures) make the result incomplete.
    if error.kind() != std::io::ErrorKind::NotFound {
        *io_errors = (*io_errors).saturating_add(1);
    }
}

fn record_matching_file(
    plan: &ScanPlan,
    path: &Path,
    metadata: &Metadata,
    files: &mut Vec<(SystemTime, String)>,
) -> bool {
    if !metadata.is_file() || !plan.matcher.is_match(path) {
        return false;
    }

    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let display_path = path
        .strip_prefix(&plan.display_root)
        .unwrap_or(path)
        .display()
        .to_string();
    files.push((modified, display_path));
    files.len() > MAX_RESULTS
}

fn scan_files(plan: ScanPlan, limits: ScanLimits, cancelled: &AtomicBool) -> ScanReport {
    let started = Instant::now();
    let mut files = Vec::new();
    let mut scanned_entries = 0usize;
    let mut stop = None;
    let mut more_matches = false;
    let mut depth_limit_reached = false;
    let mut io_errors = 0usize;

    let requested_depth = plan.pattern_depth.unwrap_or(limits.max_depth);
    let depth_was_capped = plan.pattern_depth.is_none() || requested_depth > limits.max_depth;
    let traversal_depth = requested_depth.min(limits.max_depth);
    let mut pending = VecDeque::new();
    let mut pending_path_bytes = 0usize;

    if let Some(reason) = scan_stop_reason(started, limits, cancelled, scanned_entries) {
        stop = Some(reason);
    } else {
        match std::fs::metadata(&plan.traversal_root) {
            Ok(root_metadata) => {
                scanned_entries += 1;
                if root_metadata.is_file() {
                    more_matches = record_matching_file(
                        &plan,
                        &plan.traversal_root,
                        &root_metadata,
                        &mut files,
                    );
                } else if root_metadata.is_dir() {
                    if traversal_depth == 0 {
                        depth_limit_reached = depth_was_capped;
                    } else if limits.max_pending_dirs == 0
                        || plan.traversal_root.as_os_str().len() > limits.max_pending_path_bytes
                    {
                        stop = Some(ScanStop::PendingLimit);
                    } else {
                        match directory_identity(&plan.traversal_root, &root_metadata) {
                            Ok(identity) => {
                                pending_path_bytes = plan.traversal_root.as_os_str().len();
                                pending.push_back(PendingDirectory {
                                    path: plan.traversal_root.clone(),
                                    depth: 0,
                                    ancestry: Arc::new(DirectoryAncestry {
                                        identity,
                                        parent: None,
                                    }),
                                });
                            }
                            Err(error) => record_io_error(&error, &mut io_errors),
                        }
                    }
                }
            }
            Err(error) => record_io_error(&error, &mut io_errors),
        }
    }

    // Keep only one ReadDir open at a time. Unlike WalkDir's descriptor
    // fallback, this never collects an unbounded remainder of an ancestor
    // directory inside iterator.next(); every entry is pulled only after the
    // cancellation, deadline, and entry-budget checks below.
    'scan: while stop.is_none() && !more_matches {
        let Some(directory) = pending.pop_front() else {
            break;
        };
        pending_path_bytes = pending_path_bytes.saturating_sub(directory.path.as_os_str().len());
        if let Some(reason) = scan_stop_reason(started, limits, cancelled, scanned_entries) {
            stop = Some(reason);
            break;
        }
        let mut entries = match std::fs::read_dir(&directory.path) {
            Ok(entries) => entries,
            Err(error) => {
                record_io_error(&error, &mut io_errors);
                continue;
            }
        };

        loop {
            if let Some(reason) = scan_stop_reason(started, limits, cancelled, scanned_entries) {
                stop = Some(reason);
                break 'scan;
            }
            let Some(entry) = entries.next() else {
                break;
            };
            scanned_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    record_io_error(&error, &mut io_errors);
                    continue;
                }
            };
            let path = entry.path();
            let metadata = match followed_metadata(&entry) {
                Ok(Some(metadata)) => metadata,
                Ok(None) => continue,
                Err(error) => {
                    record_io_error(&error, &mut io_errors);
                    continue;
                }
            };

            if metadata.is_dir() {
                let child_depth = directory.depth.saturating_add(1);
                if child_depth >= traversal_depth {
                    if depth_was_capped && child_depth >= limits.max_depth {
                        depth_limit_reached = true;
                    }
                    continue;
                }

                let path_bytes = path.as_os_str().len();
                if pending.len() >= limits.max_pending_dirs
                    || pending_path_bytes.saturating_add(path_bytes)
                        > limits.max_pending_path_bytes
                {
                    stop = Some(ScanStop::PendingLimit);
                    break 'scan;
                }

                let identity = match directory_identity(&path, &metadata) {
                    Ok(identity) => identity,
                    Err(error) => {
                        record_io_error(&error, &mut io_errors);
                        continue;
                    }
                };
                // Preserve legitimate skill aliases, but never re-enter a
                // physical ancestor through a symlink, junction, bind alias,
                // or other same-file path.
                if directory.ancestry.contains(&identity) {
                    continue;
                }
                pending_path_bytes = pending_path_bytes.saturating_add(path_bytes);
                pending.push_back(PendingDirectory {
                    path,
                    depth: child_depth,
                    ancestry: Arc::new(DirectoryAncestry {
                        identity,
                        parent: Some(Arc::clone(&directory.ancestry)),
                    }),
                });
                continue;
            }

            if record_matching_file(&plan, &path, &metadata, &mut files) {
                // One look-ahead match is enough to prove truncation. The old
                // implementation traversed the whole tree after result 100
                // solely to compute an exact total.
                more_matches = true;
                break 'scan;
            }
        }
    }

    files.sort_by_key(|file| std::cmp::Reverse(file.0));
    files.truncate(MAX_RESULTS);
    if stop.is_none() && depth_limit_reached {
        stop = Some(ScanStop::DepthLimit);
    } else if stop.is_none() && io_errors > 0 {
        stop = Some(ScanStop::IoErrors);
    }
    ScanReport {
        files,
        scanned_entries,
        more_matches,
        stop,
        io_errors,
    }
}

fn render_scan_report(report: ScanReport) -> ToolResult {
    let paths = report
        .files
        .iter()
        .map(|(_, path)| path.as_str())
        .collect::<Vec<_>>();

    if let Some(reason) = report.stop {
        let reason = match reason {
            ScanStop::Cancelled => "the tool call was cancelled".to_string(),
            ScanStop::Deadline => format!("the {} second scan deadline was reached", SCAN_DEADLINE.as_secs()),
            ScanStop::EntryLimit => format!("the {MAX_SCANNED_ENTRIES} entry scan limit was reached"),
            ScanStop::DepthLimit => format!("the {MAX_TRAVERSAL_DEPTH} level traversal limit was reached"),
            ScanStop::PendingLimit => format!(
                "the pending-directory memory budget ({MAX_PENDING_DIRECTORIES} directories / {MAX_PENDING_PATH_BYTES} path bytes) was reached"
            ),
            ScanStop::IoErrors => format!(
                "{} filesystem entries or directories could not be read",
                report.io_errors
            ),
        };
        let mut content = format!(
            "Glob search stopped after scanning {} entries because {reason}. Results are incomplete; narrow `path` or `pattern` and retry.",
            report.scanned_entries
        );
        if !paths.is_empty() {
            content.push_str("\nPartial matches:\n");
            content.push_str(&paths.join("\n"));
        }
        return ToolResult::error(content);
    }

    if paths.is_empty() {
        return ToolResult::text("No files matched the pattern");
    }

    let mut output = paths.into_iter().map(str::to_owned).collect::<Vec<_>>();
    if report.more_matches {
        output.push(format!(
            "... [showing the first {MAX_RESULTS} matches; more matches exist — refine the pattern or path]"
        ));
    }
    ToolResult::text(output.join("\n"))
}

pub struct GlobTool {
    cwd: PathBuf,
}

impl GlobTool {
    pub fn new(cwd: PathBuf) -> Self {
        Self { cwd }
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "Glob"
    }

    fn description(&self) -> &str {
        "Bounded OS-agnostic file pattern matching tool.\n\n\
         - Supports glob patterns like \"**/*.rs\" or \"src/**/*.ts\".\n\
         - Patterns are relative; put an absolute or parent search root in the path parameter.\n\
         - Returns matching file paths sorted by modification time within the bounded scan.\n\
         - Returns at most 100 results. Only returns files, not directories.\n\
         - The path parameter defaults to the current working directory.\n\
         - Broad searches have time, depth, and entry limits; narrow the path or pattern if a scan reports an incomplete result.\n\
         - Use this OS-agnostic tool to list files in the current directory or workspace on every operating system: \"*\" lists top-level files and \"**/*\" lists files recursively.\n\
         - Use this tool when you need to find files by name or extension patterns, and prefer it over Bash for directory file listings."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": MAX_PATTERN_BYTES,
                    "description": "Glob pattern, e.g. \"**/*.rs\""
                },
                "path": {
                    "type": "string",
                    "maxLength": MAX_PATH_BYTES,
                    "description": "Root directory (default: cwd)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        // A scan is read-only, but concurrent broad walks over the same mount
        // amplify I/O stalls and used to starve every Tokio worker. Calls in a
        // model batch therefore remain serial; a small process-wide semaphore
        // still bounds scans originating from independent conversations.
        false
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let Some(pattern) = input["pattern"].as_str() else {
            return ToolResult {
                content: "Missing required parameter: pattern".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        };

        let root = input["path"].as_str().unwrap_or(".");
        if root.len() > MAX_PATH_BYTES {
            return ToolResult::error(format!(
                "Glob path exceeds the {MAX_PATH_BYTES} byte safety limit"
            ));
        }
        let root_path = if Path::new(root).is_relative() {
            self.cwd.join(root)
        } else {
            PathBuf::from(root)
        };

        let plan = match ScanPlan::new(root_path.clone(), pattern) {
            Ok(plan) => plan,
            Err(error) => return ToolResult::error(error),
        };

        tracing::debug!(
            cwd = %self.cwd.display(),
            resolved_root = %root_path.display(),
            traversal_root = %plan.traversal_root.display(),
            pattern = %pattern,
            "GlobTool bounded scan starting"
        );

        let overall_started = Instant::now();
        let scan_lock_root = plan.traversal_root.clone();
        let Some((permit, root_guard)) =
            acquire_scan_capacity(&scan_lock_root, SCAN_CAPACITY_TIMEOUT).await
        else {
            return ToolResult::error(format!(
                "Glob search could not start within {} seconds because the filesystem scan capacity for this path is busy. A previous scan may still be waiting on slow filesystem I/O; narrow the path and retry later.",
                SCAN_CAPACITY_TIMEOUT.as_secs()
            ));
        };
        let remaining = SCAN_HARD_TIMEOUT.saturating_sub(overall_started.elapsed());
        if remaining.is_zero() {
            return ToolResult::error(format!(
                "Glob search could not start within the {} second safety budget; narrow the path or pattern and retry.",
                SCAN_HARD_TIMEOUT.as_secs()
            ));
        }

        let cancellation = CancelScanOnDrop::new();
        let worker_cancellation = cancellation.token();
        let limits = ScanLimits {
            deadline: SCAN_DEADLINE.min(remaining),
            ..ScanLimits::default()
        };
        let (result_tx, mut result_rx) = oneshot::channel();
        let worker = std::thread::Builder::new()
            .name("nomifun-glob-scan".to_string())
            .spawn(move || {
                // Keep both guards in the blocking worker. If a platform syscall
                // itself gets stuck after the async caller times out, retries for
                // this root cannot spawn an unbounded number of leaked workers.
                let _permit = permit;
                let _root_guard = root_guard;
                let report = scan_files(plan, limits, worker_cancellation.as_ref());
                let _ = result_tx.send(report);
            });
        if let Err(error) = worker {
            return ToolResult::error(format!(
                "Glob search worker could not be started: {error}"
            ));
        }

        match tokio::time::timeout(remaining, &mut result_rx).await {
            Ok(Ok(report)) => render_scan_report(report),
            Ok(Err(_)) => ToolResult::error(
                "Glob search worker stopped before producing a result; the conversation can continue."
                    .to_string(),
            ),
            Err(_) => {
                cancellation.cancel();
                tracing::warn!(
                    root = %root_path.display(),
                    pattern = %pattern,
                    timeout_secs = SCAN_HARD_TIMEOUT.as_secs(),
                    "GlobTool scan hit hard timeout"
                );
                ToolResult::error(format!(
                    "Glob search timed out after {} seconds while waiting on filesystem I/O. The conversation can continue; narrow `path` or `pattern` before retrying.",
                    SCAN_HARD_TIMEOUT.as_secs()
                ))
            }
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }

    fn describe(&self, input: &Value) -> String {
        let pattern = input.get("pattern").and_then(|v| v.as_str()).unwrap_or("*");
        format!("Search for {}", pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    use nomi_types::tool::ToolResult;

    async fn run_glob(pattern: &str, path: &str) -> ToolResult {
        let tool = GlobTool::new(PathBuf::from(path));
        let input = json!({ "pattern": pattern, "path": path });
        tool.execute(input).await
    }

    #[tokio::test]
    async fn glob_stops_after_one_lookahead_match_instead_of_counting_the_tree() {
        let dir = tempdir().unwrap();
        let base = dir.path();
        let n = super::MAX_RESULTS + 5;
        for i in 0..n {
            fs::write(base.join(format!("f{i}.rs")), "x").unwrap();
        }
        let result = run_glob("*.rs", base.to_str().unwrap()).await;
        assert!(!result.is_error, "glob should succeed: {}", result.content);
        assert!(
            result.content.contains("more matches exist"),
            "must announce bounded truncation, got: {}",
            result.content
        );
        assert_eq!(
            result.content.lines().count(),
            MAX_RESULTS + 1,
            "100 matches plus one truncation notice are returned"
        );
    }

    #[tokio::test]
    async fn test_glob_matches_pattern() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        fs::write(base.join("main.rs"), "fn main() {}").unwrap();
        fs::write(base.join("lib.rs"), "pub mod lib;").unwrap();
        fs::write(base.join("notes.txt"), "some notes").unwrap();
        fs::write(base.join("readme.md"), "# Readme").unwrap();

        let result = run_glob("*.rs", base.to_str().unwrap()).await;

        assert!(!result.is_error, "glob should succeed");
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines.len(), 2, "should match exactly 2 .rs files");
        for line in &lines {
            assert!(
                line.ends_with(".rs"),
                "each match should be a .rs file, got: {}",
                line
            );
        }
        assert!(
            !result.content.contains("notes.txt"),
            "should not include .txt files"
        );
        assert!(
            !result.content.contains("readme.md"),
            "should not include .md files"
        );
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        fs::write(base.join("main.rs"), "fn main() {}").unwrap();
        fs::write(base.join("lib.rs"), "pub mod lib;").unwrap();

        let result = run_glob("*.xyz", base.to_str().unwrap()).await;

        assert!(!result.is_error, "no-match glob should not be an error");
        assert_eq!(result.content, "No files matched the pattern");
    }

    #[tokio::test]
    async fn missing_literal_prefix_is_an_ordinary_no_match() {
        let dir = tempdir().unwrap();
        let result = run_glob("missing/*.rs", dir.path().to_str().unwrap()).await;

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(result.content, "No files matched the pattern");
    }

    #[tokio::test]
    async fn test_glob_with_limit() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        for i in 0..5 {
            fs::write(
                base.join(format!("file_{}.txt", i)),
                format!("content {}", i),
            )
            .unwrap();
        }

        let result = run_glob("*.txt", base.to_str().unwrap()).await;

        assert!(!result.is_error, "glob should succeed");
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines.len(), 5, "all 5 files should be returned");
    }

    #[tokio::test]
    async fn test_glob_recursive() {
        let dir = tempdir().unwrap();
        let base = dir.path();

        // Create nested directory structure
        let sub_a = base.join("a");
        let sub_b = base.join("a").join("b");
        fs::create_dir_all(&sub_b).unwrap();

        fs::write(base.join("root.txt"), "root level").unwrap();
        fs::write(sub_a.join("mid.txt"), "middle level").unwrap();
        fs::write(sub_b.join("deep.txt"), "deep level").unwrap();
        // Non-matching file
        fs::write(sub_a.join("skip.rs"), "not a txt").unwrap();

        let result = run_glob("**/*.txt", base.to_str().unwrap()).await;

        assert!(!result.is_error, "recursive glob should succeed");
        let lines: Vec<&str> = result.content.lines().collect();
        assert_eq!(lines.len(), 3, "should find 3 .txt files across all levels");
        assert!(
            result.content.contains("root.txt"),
            "should include root-level file"
        );
        assert!(
            result.content.contains("mid.txt"),
            "should include mid-level file"
        );
        assert!(
            result.content.contains("deep.txt"),
            "should include deep-level file"
        );
        assert!(
            !result.content.contains("skip.rs"),
            "should not include .rs files"
        );
    }

    #[tokio::test]
    async fn execute_uses_cwd_for_relative_path() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("marker.txt"), "hello").unwrap();

        let tool = GlobTool::new(tmp.path().to_path_buf());
        let input = json!({"pattern": "marker.txt"});
        let result = tool.execute(input).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(
            result.content.contains("marker.txt"),
            "should find marker.txt, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn literal_glob_characters_in_root_are_not_treated_as_pattern_syntax() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("[workspace]{literal}");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("marker.txt"), "hello").unwrap();

        let result = run_glob("*.txt", root.to_str().unwrap()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(result.content, "marker.txt");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_backslash_in_root_is_matched_literally() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join(r"workspace\literal");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("marker.txt"), "hello").unwrap();

        let result = run_glob("*.txt", root.to_str().unwrap()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(result.content, "marker.txt");
    }

    #[tokio::test]
    async fn braces_retain_the_old_literal_filename_semantics() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("name{literal}.txt"), "hello").unwrap();

        let result = run_glob("name{literal}.txt", tmp.path().to_str().unwrap()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(result.content, "name{literal}.txt");
    }

    #[tokio::test]
    async fn absolute_pattern_is_rejected_in_favour_of_the_path_parameter() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("absolute.txt"), "hello").unwrap();
        let pattern = tmp.path().join("*.txt").to_string_lossy().into_owned();

        let result = run_glob(&pattern, tmp.path().to_str().unwrap()).await;
        assert!(result.is_error);
        assert!(result.content.contains("relative"));
        assert!(result.content.contains("`path`"));
    }

    #[tokio::test]
    async fn parent_pattern_is_rejected_in_favour_of_the_path_parameter() {
        let tmp = tempdir().unwrap();
        let result = run_glob("../*.txt", tmp.path().to_str().unwrap()).await;

        assert!(result.is_error);
        assert!(result.content.contains("parent path components"));
    }

    #[tokio::test]
    async fn current_directory_components_are_normalized_for_matching() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("marker.txt"), "hello").unwrap();

        let result = run_glob("./*.txt", tmp.path().to_str().unwrap()).await;
        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert_eq!(result.content, "marker.txt");
    }

    #[test]
    fn entry_budget_stops_a_zero_match_walk() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        fs::write(tmp.path().join("a/b/c/marker.txt"), "hello").unwrap();
        let plan = ScanPlan::new(tmp.path().to_path_buf(), "**/*.missing").unwrap();
        let cancelled = AtomicBool::new(false);
        let report = scan_files(
            plan,
            ScanLimits {
                max_entries: 2,
                deadline: Duration::from_secs(1),
                ..ScanLimits::default()
            },
            &cancelled,
        );

        assert_eq!(report.stop, Some(ScanStop::EntryLimit));
        assert_eq!(report.scanned_entries, 2);
        assert!(report.files.is_empty());
    }

    #[test]
    fn cancellation_is_observed_before_the_next_filesystem_entry() {
        let tmp = tempdir().unwrap();
        fs::write(tmp.path().join("marker.txt"), "hello").unwrap();
        let plan = ScanPlan::new(tmp.path().to_path_buf(), "**/*").unwrap();
        let cancelled = AtomicBool::new(true);
        let report = scan_files(plan, ScanLimits::default(), &cancelled);

        assert_eq!(report.stop, Some(ScanStop::Cancelled));
        assert_eq!(report.scanned_entries, 0);
    }

    #[test]
    fn dropping_async_scan_owner_notifies_a_pending_worker() {
        let cancellation = CancelScanOnDrop::new();
        let worker_cancellation = cancellation.token();
        let (observed_tx, observed_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            while !worker_cancellation.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            observed_tx.send(()).unwrap();
        });

        drop(cancellation);
        observed_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("dropping the async owner must cancel its detached worker");
        worker.join().unwrap();
    }

    #[test]
    fn recursive_depth_budget_is_reported_as_incomplete() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("a/b/marker.txt"), "hello").unwrap();
        let plan = ScanPlan::new(tmp.path().to_path_buf(), "**/*.txt").unwrap();
        let cancelled = AtomicBool::new(false);
        let report = scan_files(
            plan,
            ScanLimits {
                max_depth: 1,
                deadline: Duration::from_secs(1),
                ..ScanLimits::default()
            },
            &cancelled,
        );

        assert_eq!(report.stop, Some(ScanStop::DepthLimit));
        assert!(report.files.is_empty());
    }

    #[test]
    fn finite_pattern_deeper_than_budget_is_reported_as_incomplete() {
        let tmp = tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("a/b")).unwrap();
        fs::write(tmp.path().join("a/b/marker.txt"), "hello").unwrap();
        let plan = ScanPlan::new(tmp.path().to_path_buf(), "*/*/*.txt").unwrap();
        let cancelled = AtomicBool::new(false);
        let report = scan_files(
            plan,
            ScanLimits {
                max_depth: 1,
                deadline: Duration::from_secs(1),
                ..ScanLimits::default()
            },
            &cancelled,
        );

        assert_eq!(report.stop, Some(ScanStop::DepthLimit));
        assert!(report.files.is_empty());
    }

    #[test]
    fn pending_directory_memory_budget_stops_a_wide_tree() {
        let tmp = tempdir().unwrap();
        fs::create_dir(tmp.path().join("a")).unwrap();
        fs::create_dir(tmp.path().join("b")).unwrap();
        let plan = ScanPlan::new(tmp.path().to_path_buf(), "**/*.missing").unwrap();
        let cancelled = AtomicBool::new(false);
        let report = scan_files(
            plan,
            ScanLimits {
                max_pending_dirs: 1,
                deadline: Duration::from_secs(1),
                ..ScanLimits::default()
            },
            &cancelled,
        );

        assert_eq!(report.stop, Some(ScanStop::PendingLimit));
        assert!(report.files.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn recursive_glob_follows_skill_link_but_terminates_on_ancestor_cycle() {
        use std::os::unix::fs::symlink;

        let tmp = tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let skill = tmp.path().join("skills/apple-design");
        fs::create_dir_all(workspace.join(".nomi/skills")).unwrap();
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Apple Design").unwrap();
        symlink(&skill, workspace.join(".nomi/skills/apple-design")).unwrap();
        symlink(&workspace, workspace.join("cycle")).unwrap();
        let workspace_link = tmp.path().join("workspace-link");
        symlink(&workspace, &workspace_link).unwrap();

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run_glob("**/SKILL.md", workspace.to_str().unwrap()),
        )
        .await
        .expect("symlink cycle must not keep Glob running");

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(
            result.content.contains(".nomi/skills/apple-design/SKILL.md"),
            "legitimate workspace skill link must stay searchable: {}",
            result.content
        );
        assert_eq!(result.content.lines().count(), 1);

        let missing = tokio::time::timeout(
            Duration::from_secs(1),
            run_glob("**/*.definitely-missing", workspace.to_str().unwrap()),
        )
        .await
        .expect("zero-match search must also terminate across a symlink cycle");
        assert!(!missing.is_error, "unexpected error: {}", missing.content);
        assert_eq!(missing.content, "No files matched the pattern");

        let linked_root = run_glob("**/SKILL.md", workspace_link.to_str().unwrap()).await;
        assert!(
            !linked_root.is_error,
            "root symlink must remain searchable: {}",
            linked_root.content
        );
        assert!(linked_root.content.contains("apple-design/SKILL.md"));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn recursive_glob_follows_skill_junction_but_terminates_on_ancestor_cycle() {
        let tmp = tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let skill = tmp.path().join("skills/apple-design");
        fs::create_dir_all(workspace.join(".nomi/skills")).unwrap();
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "# Apple Design").unwrap();
        junction::create(&skill, workspace.join(".nomi/skills/apple-design")).unwrap();
        junction::create(&workspace, workspace.join("cycle")).unwrap();
        let workspace_link = tmp.path().join("workspace-link");
        junction::create(&workspace, &workspace_link).unwrap();

        let result = tokio::time::timeout(
            Duration::from_secs(5),
            run_glob("**/SKILL.md", workspace.to_str().unwrap()),
        )
        .await
        .expect("junction cycle must not keep Glob running");

        assert!(!result.is_error, "unexpected error: {}", result.content);
        assert!(
            result
                .content
                .contains(r".nomi\skills\apple-design\SKILL.md"),
            "legitimate workspace skill junction must stay searchable: {}",
            result.content
        );
        assert_eq!(result.content.lines().count(), 1);

        let missing = tokio::time::timeout(
            Duration::from_secs(5),
            run_glob("**/*.definitely-missing", workspace.to_str().unwrap()),
        )
        .await
        .expect("zero-match search must also terminate across a junction cycle");
        assert!(!missing.is_error, "unexpected error: {}", missing.content);
        assert_eq!(missing.content, "No files matched the pattern");

        let linked_root = run_glob("**/SKILL.md", workspace_link.to_str().unwrap()).await;
        assert!(
            !linked_root.is_error,
            "root junction must remain searchable: {}",
            linked_root.content
        );
        assert!(linked_root.content.contains(r"apple-design\SKILL.md"));
    }

    #[tokio::test]
    async fn same_root_scan_capacity_wait_is_bounded() {
        let tmp = tempdir().unwrap();
        let first = acquire_scan_capacity(tmp.path(), Duration::from_secs(1))
            .await
            .expect("first scan should acquire capacity");
        let second = acquire_scan_capacity(tmp.path(), Duration::from_millis(10)).await;
        assert!(second.is_none(), "same-root retry must not start concurrently");
        drop(first);
        assert!(
            acquire_scan_capacity(tmp.path(), Duration::from_secs(1))
                .await
                .is_some(),
            "capacity must recover after the original scan completes"
        );
    }
}
