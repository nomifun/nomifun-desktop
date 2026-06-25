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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

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
    /// Persist (append) an entry under its eTLD+1 namespace.
    fn write(&self, etld1: &str, entry: &SiteMemoryEntry);
    /// Read all entries for a given eTLD+1.
    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry>;
    /// Overwrite all entries for a given eTLD+1 (used by reconcile to drop stale).
    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]);
}

// ─── InMemorySink (test fake) ────────────────────────────────────────────────

/// In-memory fake sink for testing (no disk, no KnowledgeService dependency).
pub struct InMemorySink {
    store: Mutex<HashMap<String, Vec<SiteMemoryEntry>>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(HashMap::new()),
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
        let mut map = self.store.lock().expect("InMemorySink poisoned");
        map.entry(etld1.to_string()).or_default().push(entry.clone());
    }

    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry> {
        let map = self.store.lock().expect("InMemorySink poisoned");
        map.get(etld1).cloned().unwrap_or_default()
    }

    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]) {
        let mut map = self.store.lock().expect("InMemorySink poisoned");
        map.insert(etld1.to_string(), entries.to_vec());
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
    /// Serializes read-modify-write so concurrent `write` calls can't lose entries.
    lock: Mutex<()>,
}

impl FileSiteMemorySink {
    /// Create a sink rooted at `root` (e.g. `<data_dir>/browser/site-memory`).
    /// Best-effort creates the directory; failure is non-fatal (writes retry mkdir).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        if let Err(e) = std::fs::create_dir_all(&root) {
            tracing::warn!(
                target: "nomi_browser::site_memory", error = %e, dir = %root.display(),
                "failed to create site-memory dir; will retry on write"
            );
        }
        Self { root, lock: Mutex::new(()) }
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

    fn read_file(path: &Path) -> Vec<SiteMemoryEntry> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!(
                    target: "nomi_browser::site_memory", error = %e, path = %path.display(),
                    "corrupt site-memory file; treating as empty"
                );
                Vec::new()
            }),
            Err(_) => Vec::new(), // missing file = no entries (not an error)
        }
    }

    fn write_file(path: &Path, entries: &[SiteMemoryEntry]) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent); // best-effort (dir may have been removed)
        }
        match serde_json::to_vec_pretty(entries) {
            Ok(bytes) => {
                if let Err(e) = std::fs::write(path, &bytes) {
                    tracing::warn!(
                        target: "nomi_browser::site_memory", error = %e, path = %path.display(),
                        "failed to persist site-memory; entry dropped (best-effort)"
                    );
                }
            }
            Err(e) => tracing::warn!(
                target: "nomi_browser::site_memory", error = %e,
                "failed to serialize site-memory entries"
            ),
        }
    }
}

impl SiteMemorySink for FileSiteMemorySink {
    fn write(&self, etld1: &str, entry: &SiteMemoryEntry) {
        let Some(path) = self.path_for(etld1) else { return };
        let _guard = self.lock.lock().expect("site-memory file lock poisoned");
        let mut entries = Self::read_file(&path);
        entries.push(entry.clone());
        Self::write_file(&path, &entries);
    }

    fn read(&self, etld1: &str) -> Vec<SiteMemoryEntry> {
        let Some(path) = self.path_for(etld1) else { return Vec::new() };
        let _guard = self.lock.lock().expect("site-memory file lock poisoned");
        Self::read_file(&path)
    }

    fn write_all(&self, etld1: &str, entries: &[SiteMemoryEntry]) {
        let Some(path) = self.path_for(etld1) else { return };
        let _guard = self.lock.lock().expect("site-memory file lock poisoned");
        Self::write_file(&path, entries);
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
                // If the entry has a selector and the selector is present in the
                // current observe, check role/name match. Mismatch → stale → drop.
                if let Some(ref sel) = e.selector
                    && let Some((cur_role, cur_name)) = current_by_selector.get(sel)
                {
                    return e.role == *cur_role && e.accessible_name == *cur_name;
                }
                // No selector or selector not found in current → keep (can't invalidate).
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
}
