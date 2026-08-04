//! **P7A — Site Memory (站点记忆)**
//!
//! Remembers a site's structure across sessions — per eTLD+1, stores stable element
//! descriptors (aria role+name, selector) and successful action paths so repeat tasks
//! on known sites skip re-exploration.
//!
//! Architecture: thin layer over a `SiteMemorySink` trait. Production impl =
//! [`FileSiteMemorySink`] (one JSON file per eTLD+1 under the data dir — sync, no new
//! deps, mirrors the codebase's existing JSON-to-data-dir persistence); tests use an
//! in-memory fake. **Deliberately NOT backed by `KnowledgeService`**: that is an async
//! RAG document store, so adapting this sync trait to it would block-on-async and would
//! pollute the user's searchable knowledge bases with machine-generated browser hints.
//! Keyed globally by eTLD+1 (NOT per-pet — browser identity is globally shared).
//!
//! **Locked invariant:** No secret value EVER stored. Entries sourced from a
//! `secret:NAME` action or whose accessible_name is a redaction placeholder are
//! dropped before persistence.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// Per-site hard bounds. Site memory is an optional hint cache, so bounded
/// recency is preferable to allowing successful actions to grow memory/disk
/// forever.
const MAX_ENTRIES_PER_DOMAIN: usize = 256;
const MAX_ENTRY_BYTES: usize = 16 * 1024;
const MAX_DOMAIN_BYTES: usize = 256 * 1024;
/// Allows one bounded read of an older file before it is compacted to the new
/// per-domain limit. Files above this size are never allocated/read.
const MAX_READ_BYTES: u64 = 512 * 1024;
const MAX_DOMAIN_FILES: usize = 512;
const MAX_TOTAL_DISK_BYTES: u64 = 64 * 1024 * 1024;
const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);
const FILE_LOCK_SHARDS: usize = 16;

static FILE_SINK_LOCKS: OnceLock<Vec<Mutex<()>>> = OnceLock::new();
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn file_sink_lock(root: &Path) -> &'static Mutex<()> {
    let locks = FILE_SINK_LOCKS.get_or_init(|| {
        (0..FILE_LOCK_SHARDS)
            .map(|_| Mutex::new(()))
            .collect()
    });
    let mut hasher = DefaultHasher::new();
    root.hash(&mut hasher);
    &locks[hasher.finish() as usize % locks.len()]
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)?;
    if let Some(parent) = to.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: both pointers refer to live NUL-terminated UTF-16 buffers for the
    // duration of the call. Flags request a same-volume atomic replacement and
    // durable completion; no ownership is transferred to Windows.
    let succeeded = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

// ─── Entry ───────────────────────────────────────────────────────────────────

/// A single remembered element descriptor for a site.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SiteMemoryEntry {
    /// The eTLD+1 this entry belongs to (e.g. "google.com").
    pub etld1: String,
    /// A URL pattern hint (not authoritative — informational only).
    pub url_pattern: String,
    /// What the user was trying to do (intent/action name).
    pub intent: String,
    /// Aria role of the element.
    pub role: String,
    /// Accessible name of the element.
    pub accessible_name: String,
    /// A CSS selector (if available) for faster re-location.
    pub selector: Option<String>,
    /// Whether this entry originated from a secret-carrying action.
    /// If true, the entry is NEVER persisted (dropped at record time).
    #[serde(default)]
    pub from_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum EntryLocatorIdentity {
    Selector(String),
    Accessible { role: String, name: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EntryIdentity {
    /// Query strings and fragments are intentionally excluded: they are both
    /// unstable and a common place for credentials/tokens to appear.
    url_scope: String,
    intent: String,
    locator: EntryLocatorIdentity,
}

fn entry_identity(entry: &SiteMemoryEntry) -> EntryIdentity {
    let locator = entry
        .selector
        .as_ref()
        .filter(|selector| !selector.is_empty())
        .map(|selector| EntryLocatorIdentity::Selector(selector.clone()))
        .unwrap_or_else(|| EntryLocatorIdentity::Accessible {
            role: entry.role.clone(),
            name: entry.accessible_name.clone(),
        });
    EntryIdentity {
        url_scope: entry.url_pattern.clone(),
        intent: entry.intent.clone(),
        locator,
    }
}

/// Remove URL components which are unstable or can carry secrets. This keeps
/// compatibility with the old string field while ensuring all newly rewritten
/// files use a stable page scope.
fn stable_url_scope(url: &str) -> String {
    let end = url
        .char_indices()
        .find_map(|(index, ch)| matches!(ch, '?' | '#').then_some(index))
        .unwrap_or(url.len());
    let mut stable = url[..end].to_string();

    // Best-effort user-info removal without adding a URL dependency. Only alter
    // strings with an explicit scheme/authority separator.
    if let Some(scheme_end) = stable.find("://") {
        let authority_start = scheme_end + 3;
        let authority_end = stable[authority_start..]
            .find('/')
            .map(|offset| authority_start + offset)
            .unwrap_or(stable.len());
        if let Some(at_offset) = stable[authority_start..authority_end].rfind('@') {
            let host_start = authority_start + at_offset + 1;
            stable.replace_range(authority_start..host_start, "");
        }
    }
    stable
}

fn normalize_entry(etld1: &str, mut entry: SiteMemoryEntry) -> Option<SiteMemoryEntry> {
    if entry.etld1 != etld1
        || entry.from_secret
        || is_redaction_placeholder(&entry.accessible_name)
    {
        return None;
    }
    entry.url_pattern = stable_url_scope(&entry.url_pattern);
    if entry.selector.as_deref() == Some("") {
        entry.selector = None;
    }
    let encoded_len = serde_json::to_vec(&entry).ok()?.len();
    (encoded_len <= MAX_ENTRY_BYTES).then_some(entry)
}

/// Canonicalize old and new records, keep the newest value for each stable
/// identity, and enforce both count and serialized-byte bounds.
fn normalize_entries(
    etld1: &str,
    entries: impl IntoIterator<Item = SiteMemoryEntry>,
) -> Vec<SiteMemoryEntry> {
    let normalized: Vec<_> = entries
        .into_iter()
        .filter_map(|entry| normalize_entry(etld1, entry))
        .collect();
    let mut reversed = Vec::new();
    let mut seen = HashSet::new();
    for entry in normalized.into_iter().rev() {
        if seen.insert(entry_identity(&entry)) {
            reversed.push(entry);
            if reversed.len() == MAX_ENTRIES_PER_DOMAIN {
                break;
            }
        }
    }
    reversed.reverse();

    let entry_sizes: Vec<usize> = reversed
        .iter()
        .map(|entry| serde_json::to_vec(entry).map_or(MAX_ENTRY_BYTES + 1, |bytes| bytes.len()))
        .collect();
    // JSON array = two brackets + one comma between adjacent entries.
    let mut serialized_len = 2_usize
        .saturating_add(entry_sizes.iter().copied().sum::<usize>())
        .saturating_add(reversed.len().saturating_sub(1));
    let mut evict = 0;
    while serialized_len > MAX_DOMAIN_BYTES && evict < reversed.len() {
        serialized_len = serialized_len.saturating_sub(entry_sizes[evict]);
        if reversed.len().saturating_sub(evict) > 1 {
            serialized_len = serialized_len.saturating_sub(1);
        }
        evict += 1;
    }
    if evict != 0 {
        reversed.drain(..evict);
    }
    reversed
}

// ─── Redaction placeholders (locked invariant: secret → drop) ────────────────

/// Redaction placeholder markers. If an entry's accessible_name matches any of
/// these, the entry is considered secret-sourced and MUST NOT be persisted.
const REDACTION_MARKERS: &[&str] = &[
    "[REDACTED]",
    "[REDACTED_SECRET]",
    "[KNOWN_SECRET_REDACTED]",
];

/// Returns true if `name` is a redaction placeholder (secret-sourced).
fn is_redaction_placeholder(name: &str) -> bool {
    REDACTION_MARKERS.iter().any(|m| name.contains(m))
}

// ─── eTLD+1 keying ───────────────────────────────────────────────────────────

/// Extract the eTLD+1 key for a given URL. Returns `None` for IPs, localhost,
/// or anything without a registrable domain.
///
/// Reuses the same PSL machinery as the firewall (`nomifun_secret::etld_plus_one`),
/// plus the IP-literal guard (`ip_literal_of_host`) to reject numeric hosts that the
/// PSL crate misclassifies as domains.
pub fn key_for(url: &str) -> Option<String> {
    // Guard: IP literals (v4/v6) have no registrable domain — reject before PSL.
    // Same pattern as firewall's `registrable_domain_for_trust`.
    let host = nomifun_secret::host_of(url)?;
    if nomi_browser_engine::firewall::ip_literal_of_host(&host).is_some() {
        return None;
    }
    nomifun_secret::etld_plus_one(url)
}

// ─── SiteMemorySink trait ────────────────────────────────────────────────────

/// Abstraction over the persistence backend. The production impl is
/// [`FileSiteMemorySink`]; tests use [`InMemorySink`]. Keyed by eTLD+1.
pub trait SiteMemorySink: Send + Sync {
    /// Persist (bounded upsert) an entry under its eTLD+1 namespace.
    fn write(&self, etld1: &str, entry: &SiteMemoryEntry);
    /// Read all entries for a given eTLD+1.
    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry>;
    /// Overwrite all entries for a given eTLD+1 (used by reconcile to drop stale).
    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]);
}

// ─── InMemorySink (test fake) ────────────────────────────────────────────────

/// In-memory fake sink for testing (no disk, no KnowledgeService dependency).
pub struct InMemorySink {
    state: Mutex<InMemoryState>,
}

#[derive(Default)]
struct InMemoryState {
    store: HashMap<String, Vec<SiteMemoryEntry>>,
    /// Least-recently-written domain first. This makes the fake obey the same
    /// global domain bound as the production sink.
    domain_order: VecDeque<String>,
    total_bytes: u64,
}

impl InMemoryState {
    fn replace(&mut self, etld1: &str, entries: Vec<SiteMemoryEntry>) {
        if let Some(old) = self.store.remove(etld1) {
            self.total_bytes = self
                .total_bytes
                .saturating_sub(serde_json::to_vec(&old).map_or(0, |bytes| bytes.len()) as u64);
        }
        self.domain_order.retain(|domain| domain != etld1);

        if !entries.is_empty() {
            self.total_bytes = self
                .total_bytes
                .saturating_add(serde_json::to_vec(&entries).map_or(0, |bytes| bytes.len()) as u64);
            self.store.insert(etld1.to_string(), entries);
            self.domain_order.push_back(etld1.to_string());
        }

        while self.store.len() > MAX_DOMAIN_FILES || self.total_bytes > MAX_TOTAL_DISK_BYTES {
            let Some(oldest) = self.domain_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.store.remove(&oldest) {
                self.total_bytes = self.total_bytes.saturating_sub(
                    serde_json::to_vec(&removed).map_or(0, |bytes| bytes.len()) as u64,
                );
            }
        }
    }
}

impl InMemorySink {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(InMemoryState::default()),
        }
    }
}

impl Default for InMemorySink {
    fn default() -> Self {
        Self::new()
    }
}

impl SiteMemorySink for InMemorySink {
    fn write(&self, etld1: &str, entry: &SiteMemoryEntry) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut entries = state.store.get(etld1).cloned().unwrap_or_default();
        entries.push(entry.clone());
        state.replace(etld1, normalize_entries(etld1, entries));
    }

    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.store.get(etld1).cloned().unwrap_or_default()
    }

    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.replace(etld1, normalize_entries(etld1, entries.to_vec()));
    }
}

// ─── FileSiteMemorySink (production: one JSON file per eTLD+1) ────────────────

/// Production sink: persists each eTLD+1's entries as `<root>/<etld1>.json` holding a
/// `Vec<SiteMemoryEntry>`. Sync, no new deps — mirrors the codebase's existing
/// JSON-to-data-dir persistence (`device_auth_store`, `device_identity`).
///
/// **Security (path-traversal guard):** the eTLD+1 key is derived from a *visited URL*
/// and is therefore attacker-influenceable. The filename is strictly validated to a
/// registrable-domain charset; any key that fails validation is a **no-op** (read→empty,
/// write→skip) so it can never escape `root` (`../../etc/...`, `/abs`, `a/b`, …). IDN
/// (raw-unicode) hosts are conservatively rejected too (fail-safe: such a site simply
/// gets no site-memory rather than risking an unsafe filename).
///
/// **Best-effort:** I/O errors are logged and swallowed — site-memory is an optimization,
/// never a correctness dependency, so a persistence failure must not break a browser action.
pub struct FileSiteMemorySink {
    root: PathBuf,
}

impl FileSiteMemorySink {
    /// Create a sink rooted at `root` (e.g. `<data_dir>/browser/site-memory`).
    /// Best-effort creates the directory; failure is non-fatal (writes retry mkdir).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut root = root.into();
        if let Err(e) = std::fs::create_dir_all(&root) {
            tracing::warn!(
                target: "nomi_browser::site_memory", error = %e, dir = %root.display(),
                "failed to create site-memory dir; will retry on write"
            );
        }
        // Canonicalization makes independently constructed sinks for the same
        // directory land on the same bounded lock shard.
        if let Ok(canonical) = std::fs::canonicalize(&root) {
            root = canonical;
        }
        let sink = Self { root };
        let _guard = file_sink_lock(&sink.root)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sink.maintain_root_quota(None);
        sink
    }

    /// Validate + resolve the on-disk path for an eTLD+1 key. `None` if the key is not
    /// a safe registrable-domain string (path-traversal guard → caller treats as no-op).
    fn path_for(&self, etld1: &str) -> Option<PathBuf> {
        if !is_safe_etld1_filename(etld1) {
            tracing::warn!(
                target: "nomi_browser::site_memory", key = %etld1,
                "site-memory key rejected (unsafe filename); skipping persistence"
            );
            return None;
        }
        Some(self.root.join(format!("{etld1}.json")))
    }

    /// Read at most `MAX_READ_BYTES + 1`, even if the file grows after its
    /// metadata check. This prevents a corrupt/hostile legacy file from causing
    /// an unbounded allocation.
    fn read_file(path: &Path) -> Vec<SiteMemoryEntry> {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(_) => return Vec::new(),
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            tracing::warn!(
                target: "nomi_browser::site_memory", path = %path.display(),
                "site-memory path is not a regular file; refusing to read it"
            );
            return Vec::new();
        }
        if metadata.len() > MAX_READ_BYTES {
            tracing::warn!(
                target: "nomi_browser::site_memory", path = %path.display(),
                bytes = metadata.len(), limit = MAX_READ_BYTES,
                "oversized site-memory file rejected"
            );
            return Vec::new();
        }

        let file = match File::open(path) {
            Ok(file) => file,
            Err(_) => return Vec::new(),
        };
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        if let Err(error) = file.take(MAX_READ_BYTES + 1).read_to_end(&mut bytes) {
            tracing::warn!(
                target: "nomi_browser::site_memory", %error, path = %path.display(),
                "failed to read site-memory file"
            );
            return Vec::new();
        }
        if bytes.len() as u64 > MAX_READ_BYTES {
            tracing::warn!(
                target: "nomi_browser::site_memory", path = %path.display(),
                limit = MAX_READ_BYTES,
                "site-memory file grew beyond the read limit; rejecting it"
            );
            return Vec::new();
        }
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            tracing::warn!(
                target: "nomi_browser::site_memory", %error, path = %path.display(),
                "corrupt site-memory file; treating as empty"
            );
            Vec::new()
        })
    }

    /// Atomically replace one domain file using a same-directory temporary file.
    /// The platform replacement primitive ensures readers observe either the
    /// complete old file or the complete new file.
    fn write_file(path: &Path, entries: &[SiteMemoryEntry]) -> bool {
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                tracing::warn!(
                    target: "nomi_browser::site_memory", %error, path = %parent.display(),
                    "failed to create site-memory directory"
                );
                return false;
            }
        }

        if let Ok(metadata) = std::fs::symlink_metadata(path)
            && (!metadata.file_type().is_file() || metadata.file_type().is_symlink())
        {
            tracing::warn!(
                target: "nomi_browser::site_memory", path = %path.display(),
                "site-memory destination is not a regular file; refusing to replace it"
            );
            return false;
        }

        if entries.is_empty() {
            return match std::fs::remove_file(path) {
                Ok(()) => true,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(error) => {
                    tracing::warn!(
                        target: "nomi_browser::site_memory", %error, path = %path.display(),
                        "failed to remove empty site-memory file"
                    );
                    false
                }
            };
        }

        let bytes = match serde_json::to_vec(entries) {
            Ok(bytes) if bytes.len() <= MAX_DOMAIN_BYTES => bytes,
            Ok(bytes) => {
                tracing::warn!(
                    target: "nomi_browser::site_memory", bytes = bytes.len(),
                    limit = MAX_DOMAIN_BYTES, path = %path.display(),
                    "refusing to write oversized site-memory domain"
                );
                return false;
            }
            Err(error) => {
                tracing::warn!(
                    target: "nomi_browser::site_memory", %error,
                    "failed to serialize site-memory entries"
                );
                return false;
            }
        };

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let mut temp = None;
        for _ in 0..8 {
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let candidate = parent.join(format!(
                ".site-memory-{}-{sequence}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&candidate)
            {
                Ok(file) => {
                    temp = Some((candidate, file));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    tracing::warn!(
                        target: "nomi_browser::site_memory", %error, path = %candidate.display(),
                        "failed to create site-memory temporary file"
                    );
                    return false;
                }
            }
        }
        let Some((temp_path, mut temp_file)) = temp else {
            tracing::warn!(
                target: "nomi_browser::site_memory", path = %path.display(),
                "failed to allocate a unique site-memory temporary file"
            );
            return false;
        };

        let persisted = (|| -> std::io::Result<()> {
            temp_file.write_all(&bytes)?;
            temp_file.sync_all()?;
            drop(temp_file);
            atomic_replace(&temp_path, path)
        })();
        if let Err(error) = persisted {
            let _ = std::fs::remove_file(&temp_path);
            tracing::warn!(
                target: "nomi_browser::site_memory", %error, path = %path.display(),
                "failed to atomically persist site-memory"
            );
            return false;
        }
        true
    }

    fn is_managed_domain_path(path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(is_safe_etld1_filename)
    }

    fn is_managed_temp_path(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        name.starts_with(".site-memory-") && name.ends_with(".tmp")
    }

    /// Enforce a global domain/disk quota and remove crash-left temporary files.
    /// Only files whose names match this module's strict managed patterns are
    /// touched; unrelated data in the directory is never deleted.
    fn maintain_root_quota(&self, protected: Option<&Path>) {
        let Ok(dir) = std::fs::read_dir(&self.root) else {
            return;
        };
        let now = SystemTime::now();
        let current_pid_prefix = format!(".site-memory-{}-", std::process::id());
        let mut files = Vec::new();
        let mut unreclaimed_count = 0_usize;
        let mut unreclaimed_bytes = 0_u64;
        for item in dir.flatten() {
            let path = item.path();
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                continue;
            }
            if Self::is_managed_temp_path(&path) {
                let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
                let stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= STALE_TEMP_AGE);
                // The process-global mutex proves no same-process temp is live.
                if name.starts_with(&current_pid_prefix) || stale {
                    let _ = std::fs::remove_file(&path);
                }
                continue;
            }
            if !Self::is_managed_domain_path(&path) {
                continue;
            }
            if metadata.len() > MAX_READ_BYTES && protected != Some(path.as_path()) {
                tracing::warn!(
                    target: "nomi_browser::site_memory", path = %path.display(),
                    bytes = metadata.len(), limit = MAX_READ_BYTES,
                    "removing oversized managed site-memory file"
                );
                match std::fs::remove_file(&path) {
                    Ok(()) => continue,
                    Err(error) => tracing::warn!(
                        target: "nomi_browser::site_memory", %error, path = %path.display(),
                        "failed to remove oversized managed site-memory file"
                    ),
                }
            }
            // Compact an old append-only file immediately when it exceeds the
            // new on-disk per-domain bound. This preserves the legacy Vec format
            // while ensuring an unqueried domain cannot retain over-quota debt.
            if metadata.len() > MAX_DOMAIN_BYTES as u64 {
                let etld1 = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or_default();
                let compacted = normalize_entries(etld1, Self::read_file(&path));
                Self::write_file(&path, &compacted);
                if !path.exists() {
                    continue;
                }
            }
            let Ok(metadata) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            files.push((
                path,
                metadata.len(),
                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            ));
            // Keep only a bounded newest-domain working set while scanning. A
            // legacy directory containing millions of files must not turn quota
            // reconciliation itself into O(N) memory growth.
            if files.len() > MAX_DOMAIN_FILES {
                let Some((oldest_index, _)) = files
                    .iter()
                    .enumerate()
                    .filter(|(_, (path, _, _))| protected != Some(path.as_path()))
                    .min_by(|(_, left), (_, right)| {
                        left.2.cmp(&right.2).then_with(|| left.0.cmp(&right.0))
                    })
                else {
                    continue;
                };
                let (oldest_path, oldest_bytes, _) = files.remove(oldest_index);
                if let Err(error) = std::fs::remove_file(&oldest_path) {
                    unreclaimed_count = unreclaimed_count.saturating_add(1);
                    unreclaimed_bytes = unreclaimed_bytes.saturating_add(oldest_bytes);
                    tracing::warn!(
                        target: "nomi_browser::site_memory", %error,
                        path = %oldest_path.display(),
                        "failed to evict excess site-memory domain"
                    );
                }
            }
        }

        files.sort_by(|left, right| left.2.cmp(&right.2).then_with(|| left.0.cmp(&right.0)));
        let mut total = files
            .iter()
            .fold(unreclaimed_bytes, |sum, (_, bytes, _)| sum.saturating_add(*bytes));
        let mut count = files.len().saturating_add(unreclaimed_count);
        for (path, bytes, _) in files {
            if count <= MAX_DOMAIN_FILES && total <= MAX_TOTAL_DISK_BYTES {
                break;
            }
            if protected == Some(path.as_path()) {
                continue;
            }
            match std::fs::remove_file(&path) {
                Ok(()) => {
                    count = count.saturating_sub(1);
                    total = total.saturating_sub(bytes);
                }
                Err(error) => tracing::warn!(
                    target: "nomi_browser::site_memory", %error, path = %path.display(),
                    "failed to evict site-memory file for global quota"
                ),
            }
        }
        if count > MAX_DOMAIN_FILES || total > MAX_TOTAL_DISK_BYTES {
            tracing::warn!(
                target: "nomi_browser::site_memory", count, total,
                max_count = MAX_DOMAIN_FILES, max_bytes = MAX_TOTAL_DISK_BYTES,
                "site-memory global quota remains exceeded after bounded cleanup"
            );
        }
    }
}

impl SiteMemorySink for FileSiteMemorySink {
    fn write(&self, etld1: &str, entry: &SiteMemoryEntry) {
        let Some(path) = self.path_for(etld1) else { return };
        let _guard = file_sink_lock(&self.root)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let raw = Self::read_file(&path);
        let mut candidates = raw.clone();
        candidates.push(entry.clone());
        let entries = normalize_entries(etld1, candidates);
        let changed = entries != raw || (entries.is_empty() && path.exists());
        if changed {
            Self::write_file(&path, &entries);
            self.maintain_root_quota(Some(&path));
        }
    }

    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry> {
        let Some(path) = self.path_for(etld1) else { return Vec::new() };
        let _guard = file_sink_lock(&self.root)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let raw = Self::read_file(&path);
        let entries = normalize_entries(etld1, raw.clone());
        // Opportunistically migrate old append-only/duplicate/oversized-in-policy
        // files. Corrupt or over-read-limit files normalize to empty and are
        // removed, preventing persistent disk debt.
        let changed = entries != raw || (entries.is_empty() && path.exists());
        if changed {
            Self::write_file(&path, &entries);
            self.maintain_root_quota(Some(&path));
        }
        entries
    }

    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]) {
        let Some(path) = self.path_for(etld1) else { return };
        let _guard = file_sink_lock(&self.root)
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entries = normalize_entries(etld1, entries.to_vec());
        Self::write_file(&path, &entries);
        self.maintain_root_quota(Some(&path));
    }
}

/// Strict registrable-domain filename validation (path-traversal guard). The key comes
/// from a visited URL (attacker-influenceable), so only allow what a real eTLD+1 can
/// contain: non-empty, ≤253 bytes, ASCII `[a-zA-Z0-9.-]`, no `..`, no leading/trailing
/// dot or dash. Everything else (separators, absolute paths, IDN unicode) is rejected.
fn is_safe_etld1_filename(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    if s.starts_with('.') || s.ends_with('.') || s.starts_with('-') || s.ends_with('-') {
        return false;
    }
    if s.contains("..") {
        return false;
    }
    s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

// ─── SiteMemoryStore ─────────────────────────────────────────────────────────

/// The main site-memory store. Wraps a [`SiteMemorySink`] and enforces invariants
/// (secret-skip, dedup) before delegating to the sink.
pub struct SiteMemoryStore {
    sink: Box<dyn SiteMemorySink>,
}

impl SiteMemoryStore {
    /// Create a new store backed by the given sink.
    pub fn new(sink: Box<dyn SiteMemorySink>) -> Self {
        Self { sink }
    }

    /// Record a successful action's element descriptor.
    ///
    /// **Locked invariant:** drops the entry if `from_secret == true` OR the
    /// accessible_name is a redaction placeholder. No secret value ever reaches
    /// the sink.
    pub fn record(&self, entry: SiteMemoryEntry) {
        // Secret guard: never persist secret-sourced descriptors.
        if entry.from_secret || is_redaction_placeholder(&entry.accessible_name) {
            return;
        }
        self.sink.write(&entry.etld1, &entry);
    }

    /// Query remembered hints for a given eTLD+1.
    pub fn query(&self, etld1: &str) -> Vec<SiteMemoryEntry> {
        self.sink.read(etld1)
    }

    /// Reconcile remembered entries against the current observation: drop entries
    /// whose selector now resolves to a different role/name (stale).
    ///
    /// `current_elements` is a list of (role, accessible_name) pairs from the
    /// current observe snapshot, keyed by selector (for entries that have one).
    pub fn reconcile(
        &self,
        etld1: &str,
        current_by_selector: &HashMap<String, (String, String)>,
    ) {
        let entries = self.sink.read(etld1);
        if entries.is_empty() {
            return;
        }
        let retained: Vec<SiteMemoryEntry> = entries
            .into_iter()
            .filter(|e| {
                // A selector-bearing entry is valid only when the current
                // observation still contains it with the same role/name.
                // Missing selectors are stale too; keeping them was the old path
                // by which dead hints accumulated forever.
                if let Some(ref sel) = e.selector {
                    return current_by_selector.get(sel).is_some_and(
                        |(cur_role, cur_name)| {
                            e.role == *cur_role && e.accessible_name == *cur_name
                        },
                    );
                }
                // Facade-generated entries have no selector and cannot be
                // authoritatively invalidated by this selector-indexed snapshot.
                true
            })
            .collect();
        self.sink.write_all(etld1, &retained);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(etld1: &str, name: &str) -> SiteMemoryEntry {
        SiteMemoryEntry {
            etld1: etld1.into(),
            url_pattern: format!("https://{etld1}/"),
            intent: "click".into(),
            role: "button".into(),
            accessible_name: name.into(),
            selector: Some(format!("#{name}")),
            from_secret: false,
        }
    }

    #[test]
    fn file_sink_write_read_round_trip() {
        let dir = TempDir::new().unwrap();
        let sink = FileSiteMemorySink::new(dir.path());
        sink.write("example.com", &entry("example.com", "login"));
        sink.write("example.com", &entry("example.com", "search"));
        let got = sink.read("example.com");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].accessible_name, "login");
        assert_eq!(got[1].accessible_name, "search");
        assert!(dir.path().join("example.com.json").is_file(), "really persisted to disk");
    }

    #[test]
    fn file_sink_persists_across_instances() {
        let dir = TempDir::new().unwrap();
        {
            let sink = FileSiteMemorySink::new(dir.path());
            sink.write("acme.com", &entry("acme.com", "buy"));
        } // dropped — must survive
        let sink2 = FileSiteMemorySink::new(dir.path());
        let got = sink2.read("acme.com");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].accessible_name, "buy");
    }

    #[test]
    fn file_sink_write_all_overwrites() {
        let dir = TempDir::new().unwrap();
        let sink = FileSiteMemorySink::new(dir.path());
        sink.write("a.com", &entry("a.com", "x"));
        sink.write("a.com", &entry("a.com", "y"));
        sink.write_all("a.com", &[entry("a.com", "only")]);
        let got = sink.read("a.com");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].accessible_name, "only");
    }

    #[test]
    fn file_sink_isolates_domains() {
        let dir = TempDir::new().unwrap();
        let sink = FileSiteMemorySink::new(dir.path());
        sink.write("a.com", &entry("a.com", "a-entry"));
        sink.write("b.com", &entry("b.com", "b-entry"));
        assert_eq!(sink.read("a.com").len(), 1);
        assert_eq!(sink.read("b.com").len(), 1);
        assert_eq!(sink.read("a.com")[0].accessible_name, "a-entry");
    }

    #[test]
    fn independent_sink_instances_do_not_lose_concurrent_upserts() {
        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let start = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|worker| {
                let root = root.clone();
                let start = start.clone();
                std::thread::spawn(move || {
                    let sink = FileSiteMemorySink::new(root);
                    start.wait();
                    for index in 0..16 {
                        sink.write(
                            "shared.example",
                            &entry("shared.example", &format!("w{worker}-{index}")),
                        );
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }

        let got = FileSiteMemorySink::new(root).read("shared.example");
        assert_eq!(got.len(), 32);
    }

    #[test]
    fn file_sink_rejects_path_traversal_keys() {
        let dir = TempDir::new().unwrap();
        let sink = FileSiteMemorySink::new(dir.path());
        // Path-traversal / injection attempts must be no-ops (never escape `root`).
        for bad in ["../escape", "../../etc/passwd", "a/b", "/abs", ".hidden", "a..b", "a\\b", ""] {
            sink.write(bad, &entry("x", "evil"));
            assert!(sink.read(bad).is_empty(), "unsafe key {bad:?} must not persist");
        }
        // Confirm nothing escaped into the parent of root.
        let parent = dir.path().parent().unwrap();
        assert!(!parent.join("escape.json").exists());
        assert!(!parent.join("escape").exists());
    }

    #[test]
    fn is_safe_etld1_filename_accepts_real_domains_rejects_unsafe() {
        for ok in ["example.com", "sub.example.co.uk", "xn--mnchen-3ya.de", "a-b.com"] {
            assert!(is_safe_etld1_filename(ok), "{ok} should be accepted");
        }
        for bad in ["", "../etc", "a/b", "/abs", ".leading", "trailing.", "a..b", "a\\b", "-x.com"] {
            assert!(!is_safe_etld1_filename(bad), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn store_over_file_sink_drops_secret_entries() {
        // Locked invariant holds through the real file sink: secret-sourced entries
        // never reach disk.
        let dir = TempDir::new().unwrap();
        let store = SiteMemoryStore::new(Box::new(FileSiteMemorySink::new(dir.path())));
        let mut secret = entry("bank.com", "[REDACTED]");
        secret.from_secret = true;
        store.record(secret);
        store.record(entry("bank.com", "normal-button"));
        let got = store.query("bank.com");
        assert_eq!(got.len(), 1, "secret entry must be dropped, normal kept");
        assert_eq!(got[0].accessible_name, "normal-button");
    }

    #[test]
    fn one_hundred_thousand_duplicate_records_stay_one_upserted_entry() {
        let store = SiteMemoryStore::new(Box::new(InMemorySink::new()));
        for index in 0..100_000 {
            let mut repeated = entry("repeat.example", &format!("name-{index}"));
            repeated.selector = Some("#stable-target".into());
            repeated.url_pattern = format!(
                "https://repeat.example/account?token=secret-{index}#dynamic"
            );
            store.record(repeated);
        }

        let got = store.query("repeat.example");
        assert_eq!(got.len(), 1, "stable identity must be an upsert, not append");
        assert_eq!(got[0].accessible_name, "name-99999", "newest descriptor wins");
        assert_eq!(
            got[0].url_pattern,
            "https://repeat.example/account",
            "unstable/secret-bearing URL components must not be persisted"
        );
    }

    #[test]
    fn per_domain_entry_and_serialized_byte_limits_are_hard() {
        let sink = InMemorySink::new();
        for index in 0..(MAX_ENTRIES_PER_DOMAIN * 4) {
            sink.write(
                "bounded.example",
                &entry("bounded.example", &format!("target-{index:04}")),
            );
        }
        let got = sink.read("bounded.example");
        assert_eq!(got.len(), MAX_ENTRIES_PER_DOMAIN);
        assert_eq!(
            got.first().unwrap().accessible_name,
            format!("target-{:04}", MAX_ENTRIES_PER_DOMAIN * 3),
            "oldest records must be evicted"
        );
        assert!(serde_json::to_vec(&got).unwrap().len() <= MAX_DOMAIN_BYTES);

        let mut too_large = entry("bounded.example", "large");
        too_large.accessible_name = "x".repeat(MAX_ENTRY_BYTES + 1);
        sink.write("bounded.example", &too_large);
        assert_eq!(sink.read("bounded.example"), got, "oversized entry is rejected");
    }

    #[test]
    fn file_sink_compacts_legacy_duplicate_vector_on_read() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("legacy.example.json");
        let mut old = entry("legacy.example", "old-name");
        old.selector = Some("#same".into());
        old.url_pattern = "https://legacy.example/page?old=secret".into();
        let mut newest = old.clone();
        newest.accessible_name = "new-name".into();
        newest.url_pattern = "https://legacy.example/page?new=secret".into();
        std::fs::write(
            &path,
            serde_json::to_vec(&vec![old; 100]
                .into_iter()
                .chain([newest])
                .collect::<Vec<_>>())
                .unwrap(),
        )
        .unwrap();

        let sink = FileSiteMemorySink::new(dir.path());
        let got = sink.read("legacy.example");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].accessible_name, "new-name");
        let persisted: Vec<SiteMemoryEntry> =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(persisted, got, "old Vec format remains readable and is compacted");
    }

    #[test]
    fn oversized_file_is_never_read_and_is_reclaimed() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("oversized.example.json");
        let file = File::create(&path).unwrap();
        file.set_len(MAX_READ_BYTES + 1).unwrap();

        let sink = FileSiteMemorySink::new(dir.path());
        assert!(sink.read("oversized.example").is_empty());
        assert!(!path.exists(), "managed oversized file must not remain as disk debt");
    }

    #[test]
    fn global_domain_quota_evicts_managed_domain_files_only() {
        let dir = TempDir::new().unwrap();
        let unrelated = dir.path().join("do-not-touch.txt");
        std::fs::write(&unrelated, b"owned by another subsystem").unwrap();
        let sink = FileSiteMemorySink::new(dir.path());
        for index in 0..(MAX_DOMAIN_FILES + 12) {
            let domain = format!("d{index:04}.example");
            sink.write(&domain, &entry(&domain, "target"));
        }

        let managed_count = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|item| FileSiteMemorySink::is_managed_domain_path(&item.path()))
            .count();
        assert!(managed_count <= MAX_DOMAIN_FILES);
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"owned by another subsystem");
    }

    #[test]
    fn reconcile_removes_missing_and_mismatched_selectors() {
        let store = SiteMemoryStore::new(Box::new(InMemorySink::new()));
        store.record(entry("reconcile.example", "missing"));
        store.record(entry("reconcile.example", "mismatch"));
        store.record(entry("reconcile.example", "valid"));
        let current = HashMap::from([
            ("#mismatch".into(), ("link".into(), "mismatch".into())),
            ("#valid".into(), ("button".into(), "valid".into())),
        ]);

        store.reconcile("reconcile.example", &current);
        let got = store.query("reconcile.example");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].accessible_name, "valid");
    }

    #[test]
    fn sink_rejects_cross_domain_entry_poisoning() {
        let sink = InMemorySink::new();
        sink.write("safe.example", &entry("evil.example", "target"));
        assert!(sink.read("safe.example").is_empty());
    }
}
