//! Companion memory-bundle / companion-bundle zip export & import (spec §4.8) —
//! explicit cross-machine transfer for the shared memory hub and single companions.
//!
//! Package layouts (zip root), enveloped by a strict `manifest.json`:
//! - memory bundle (`kind: "memory"`): `memories.jsonl` (every companion_memories
//!   row, archived included), `state.json`, and an empty legacy
//!   `learn_runs.jsonl` compatibility marker
//!   (`{"mood": …}`), optional raw `events/*.jsonl` day files.
//! - companion bundle (`kind: "companion"`): `companion.json` (full profile), `state.json`
//!   (`{"xp": …}`), `knowledge_refs.json` (`{"names": […]}` — binding names
//!   are collected by the frontend; this crate never touches the knowledge
//!   domain, and binding reconstruction after import is the frontend's job).
//!
//! Import uses the shared `nomifun_common::zip_safe` hardening (also used by
//! the knowledge/skill importers): component-sanitized entry paths
//! (zip-slip), symlink rejection, decompression-bomb caps, a strict entry
//! whitelist, and a manifest format/kind/version gate before anything is
//! written. v3 packages
//! are accepted only at exactly version 3; payload JSON uses closed schemas.
//! Memory import is staged and committed in one SQLite transaction. Event files
//! use no-clobber publication and an existing same-name file is idempotent only
//! when both its SHA-256 and bytes are identical.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use nomifun_common::{AppError, TimestampMs, now_ms, validate_uuidv7, zip_safe};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::profile::CompanionProfileConfig;
use crate::registry::CompanionRegistry;
use crate::service::CompanionService;
use crate::store::{CompanionMemory, CompanionStore};

/// v3 is a hard export/import baseline. Any other package version is rejected.
pub const EXPORT_FORMAT: &str = "nomifun-export";
pub const EXPORT_KIND_MEMORY: &str = "memory";
pub const EXPORT_KIND_COMPANION: &str = "companion";
pub const EXPORT_VERSION: u32 = 3;

/// Result of a successful export, returned to the frontend.
#[derive(Debug, Clone, Serialize)]
pub struct ExportSummary {
    /// `"memory"` or `"companion"`.
    pub kind: String,
    /// Data entries written to the package (manifest excluded).
    pub file_count: u64,
    /// Uncompressed size of the packaged payload.
    pub total_bytes: u64,
    pub dest_path: String,
    /// Memory rows in the package (0 for companion bundles).
    pub memories: u64,
    /// Always zero. Kept on the v3 response wire for compatibility; learning
    /// run history is no longer recorded or exported.
    pub learn_runs: u64,
    /// Raw `events/*.jsonl` files in the package (0 unless requested).
    pub event_files: u64,
}

/// Result of a successful import, returned to the frontend
/// (`{"kind":"memory",…}` / `{"kind":"companion",…}`).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ImportOutcome {
    Memory {
        /// Memory rows inserted.
        imported: u64,
        /// Memory rows skipped as duplicates of local data.
        skipped_duplicates: u64,
    },
    Companion {
        companion_id: String,
        /// Final name after duplicate-name suffixing (`"name (2)"`, …).
        name: String,
        /// Echoed back verbatim from `knowledge_refs.json` so the frontend
        /// can rebuild knowledge bindings.
        knowledge_names: Vec<String>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportManifest {
    format: String,
    version: u32,
    kind: String,
    exported_at: TimestampMs,
    app_version: String,
}

fn manifest_for(kind: &str) -> ExportManifest {
    ExportManifest {
        format: EXPORT_FORMAT.to_owned(),
        version: EXPORT_VERSION,
        kind: kind.to_owned(),
        exported_at: now_ms(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    }
}

/// A required JSON field whose value may itself be null. A plain
/// `Option<String>` accepts a missing field, which is not valid for a v3
/// payload.
#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
struct RequiredOptionalString(Option<String>);

/// `state.json` of a memory bundle. Mood is parsed strictly but deliberately
/// not applied on import (the local machine's mood wins).
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryStatePayload {
    mood: RequiredOptionalString,
}

/// Closed schema for the retired v3 `learn_runs.jsonl` payload. New exports
/// write an empty compatibility marker; old packages are still validated, then
/// their historical rows are deliberately discarded rather than re-persisted.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyLearnRun {
    learn_run_id: String,
    #[serde(rename = "started_at")]
    _started_at: TimestampMs,
    #[serde(rename = "finished_at")]
    _finished_at: Option<TimestampMs>,
    #[serde(rename = "status")]
    _status: String,
    #[serde(rename = "events_processed")]
    _events_processed: i64,
    #[serde(rename = "memories_added")]
    _memories_added: i64,
    #[serde(rename = "suggestions_added")]
    _suggestions_added: i64,
    #[serde(rename = "error")]
    _error: Option<String>,
    #[serde(rename = "summary")]
    _summary: Option<String>,
}

/// `state.json` of a companion bundle.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompanionStatePayload {
    xp: i64,
}

/// `knowledge_refs.json` of a companion bundle.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KnowledgeRefsPayload {
    names: Vec<String>,
}

// ── Roster access ───────────────────────────────────────────────────

/// The companion-roster operations a companion-bundle import needs. `CompanionService` is the
/// production implementation (live in-memory roster + WS events + default-companion
/// pointer); `CompanionRegistry` backs the tests. The registry itself must never be
/// re-scanned behind the service's back — going through the service keeps its
/// live map coherent.
#[async_trait::async_trait]
pub trait CompanionRoster: Send + Sync {
    async fn list_companions(&self) -> Vec<CompanionProfileConfig>;
    async fn create_companion(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError>;
    async fn patch_companion(
        &self,
        companion_id: &str,
        patch: serde_json::Value,
    ) -> Result<CompanionProfileConfig, AppError>;
    async fn remove_companion(&self, companion_id: &str) -> Result<(), AppError>;
}

#[async_trait::async_trait]
impl CompanionRoster for CompanionService {
    async fn list_companions(&self) -> Vec<CompanionProfileConfig> {
        CompanionService::list_companions(self).await
    }
    async fn create_companion(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError> {
        CompanionService::create_companion(self, name, character).await
    }
    async fn patch_companion(
        &self,
        companion_id: &str,
        patch: serde_json::Value,
    ) -> Result<CompanionProfileConfig, AppError> {
        CompanionService::patch_companion(self, companion_id, patch).await
    }
    async fn remove_companion(&self, companion_id: &str) -> Result<(), AppError> {
        CompanionService::delete_companion(self, companion_id).await
    }
}

#[async_trait::async_trait]
impl CompanionRoster for CompanionRegistry {
    async fn list_companions(&self) -> Vec<CompanionProfileConfig> {
        self.list().await
    }
    async fn create_companion(&self, name: &str, character: &str) -> Result<CompanionProfileConfig, AppError> {
        self.create(name, character).await
    }
    async fn patch_companion(
        &self,
        companion_id: &str,
        patch: serde_json::Value,
    ) -> Result<CompanionProfileConfig, AppError> {
        self.patch(companion_id, patch).await
    }
    async fn remove_companion(&self, companion_id: &str) -> Result<(), AppError> {
        self.remove(companion_id).await.map(|_| ())
    }
}

// ── Export ──────────────────────────────────────────────────────────

/// Package the shared memory hub (memories + mood, optionally the raw event
/// day files) into a zip at `dest_path`, written atomically via
/// a same-directory tempfile + persist.
pub async fn export_memory_bundle(
    store: &CompanionStore,
    shared_dir: &Path,
    dest_path: &Path,
    include_events: bool,
) -> Result<ExportSummary, AppError> {
    if !dest_path.is_absolute() {
        return Err(AppError::BadRequest("dest_path must be absolute".into()));
    }
    let memories = store.dump_memories_all().await?;
    let mood = store.get_state("mood").await?;

    let dest = dest_path.to_path_buf();
    let events_dir = include_events.then(|| shared_dir.join("events"));
    let memories_count = memories.len() as u64;
    let (file_count, total_bytes, event_files) = tokio::task::spawn_blocking(move || {
        atomic_zip(&dest, |zip| {
            let mut total_bytes = 0u64;
            add_json_entry(zip, "manifest.json", &manifest_for(EXPORT_KIND_MEMORY))?;
            total_bytes += add_jsonl_entry(zip, "memories.jsonl", &memories)?;
            // v3 importers require this entry. It intentionally contains no
            // rows now that learning-run history has been retired.
            add_raw_entry(zip, "learn_runs.jsonl", &[])?;
            total_bytes += add_json_entry(
                zip,
                "state.json",
                &MemoryStatePayload {
                    mood: RequiredOptionalString(mood),
                },
            )?;
            let mut event_files = 0u64;
            if let Some(events_dir) = events_dir {
                let mut files = match std::fs::read_dir(&events_dir) {
                    Ok(entries) => entries
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|error| {
                            AppError::Internal(format!(
                                "failed to scan event directory {}: {error}",
                                events_dir.display()
                            ))
                        })?,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(error) => {
                        return Err(AppError::Internal(format!(
                            "failed to read event directory {}: {error}",
                            events_dir.display()
                        )));
                    }
                };
                let mut files = files
                    .drain(..)
                    .map(|entry| {
                        let file_type = entry.file_type().map_err(|error| {
                            AppError::Internal(format!(
                                "failed to inspect event entry {}: {error}",
                                entry.path().display()
                            ))
                        })?;
                        if !file_type.is_file() {
                            return Err(AppError::Internal(format!(
                                "event directory contains non-regular entry {}",
                                entry.path().display()
                            )));
                        }
                        let name = entry.file_name().into_string().map_err(|_| {
                            AppError::Internal(format!(
                                "event directory contains a non-UTF8 file name: {}",
                                entry.path().display()
                            ))
                        })?;
                        if !name.ends_with(".jsonl") {
                            return Err(AppError::Internal(format!(
                                "event directory contains unsupported file {name:?}"
                            )));
                        }
                        Ok(entry.path())
                    })
                    .collect::<Result<Vec<_>, AppError>>()?;
                files.sort();
                for path in files {
                    crate::collector::validate_event_file(&path)?;
                    let name = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .ok_or_else(|| AppError::Internal("event file name became non-UTF8".into()))?;
                    let bytes = std::fs::read(&path)
                        .map_err(|e| AppError::Internal(format!("failed to read event file {name}: {e}")))?;
                    add_raw_entry(zip, &format!("events/{name}"), &bytes)?;
                    total_bytes += bytes.len() as u64;
                    event_files += 1;
                }
            }
            Ok((3 + event_files, total_bytes, event_files))
        })
    })
    .await
    .map_err(|e| AppError::Internal(format!("export task join error: {e}")))??;

    Ok(ExportSummary {
        kind: EXPORT_KIND_MEMORY.to_owned(),
        file_count,
        total_bytes,
        dest_path: dest_path.to_string_lossy().to_string(),
        memories: memories_count,
        learn_runs: 0,
        event_files,
    })
}

/// Package one companion (full profile + per-companion xp + knowledge binding names) into
/// a zip at `dest_path`. `knowledge_names` is supplied by the caller — the
/// binding list crosses domains and is collected on the frontend.
pub async fn export_companion_bundle(
    store: &CompanionStore,
    profile: &CompanionProfileConfig,
    dest_path: &Path,
    knowledge_names: &[String],
) -> Result<ExportSummary, AppError> {
    if !dest_path.is_absolute() {
        return Err(AppError::BadRequest("dest_path must be absolute".into()));
    }
    let xp = store.get_companion_state_i64(&profile.companion_id, "xp").await?;

    let dest = dest_path.to_path_buf();
    let profile = profile.clone();
    let refs = KnowledgeRefsPayload {
        names: knowledge_names.to_vec(),
    };
    let (file_count, total_bytes) = tokio::task::spawn_blocking(move || {
        atomic_zip(&dest, |zip| {
            let mut total_bytes = 0u64;
            add_json_entry(zip, "manifest.json", &manifest_for(EXPORT_KIND_COMPANION))?;
            total_bytes += add_json_entry(zip, "companion.json", &profile)?;
            total_bytes += add_json_entry(zip, "state.json", &CompanionStatePayload { xp })?;
            total_bytes += add_json_entry(zip, "knowledge_refs.json", &refs)?;
            Ok((3u64, total_bytes))
        })
    })
    .await
    .map_err(|e| AppError::Internal(format!("export task join error: {e}")))??;

    Ok(ExportSummary {
        kind: EXPORT_KIND_COMPANION.to_owned(),
        file_count,
        total_bytes,
        dest_path: dest_path.to_string_lossy().to_string(),
        memories: 0,
        learn_runs: 0,
        event_files: 0,
    })
}

/// Atomic zip write: parent dirs created, payload written to a securely-created
/// same-directory tempfile, fsynced, then persisted into place. A failed export
/// never leaves a half-written package behind.
fn atomic_zip<T>(
    dest: &Path,
    write: impl FnOnce(&mut zip::ZipWriter<std::fs::File>) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let parent = dest.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| AppError::Internal(format!("failed to create export dir: {e}")))?;
    let temp = tempfile::Builder::new()
        .prefix(".nomifun-export.")
        .tempfile_in(parent)
        .map_err(|e| AppError::Internal(format!("failed to create export tempfile: {e}")))?;
    let file = temp
        .reopen()
        .map_err(|e| AppError::Internal(format!("failed to reopen export tempfile: {e}")))?;
    let mut zip = zip::ZipWriter::new(file);
    let out = write(&mut zip)?;
    let file = zip
        .finish()
        .map_err(|e| AppError::Internal(format!("failed to write zip: {e}")))?;
    file.sync_all()
        .map_err(|e| AppError::Internal(format!("failed to fsync export file: {e}")))?;
    drop(file);
    temp.persist(dest)
        .map_err(|error| AppError::Internal(format!("failed to finalize export file: {}", error.error)))?;
    #[cfg(unix)]
    {
        std::fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| AppError::Internal(format!("failed to fsync export directory: {e}")))?;
    }
    Ok(out)
}

/// Pretty-printed JSON entry; returns the payload size in bytes.
fn add_json_entry(
    zip: &mut zip::ZipWriter<std::fs::File>,
    name: &str,
    value: &impl Serialize,
) -> Result<u64, AppError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| AppError::Internal(e.to_string()))?;
    add_raw_entry(zip, name, &bytes)?;
    Ok(bytes.len() as u64)
}

/// One JSON object per line; returns the payload size in bytes.
fn add_jsonl_entry(
    zip: &mut zip::ZipWriter<std::fs::File>,
    name: &str,
    rows: &[impl Serialize],
) -> Result<u64, AppError> {
    let mut buf = Vec::new();
    for row in rows {
        serde_json::to_writer(&mut buf, row).map_err(|e| AppError::Internal(e.to_string()))?;
        buf.push(b'\n');
    }
    add_raw_entry(zip, name, &buf)?;
    Ok(buf.len() as u64)
}

fn add_raw_entry(zip: &mut zip::ZipWriter<std::fs::File>, name: &str, bytes: &[u8]) -> Result<(), AppError> {
    zip.start_file(name, zip::write::SimpleFileOptions::default())
        .map_err(|e| AppError::Internal(format!("failed to write zip: {e}")))?;
    zip.write_all(bytes)
        .map_err(|e| AppError::Internal(format!("failed to package {name}: {e}")))?;
    Ok(())
}

// ── Import ──────────────────────────────────────────────────────────

/// Test-only import entry point without the production event-store gate. Live
/// callers must use [`import_bundle_with_event_lock`] through the service so
/// the configured hard capacity remains an invariant.
#[cfg(test)]
pub async fn import_bundle(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    shared_dir: &Path,
    src_path: &Path,
) -> Result<ImportOutcome, AppError> {
    import_bundle_inner(store, roster, shared_dir, src_path, None, None).await
}

/// Production import variant. ZIP extraction and package parsing happen
/// outside the gate; memory-event destination preflight, publication and
/// rollback are serialized with the collector. The current capacity policy is
/// read only after taking that gate, and an over-cap import is rejected before
/// either event files or memory rows are published.
pub(crate) async fn import_bundle_with_event_lock(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    shared_dir: &Path,
    src_path: &Path,
    event_store_lock: crate::collector::SharedEventStoreLock,
    config: crate::collector::SharedConfig,
) -> Result<ImportOutcome, AppError> {
    import_bundle_inner(
        store,
        roster,
        shared_dir,
        src_path,
        Some(event_store_lock),
        Some(config),
    )
    .await
}

async fn import_bundle_inner(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    shared_dir: &Path,
    src_path: &Path,
    event_store_lock: Option<crate::collector::SharedEventStoreLock>,
    config: Option<crate::collector::SharedConfig>,
) -> Result<ImportOutcome, AppError> {
    if !src_path.is_file() {
        return Err(AppError::BadRequest(format!(
            "import file does not exist: {}",
            src_path.display()
        )));
    }

    // Extraction temp lives under the shared dir (same volume as the events
    // destination), namespaced to avoid collisions.
    let tmp_root = shared_dir.join(".import-tmp");
    let extract_dir = tmp_root.join(format!("companion-{}-{}", std::process::id(), now_ms()));
    tokio::fs::create_dir_all(&extract_dir)
        .await
        .map_err(|e| AppError::Internal(format!("failed to create import temp dir: {e}")))?;

    let result = import_extracted(
        store,
        roster,
        shared_dir,
        src_path,
        &extract_dir,
        event_store_lock.as_ref(),
        config.as_ref(),
    )
    .await;
    let _ = tokio::fs::remove_dir_all(&extract_dir).await;
    let _ = tokio::fs::remove_dir(&tmp_root).await; // best-effort, only when empty
    result
}

async fn import_extracted(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    shared_dir: &Path,
    src_path: &Path,
    extract_dir: &Path,
    event_store_lock: Option<&crate::collector::SharedEventStoreLock>,
    config: Option<&crate::collector::SharedConfig>,
) -> Result<ImportOutcome, AppError> {
    let src = src_path.to_path_buf();
    let dest = extract_dir.to_path_buf();
    let kind = tokio::task::spawn_blocking(move || extract_zip_validated(&src, &dest))
        .await
        .map_err(|e| AppError::Internal(format!("import task join error: {e}")))??;

    match kind.as_str() {
        EXPORT_KIND_MEMORY => {
            import_memory_bundle(
                store,
                roster,
                shared_dir,
                extract_dir,
                event_store_lock,
                config,
            )
            .await
        }
        EXPORT_KIND_COMPANION => import_companion_bundle(store, roster, extract_dir).await,
        other => Err(AppError::BadRequest(format!("导入包类型不支持: {other}"))),
    }
}

/// Re-home every imported memory onto a LOCAL owner, in place.
///
/// Companion ids are never stable across machines — `import_companion_bundle`
/// allocates a fresh id — so after a cross-machine transfer every exported
/// private row carries an owner that means nothing here. This is BOOT-CRITICAL:
/// `CompanionStore::validate_companion_references` hard-fails startup on any
/// orphaned companion reference and runs unconditionally from
/// `CompanionService::new`, so importing a bundle with a foreign owner would
/// brick the next launch.
///
/// - owner that IS a live local companion → kept verbatim
/// - foreign owner, or a vestigial unowned row → the resolved local owner
/// - empty roster (no legal owner at all) → parked as unowned (`('user', NULL)`),
///   which stays legal at the DB level and gets re-homed by the boot migration as
///   soon as a companion exists
///
/// Nothing is ever dropped: an imported memory always survives, the only
/// question is whose it becomes.
fn rehome_imported_memories(
    memories: &mut [CompanionMemory],
    roster: &[CompanionProfileConfig],
    default_companion_id: Option<&str>,
) {
    let live: HashSet<&str> = roster.iter().map(|p| p.companion_id.as_str()).collect();
    let local_owner = crate::registry::memory_owner_of(roster.iter(), default_companion_id);
    for memory in memories {
        let owned_locally = memory
            .scope_companion_id
            .as_deref()
            .is_some_and(|owner| live.contains(owner));
        if owned_locally {
            continue;
        }
        match &local_owner {
            Some(owner) => {
                memory.scope_kind = "companion".to_owned();
                memory.scope_companion_id = Some(owner.clone());
            }
            None => {
                // No legal owner yet — park it rather than lose it or brick boot.
                memory.scope_kind = "user".to_owned();
                memory.scope_companion_id = None;
            }
        }
    }
}

/// Merge a memory bundle into the local store. Both jsonl files are parsed
/// fully before a SQLite transaction is opened. Event files are also
/// preflighted before the transaction: a same-name local file must have both
/// the same SHA-256 and the same bytes. New event files are published with
/// no-clobber hard links while the transaction remains uncommitted; any
/// publication failure rolls back both the DB rows and files created by this
/// attempt. The packaged mood is deliberately ignored.
async fn import_memory_bundle(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    shared_dir: &Path,
    extract_dir: &Path,
    event_store_lock: Option<&crate::collector::SharedEventStoreLock>,
    config: Option<&crate::collector::SharedConfig>,
) -> Result<ImportOutcome, AppError> {
    let mut memories =
        parse_jsonl::<CompanionMemory>(&extract_dir.join("memories.jsonl"), "memories.jsonl", true)?;
    let default_companion_id = match config {
        Some(config) => config.read().await.default_companion_id.clone(),
        None => None,
    };
    rehome_imported_memories(
        &mut memories,
        &roster.list_companions().await,
        default_companion_id.as_deref(),
    );
    let legacy_learn_runs =
        parse_jsonl::<LegacyLearnRun>(&extract_dir.join("learn_runs.jsonl"), "learn_runs.jsonl", true)?;
    for run in &legacy_learn_runs {
        validate_uuidv7(&run.learn_run_id)
            .map_err(|error| AppError::BadRequest(format!("invalid legacy learn-run id: {error}")))?;
    }
    let _state: MemoryStatePayload = read_json_strict(&extract_dir.join("state.json"), "state.json")?;
    let _event_guard = match event_store_lock {
        Some(lock) => Some(lock.write().await),
        None => None,
    };
    let storage_policy = match config {
        Some(config) => Some(config.read().await.clone()),
        None => None,
    };
    let event_plan = plan_event_import(&extract_dir.join("events"), &shared_dir.join("events"))?;
    if let Some(config) = &storage_policy {
        ensure_event_import_fits_capacity(
            shared_dir,
            &event_plan,
            config.collect.event_max_storage_mb,
        )?;
    }

    let transaction = store.begin_memory_import(&memories).await?;
    let stats = transaction.stats();
    let published = match publish_event_import(&event_plan) {
        Ok(published) => published,
        Err(error) => {
            transaction.rollback().await?;
            return Err(error);
        }
    };
    match transaction.commit().await {
        Ok(_) => {
            // Keep the event-store write gate through the immediate retention
            // pass. Cleanup is maintenance after an already-committed import:
            // it is deliberately best-effort so callers never receive a
            // failure after both memory rows and event files became durable.
            if let Some(config) = &storage_policy {
                match crate::collector::active_consumer_watermark(store, config).await {
                    Ok(protected_after_ts) => {
                        if let Err(error) = crate::collector::prune_event_store(
                            shared_dir,
                            config.collect.event_retention_days,
                            config.collect.event_max_storage_mb,
                            protected_after_ts,
                            0,
                        ) {
                            tracing::warn!(
                                error = %error,
                                "companion event cleanup after committed memory import failed; will retry"
                            );
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            "companion event cleanup could not read consumer cursors after committed memory import; will retry"
                        );
                    }
                }
            }
            Ok(ImportOutcome::Memory {
                imported: stats.imported,
                skipped_duplicates: stats.skipped_duplicates,
            })
        }
        Err(error) => {
            published.rollback();
            Err(error)
        }
    }
}

/// Recreate a packaged companion through the live roster: `create` (validated name,
/// deduplicated against existing companions) + `patch` (persona/model/appearance),
/// then the per-companion xp. Any failure after creation rolls the new companion back.
async fn import_companion_bundle(
    store: &CompanionStore,
    roster: &dyn CompanionRoster,
    extract_dir: &Path,
) -> Result<ImportOutcome, AppError> {
    let companion_bytes = std::fs::read(extract_dir.join("companion.json"))
        .map_err(|_| AppError::BadRequest("导出包缺少 companion.json".into()))?;
    let profile: CompanionProfileConfig =
        serde_json::from_slice(&companion_bytes).map_err(|e| AppError::BadRequest(format!("companion.json 无法解析: {e}")))?;
    nomifun_common::CompanionId::try_from(profile.companion_id.as_str())
        .map_err(|error| AppError::BadRequest(format!("companion.json companion_id 无效: {error}")))?;
    if profile.seq == 0 {
        return Err(AppError::BadRequest("companion.json seq 必须大于 0".into()));
    }
    let state: CompanionStatePayload = read_json_strict(&extract_dir.join("state.json"), "state.json")?;
    let refs: KnowledgeRefsPayload =
        read_json_strict(&extract_dir.join("knowledge_refs.json"), "knowledge_refs.json")?;

    let existing: HashSet<String> = roster.list_companions().await.into_iter().map(|p| p.name).collect();
    let base_name = match profile.name.trim() {
        "" => "导入的伙伴",
        name => name,
    };
    let final_name = dedup_name(&existing, base_name);

    let created = roster.create_companion(&final_name, &profile.character).await?;
    let setup = async {
        roster
            .patch_companion(
                &created.companion_id,
                serde_json::json!({
                    "persona": profile.persona,
                    "model": profile.model,
                    "appearance": profile.appearance,
                }),
            )
            .await?;
        if state.xp != 0 {
            store.set_companion_state(&created.companion_id, "xp", &state.xp.to_string()).await?;
        }
        Ok::<(), AppError>(())
    }
    .await;
    if let Err(e) = setup {
        // Roll back the half-imported companion; a failed rollback only warns.
        if let Err(cleanup) = store.delete_companion_rows(&created.companion_id).await {
            tracing::warn!(
                companion_id = %created.companion_id,
                error = %cleanup,
                "rollback of failed companion import left stale store rows"
            );
        }
        if let Err(del) = roster.remove_companion(&created.companion_id).await {
            tracing::warn!(companion_id = %created.companion_id, error = %del, "rollback of failed companion import left a stale companion");
        }
        return Err(e);
    }

    Ok(ImportOutcome::Companion {
        companion_id: created.companion_id,
        name: final_name,
        knowledge_names: refs.names,
    })
}

/// Parse one jsonl file into rows, strictly: any malformed line fails the
/// whole import before anything was written. `required` distinguishes a
/// mandatory file (missing → BadRequest) from an optional one (missing →
/// empty).
fn parse_jsonl<T: serde::de::DeserializeOwned>(
    path: &Path,
    label: &str,
    required: bool,
) -> Result<Vec<T>, AppError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {
            return Ok(Vec::new());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::BadRequest(format!("导出包缺少 {label}")));
        }
        Err(error) => {
            return Err(AppError::Internal(format!("检查 {label} 失败: {error}")));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(AppError::BadRequest(format!("{label} 必须是普通文件")));
    }
    let raw = match std::fs::read(path) {
        Ok(raw) => raw,
        Err(error) => {
            return Err(AppError::Internal(format!("读取 {label} 失败: {error}")));
        }
    };
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    if !raw.is_empty() && !raw.ends_with(b"\n") {
        return Err(AppError::BadRequest(format!("{label} 末行不完整")));
    }
    let mut rows = Vec::new();
    let lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if index + 1 == lines.len() && raw.ends_with(b"\n") {
                continue;
            }
            return Err(AppError::BadRequest(format!(
                "{label} 第 {} 行为空记录",
                index + 1
            )));
        }
        let row: T = serde_json::from_slice(line)
            .map_err(|e| AppError::BadRequest(format!("{label} 第 {} 行无法解析: {e}", index + 1)))?;
        rows.push(row);
    }
    Ok(rows)
}

fn read_json_strict<T: serde::de::DeserializeOwned>(path: &Path, label: &str) -> Result<T, AppError> {
    let bytes = std::fs::read(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            AppError::BadRequest(format!("导出包缺少 {label}"))
        } else {
            AppError::Internal(format!("读取 {label} 失败: {error}"))
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|error| AppError::BadRequest(format!("{label} 无法解析: {error}")))
}

#[derive(Debug)]
struct EventImportPlan {
    source: PathBuf,
    target: PathBuf,
}

#[derive(Debug)]
struct PublishedEvents {
    targets: Vec<PathBuf>,
}

impl PublishedEvents {
    fn rollback(self) {
        let mut parents = std::collections::HashSet::new();
        for target in self.targets.into_iter().rev() {
            if let Some(parent) = target.parent() {
                parents.insert(parent.to_path_buf());
            }
            if let Err(error) = crate::fsio::remove_path_entry(&target) {
                tracing::warn!(path = %target.display(), %error, "failed to roll back imported event file");
            }
        }
        for parent in parents {
            if let Err(error) = crate::fsio::sync_dir(&parent) {
                tracing::warn!(path = %parent.display(), %error, "failed to fsync rolled-back event directory");
            }
        }
    }
}

/// Build a deterministic publication plan, strictly validate every imported
/// event JSONL file, and reject every existing same-name event whose digest or
/// bytes differ. Comparing bytes after SHA-256 avoids treating a theoretical
/// hash collision as identical content.
fn plan_event_import(package_dir: &Path, destination_dir: &Path) -> Result<Vec<EventImportPlan>, AppError> {
    if !package_dir.exists() {
        return Ok(Vec::new());
    }
    if !package_dir.is_dir() {
        return Err(AppError::BadRequest("events 必须是目录".into()));
    }

    let mut sources = std::fs::read_dir(package_dir)
        .map_err(|error| AppError::Internal(format!("读取导入 events 目录失败: {error}")))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| AppError::Internal(format!("读取导入 event 条目失败: {error}")))?;
    sources.sort_by_key(std::fs::DirEntry::file_name);

    let mut plan = Vec::new();
    for entry in sources {
        let file_type = entry
            .file_type()
            .map_err(|error| AppError::Internal(format!("检查导入 event 类型失败: {error}")))?;
        if !file_type.is_file() {
            return Err(AppError::BadRequest(format!(
                "events 包含非普通文件: {}",
                entry.file_name().to_string_lossy()
            )));
        }
        let source = entry.path();
        if source.extension().is_none_or(|extension| extension != "jsonl") {
            return Err(AppError::BadRequest(format!(
                "events 包含非 jsonl 文件: {}",
                entry.file_name().to_string_lossy()
            )));
        }
        crate::collector::validate_event_file(&source).map_err(|error| {
            AppError::BadRequest(format!(
                "导入 event 文件 {} 损坏: {error}",
                entry.file_name().to_string_lossy()
            ))
        })?;
        let target = destination_dir.join(entry.file_name());
        match std::fs::read(&target) {
            Ok(local) => {
                crate::collector::validate_event_file(&target)?;
                let imported = std::fs::read(&source)
                    .map_err(|error| AppError::Internal(format!("读取导入 event 文件失败: {error}")))?;
                if sha256_bytes(&local) != sha256_bytes(&imported) || local != imported {
                    return Err(AppError::Conflict(format!(
                        "event import conflict for {}: local and imported hash/content differ",
                        entry.file_name().to_string_lossy()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                plan.push(EventImportPlan { source, target });
            }
            Err(error) => {
                return Err(AppError::Internal(format!(
                    "读取本地 event 文件 {} 失败: {error}",
                    target.display()
                )));
            }
        }
    }
    Ok(plan)
}

fn ensure_event_import_fits_capacity(
    shared_dir: &Path,
    plan: &[EventImportPlan],
    max_storage_mb: u32,
) -> Result<(), AppError> {
    // A memory-only package must not inspect or be blocked by historical raw
    // event storage. There are no event bytes to reserve or publish.
    if plan.is_empty() {
        return Ok(());
    }
    let current_bytes = crate::collector::event_storage_total_bytes(shared_dir)?;
    let incoming_bytes = plan.iter().try_fold(0u64, |total, entry| {
        let bytes = std::fs::metadata(&entry.source)
            .map_err(|error| {
                AppError::Internal(format!(
                    "inspect imported event file {}: {error}",
                    entry.source.display()
                ))
            })?
            .len();
        total.checked_add(bytes).ok_or_else(|| {
            AppError::BadRequest("imported event files exceed the supported size range".into())
        })
    })?;
    let projected_bytes = current_bytes.checked_add(incoming_bytes).ok_or_else(|| {
        AppError::BadRequest("imported event files exceed the supported size range".into())
    })?;
    let max_bytes = u64::from(max_storage_mb) * 1024 * 1024;
    if projected_bytes > max_bytes {
        return Err(AppError::BadRequest(format!(
            "imported raw events would use {projected_bytes} bytes, exceeding the configured {max_storage_mb} MiB event-storage limit"
        )));
    }
    Ok(())
}

/// Publish staged event files without overwriting anything. A hard link is an
/// atomic no-clobber operation and keeps the extracted staging directory alive
/// until the import transaction is committed or rolled back.
fn publish_event_import(plan: &[EventImportPlan]) -> Result<PublishedEvents, AppError> {
    publish_event_import_with_sync(plan, crate::fsio::sync_dir)
}

fn publish_event_import_with_sync(
    plan: &[EventImportPlan],
    sync_destination: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<PublishedEvents, AppError> {
    publish_event_import_with_io(plan, sync_destination, |path| std::fs::read(path))
}

fn publish_event_import_with_io(
    plan: &[EventImportPlan],
    sync_destination: impl FnOnce(&Path) -> std::io::Result<()>,
    mut read_file: impl FnMut(&Path) -> std::io::Result<Vec<u8>>,
) -> Result<PublishedEvents, AppError> {
    let Some(destination_dir) = plan.first().and_then(|entry| entry.target.parent()) else {
        return Ok(PublishedEvents { targets: Vec::new() });
    };
    std::fs::create_dir_all(destination_dir)
        .map_err(|error| AppError::Internal(format!("创建 events 目录失败: {error}")))?;

    let mut published = PublishedEvents { targets: Vec::new() };
    for entry in plan {
        match std::fs::hard_link(&entry.source, &entry.target) {
            Ok(()) => published.targets.push(entry.target.clone()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let local = match read_file(&entry.target) {
                    Ok(local) => local,
                    Err(read_error) => {
                        published.rollback();
                        return Err(AppError::Internal(format!(
                            "读取并发创建的 event 文件 {} 失败: {read_error}",
                            entry.target.display()
                        )));
                    }
                };
                let imported = match read_file(&entry.source) {
                    Ok(imported) => imported,
                    Err(read_error) => {
                        published.rollback();
                        return Err(AppError::Internal(format!(
                            "读取导入 event 文件失败: {read_error}"
                        )));
                    }
                };
                if sha256_bytes(&local) != sha256_bytes(&imported) || local != imported {
                    published.rollback();
                    return Err(AppError::Conflict(format!(
                        "event import conflict for {}: local and imported hash/content differ",
                        entry.target.file_name().unwrap_or_default().to_string_lossy()
                    )));
                }
            }
            Err(error) => {
                published.rollback();
                return Err(AppError::Internal(format!(
                    "发布 event 文件 {} 失败: {error}",
                    entry.target.display()
                )));
            }
        }
    }
    if let Err(error) = sync_destination(destination_dir) {
        published.rollback();
        return Err(AppError::Internal(format!(
            "fsync imported events directory: {error}"
        )));
    }
    Ok(published)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Blocking extraction with validation. Only the documented package entries
/// are accepted (`manifest.json`, `memories.jsonl`, `learn_runs.jsonl`,
/// `state.json`, `companion.json`, `knowledge_refs.json`, `events/*.jsonl`); every
/// entry path is sanitized (zip-slip), symlink entries are rejected, and
/// decompression-bomb caps (entry count + cumulative actually-written bytes)
/// bound the extraction.
/// Returns the manifest `kind` after the format/version checks passed.
fn extract_zip_validated(archive_path: &Path, destination: &Path) -> Result<String, AppError> {
    let file = std::fs::File::open(archive_path)
        .map_err(|e| AppError::BadRequest(format!("failed to open import file: {e}")))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|_| AppError::BadRequest("不是 NomiFun 导出包".into()))?;

    let mut budget = zip_safe::ZipExtractionBudget::default();
    budget
        .check_entry_count(archive.len())
        .map_err(|e| AppError::BadRequest(e.to_string()))?;
    let mut seen_entries = HashSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| AppError::BadRequest(format!("corrupt zip archive: {e}")))?;
        let entry_name = entry.name().to_string();
        reject_zip_symlink(&entry, &entry_name)?;
        let rel = safe_zip_entry_path(&entry_name)?;
        if !seen_entries.insert(rel.clone()) {
            return Err(AppError::BadRequest(format!(
                "导出包包含重复条目: {entry_name}"
            )));
        }

        if entry.is_dir() {
            if rel != Path::new("events") {
                return Err(AppError::BadRequest(format!(
                    "不是 NomiFun 导出包（包含不支持的条目: {entry_name}）"
                )));
            }
            std::fs::create_dir_all(destination.join(&rel))
                .map_err(|e| AppError::Internal(format!("failed to extract dir: {e}")))?;
            continue;
        }

        let allowed = rel == Path::new("manifest.json")
            || rel == Path::new("memories.jsonl")
            || rel == Path::new("learn_runs.jsonl")
            || rel == Path::new("state.json")
            || rel == Path::new("companion.json")
            || rel == Path::new("knowledge_refs.json")
            || (rel.parent() == Some(Path::new("events")) && rel.extension().is_some_and(|ext| ext == "jsonl"));
        if !allowed {
            return Err(AppError::BadRequest(format!(
                "不是 NomiFun 导出包（包含不支持的条目: {entry_name}）"
            )));
        }

        let output_path = destination.join(&rel);
        // Defense in depth on top of component sanitization: the resolved
        // path must stay inside the extraction dir.
        if !output_path.starts_with(destination) {
            return Err(AppError::BadRequest(format!("非法压缩包条目: {entry_name}")));
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Internal(format!("failed to extract dirs: {e}")))?;
        }
        let mut output = std::fs::File::create(&output_path)
            .map_err(|e| AppError::Internal(format!("failed to extract file: {e}")))?;
        let written = std::io::copy(&mut entry, &mut output)
            .map_err(|e| AppError::Internal(format!("failed to extract file: {e}")))?;
        budget
            .record_written(written)
            .map_err(|e| AppError::BadRequest(e.to_string()))?;
    }

    let manifest_bytes = std::fs::read(destination.join("manifest.json"))
        .map_err(|_| AppError::BadRequest("不是 NomiFun 导出包".into()))?;
    let manifest: ExportManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| AppError::BadRequest(format!("manifest.json 无法解析: {error}")))?;
    validate_manifest(&manifest, destination)
}

/// Envelope and package-shape check. The version must be exactly 3:
/// missing/zero/lower and future versions all fail closed.
fn validate_manifest(manifest: &ExportManifest, destination: &Path) -> Result<String, AppError> {
    if manifest.format != EXPORT_FORMAT {
        return Err(AppError::BadRequest("不是 NomiFun 导出包".into()));
    }
    if manifest.version != EXPORT_VERSION {
        if manifest.version > EXPORT_VERSION {
            return Err(AppError::BadRequest("导入包版本过新，请升级应用".into()));
        }
        return Err(AppError::BadRequest("导入包版本过旧，必须使用精确 v3".into()));
    }
    let kind = match manifest.kind.as_str() {
        EXPORT_KIND_MEMORY | EXPORT_KIND_COMPANION => manifest.kind.clone(),
        other => return Err(AppError::BadRequest(format!("导入包类型不支持: {other}"))),
    };
    let present = |name: &str| destination.join(name).is_file();
    let valid_shape = match kind.as_str() {
        EXPORT_KIND_MEMORY => {
            present("memories.jsonl")
                && present("learn_runs.jsonl")
                && present("state.json")
                && !present("companion.json")
                && !present("knowledge_refs.json")
        }
        EXPORT_KIND_COMPANION => {
            present("companion.json")
                && present("state.json")
                && present("knowledge_refs.json")
                && !present("memories.jsonl")
                && !present("learn_runs.jsonl")
                && !destination.join("events").exists()
        }
        _ => false,
    };
    if !valid_shape {
        return Err(AppError::BadRequest(format!(
            "v3 {kind} 导出包文件集合不完整或包含错误条目"
        )));
    }
    Ok(kind)
}

/// Sanitize a zip entry name into a safe relative path via the shared
/// [`nomifun_common::zip_safe`] hardening. Companion bundles use the strict
/// colon policy — every `':'` byte is rejected, portably covering Windows
/// drive prefixes ("C:/…") which parse as `Component::Prefix` only on
/// Windows. (Our own exporter never writes a `':'` into an entry name.)
fn safe_zip_entry_path(name: &str) -> Result<PathBuf, AppError> {
    zip_safe::safe_zip_entry_path(name, zip_safe::ZipColonPolicy::RejectAll)
        .ok_or_else(|| AppError::BadRequest(format!("非法压缩包条目: {name}")))
}

fn reject_zip_symlink(entry: &zip::read::ZipFile<'_>, name: &str) -> Result<(), AppError> {
    if zip_safe::zip_entry_is_symlink(entry.unix_mode()) {
        return Err(AppError::BadRequest(format!("非法压缩包条目: {name}")));
    }
    Ok(())
}

/// Suffix `name` with `" (2)"`, `" (3)"`, … until it no longer collides
/// with an existing companion name.
fn dedup_name(existing: &HashSet<String>, name: &str) -> String {
    if !existing.contains(name) {
        return name.to_owned();
    }
    for n in 2u32.. {
        let candidate = format!("{name} ({n})");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u32 suffix space exhausted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CompanionRegistry;

    fn memory_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::CompanionMemoryId::try_from(raw.as_str()).unwrap().into_string()
    }

    /// A canonical companion id that does NOT exist in the local roster — what an
    /// exported private memory from another machine carries.
    fn companion_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::CompanionId::try_from(raw.as_str()).unwrap().into_string()
    }

    fn provider_fixture(sequence: u64) -> String {
        let raw = format!("0190f5fe-7c00-7a00-8abc-{sequence:012}");
        nomifun_common::ProviderId::try_from(raw.as_str()).unwrap().into_string()
    }

    /// Registry over `{root}/{companions}` with its seq-watermark state beside it
    /// at `{root}/{companions}-shared` (each test roster gets its own watermark).
    fn scan_registry(root: &Path, companions: &str) -> CompanionRegistry {
        CompanionRegistry::scan(
            root.join(companions),
            root.join(format!("{companions}-shared")),
        )
        .unwrap()
    }

    fn raw_memory(memory_id: &str, kind: &str, content: &str, status: &str) -> CompanionMemory {
        CompanionMemory {
            memory_id: memory_id.to_owned(),
            kind: kind.to_owned(),
            content: content.to_owned(),
            tags: vec!["标签".into()],
            importance: 0.8,
            strength: 0.42,
            pinned: kind == "preference",
            source: "manual".into(),
            status: status.to_owned(),
            created_at: 1_111,
            updated_at: 2_222,
            last_reinforced_at: 3_333,
            scope_kind: "user".into(),
            scope_companion_id: None,
        }
    }

    fn write_test_zip(path: &Path, entries: &[(&str, &str)]) {
        let file = std::fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default();
        for (name, content) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(content.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }

    fn manifest_json(version: u32, kind: &str) -> String {
        format!(
            r#"{{"format":"nomifun-export","version":{version},"kind":"{kind}","exported_at":0,"app_version":"0.0.0"}}"#
        )
    }

    fn event_jsonl(ts: i64) -> String {
        let event = crate::collector::CollectedEvent {
            event_id: nomifun_common::generate_id(),
            ts,
            source: "chat_user_messages".into(),
            name: "message.userCreated".into(),
            data: serde_json::json!({"content": "import fixture"}),
        };
        format!("{}\n", serde_json::to_string(&event).unwrap())
    }

    fn sorted_json(memories: &mut Vec<CompanionMemory>) -> serde_json::Value {
        memories.sort_by(|a, b| a.memory_id.cmp(&b.memory_id));
        serde_json::to_value(&*memories).unwrap()
    }

    #[tokio::test]
    async fn memory_bundle_roundtrip_full_fidelity_and_dedup() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared_a = dir.path().join("shared-a");
        std::fs::create_dir_all(shared_a.join("events")).unwrap();
        let event_line = format!(
            r#"{{"event_id":"{}","ts":1,"source":"chat","name":"x","data":{{}}}}"#,
            nomifun_common::generate_id()
        );
        std::fs::write(shared_a.join("events").join("20260601.jsonl"), format!("{event_line}\n")).unwrap();

        let store_a = CompanionStore::open_memory().await.unwrap();
        let mut originals = vec![
            raw_memory(&memory_fixture(1), "preference", "主人喜欢深色主题", "active"),
            raw_memory(&memory_fixture(2), "episode", "上周修了导出 bug", "archived"),
            raw_memory(&memory_fixture(3), "knowledge", "cargo test -p nomifun-companion 是门禁", "active"),
        ];
        for m in &originals {
            store_a.insert_memory_raw(m).await.unwrap();
        }
        store_a.set_state("mood", "happy").await.unwrap();

        let zip_path = dir.path().join("out").join("memory.zip");
        let summary = export_memory_bundle(&store_a, &shared_a, &zip_path, true).await.unwrap();
        assert_eq!(summary.kind, "memory");
        assert_eq!(summary.memories, 3);
        assert_eq!(summary.learn_runs, 0);
        assert_eq!(summary.event_files, 1);
        assert_eq!(summary.file_count, 4);
        assert!(summary.total_bytes > 0);
        assert!(zip_path.is_file());
        assert!(
            !dir.path().join("out").join("memory.zip.tmp").exists(),
            "tmp must be renamed away"
        );
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(
            archive.by_name("learn_runs.jsonl").unwrap().size(),
            0,
            "the v3 compatibility marker must never contain history rows"
        );

        // Import into a fresh machine: full fidelity, mood untouched.
        let shared_b = dir.path().join("shared-b");
        let store_b = CompanionStore::open_memory().await.unwrap();
        store_b.set_state("mood", "calm").await.unwrap();
        let roster_b = scan_registry(dir.path(), "companions-b");
        let outcome = import_bundle(&store_b, &roster_b, &shared_b, &zip_path).await.unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Memory {
                imported: 3,
                skipped_duplicates: 0
            }
        );

        let mut restored = store_b.dump_memories_all().await.unwrap();
        assert_eq!(sorted_json(&mut restored), sorted_json(&mut originals));
        assert_eq!(store_b.get_state("mood").await.unwrap().as_deref(), Some("calm"));
        let landed = shared_b.join("events").join("20260601.jsonl");
        assert_eq!(std::fs::read_to_string(&landed).unwrap(), format!("{event_line}\n"));

        // Re-import with byte-identical events: the archived row and event file
        // are idempotently skipped.
        let outcome = import_bundle(&store_b, &roster_b, &shared_b, &zip_path).await.unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Memory {
                imported: 0,
                skipped_duplicates: 3
            }
        );
        assert_eq!(store_b.dump_memories_all().await.unwrap().len(), 3);

        // A same-name event is never silently preferred. Different hash or
        // bytes is a hard conflict and leaves both DB and local file unchanged.
        let local_event = format!(
            "{{\"event_id\":\"{}\",\"ts\":2,\"source\":\"chat\",\"name\":\"local\",\"data\":{{}}}}\n",
            nomifun_common::generate_id()
        );
        std::fs::write(&landed, &local_event).unwrap();
        let error = import_bundle(&store_b, &roster_b, &shared_b, &zip_path)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("event import conflict"), "{error}");
        assert_eq!(store_b.dump_memories_all().await.unwrap().len(), 3);
        assert_eq!(std::fs::read_to_string(&landed).unwrap(), local_event);
    }

    /// BOOT-CRITICAL: companion ids are not stable across machines, so every
    /// imported memory must end up owned by a LIVE local companion (or parked as
    /// unowned when there is none). A surviving foreign owner would make
    /// `validate_companion_references` hard-fail the next launch.
    #[tokio::test]
    async fn memory_import_rehomes_foreign_and_unowned_rows_onto_the_local_owner() {
        let dir = tempfile::TempDir::new().unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let oldest = roster.create_companion("甲", "ink").await.unwrap().companion_id;
        let second = roster.create_companion("乙", "ink").await.unwrap().companion_id;

        // Three rows: one owned by a companion that only exists on the SOURCE
        // machine, one vestigial unowned row, one owned by a live local companion.
        let foreign_owner = companion_fixture(999);
        let mut foreign = raw_memory(&memory_fixture(11), "knowledge", "源机器私有记忆", "active");
        foreign.scope_kind = "companion".into();
        foreign.scope_companion_id = Some(foreign_owner.clone());
        let unowned = raw_memory(&memory_fixture(12), "preference", "源机器共享记忆", "active");
        let mut local = raw_memory(&memory_fixture(13), "episode", "本机伙伴乙的记忆", "archived");
        local.scope_kind = "companion".into();
        local.scope_companion_id = Some(second.clone());
        let rows: String = [&foreign, &unowned, &local]
            .iter()
            .map(|m| format!("{}\n", serde_json::to_string(m).unwrap()))
            .collect();

        let archive = dir.path().join("foreign-memory.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &rows),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );

        let store = CompanionStore::open_memory().await.unwrap();
        let outcome = import_bundle(&store, &roster, &dir.path().join("shared"), &archive)
            .await
            .unwrap();
        assert_eq!(outcome, ImportOutcome::Memory { imported: 3, skipped_duplicates: 0 });

        // Nothing was dropped, and every row now has a local owner.
        let restored = store.dump_memories_all().await.unwrap();
        assert_eq!(restored.len(), 3);
        let owner_of = |memory_id: &str| {
            restored
                .iter()
                .find(|m| m.memory_id == memory_id)
                .map(|m| (m.scope_kind.clone(), m.scope_companion_id.clone()))
                .unwrap()
        };
        assert_eq!(
            owner_of(&memory_fixture(11)),
            ("companion".to_owned(), Some(oldest.clone())),
            "a foreign owner must be re-homed onto the resolved local owner"
        );
        assert_eq!(
            owner_of(&memory_fixture(12)),
            ("companion".to_owned(), Some(oldest.clone())),
            "an unowned row must be re-homed too"
        );
        assert_eq!(
            owner_of(&memory_fixture(13)),
            ("companion".to_owned(), Some(second.clone())),
            "a row already owned by a live local companion is kept verbatim"
        );

        // The whole point: the next boot's reference audit passes.
        let live: HashSet<String> = [oldest, second].into_iter().collect();
        store.validate_companion_references(&live).await.unwrap();
    }

    /// 共享记忆删除后，导入去重必须按主人算。一条要落到甲名下的导入记忆，不能因为
    /// **乙**恰好有一条相似记忆就被当成重复静默丢掉 —— 甲永远拿不到它，而那条"重复"
    /// 的记忆根本不属于甲。这与写入路径的 `find_similar_active` 是同一条规则。
    #[tokio::test]
    async fn memory_import_dedup_is_owner_scoped_not_install_wide() {
        let dir = tempfile::TempDir::new().unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let oldest = roster.create_companion("甲", "ink").await.unwrap().companion_id;
        let other = roster.create_companion("乙", "ink").await.unwrap().companion_id;

        let store = CompanionStore::open_memory().await.unwrap();
        // 乙 already holds this exact fact. It is 乙's, and only 乙's.
        let mut owned_by_other = raw_memory(&memory_fixture(41), "preference", "主人喜欢深烘焙咖啡豆", "active");
        owned_by_other.scope_kind = "companion".into();
        owned_by_other.scope_companion_id = Some(other.clone());
        store.insert_memory_raw(&owned_by_other).await.unwrap();

        // The bundle carries the same fact from another machine; it re-homes onto 甲.
        let incoming = raw_memory(&memory_fixture(42), "preference", "主人喜欢深烘焙咖啡豆", "active");
        let archive = dir.path().join("cross-owner-memory.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &format!("{}\n", serde_json::to_string(&incoming).unwrap())),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );
        let outcome = import_bundle(&store, &roster, &dir.path().join("shared"), &archive)
            .await
            .unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Memory { imported: 1, skipped_duplicates: 0 },
            "another companion's similar memory must not swallow this import"
        );
        let restored = store.dump_memories_all().await.unwrap();
        assert_eq!(restored.len(), 2, "both owners keep their own copy: {restored:?}");
        let landed = restored.iter().find(|m| m.memory_id == memory_fixture(42)).unwrap();
        assert_eq!(landed.scope_companion_id.as_deref(), Some(oldest.as_str()));

        // Re-importing the same bundle IS a duplicate (same id, same re-homed
        // shape), so idempotency is untouched by the owner scoping.
        let again = import_bundle(&store, &roster, &dir.path().join("shared"), &archive)
            .await
            .unwrap();
        assert_eq!(again, ImportOutcome::Memory { imported: 0, skipped_duplicates: 1 });
        assert_eq!(store.dump_memories_all().await.unwrap().len(), 2);
    }

    /// With no companion at all there is no legal owner: an imported private row
    /// is PARKED as unowned rather than kept foreign (which would brick boot) or
    /// dropped (which would lose the memory). The boot migration re-homes it as
    /// soon as a companion exists.
    #[tokio::test]
    async fn memory_import_parks_rows_as_unowned_when_the_roster_is_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let mut foreign = raw_memory(&memory_fixture(21), "knowledge", "源机器私有记忆", "active");
        foreign.scope_kind = "companion".into();
        foreign.scope_companion_id = Some(companion_fixture(998));
        let archive = dir.path().join("orphan-memory.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &format!("{}\n", serde_json::to_string(&foreign).unwrap())),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );

        let store = CompanionStore::open_memory().await.unwrap();
        import_bundle(&store, &roster, &dir.path().join("shared"), &archive)
            .await
            .unwrap();
        let restored = store.dump_memories_all().await.unwrap();
        assert_eq!(restored.len(), 1, "the memory must survive");
        assert_eq!(restored[0].scope_kind, "user");
        assert_eq!(restored[0].scope_companion_id, None);
        store.validate_companion_references(&HashSet::new()).await.unwrap();
    }

    /// A bundle written without the scope fields at all still imports (the two
    /// fields default to the vestigial unowned pair instead of failing
    /// `deny_unknown_fields`/missing-field validation).
    #[test]
    fn memory_rows_without_scope_fields_still_parse() {
        let row = serde_json::json!({
            "memory_id": memory_fixture(31),
            "kind": "preference",
            "content": "没有 scope 字段的旧包",
            "tags": [],
            "importance": 0.8,
            "strength": 0.8,
            "pinned": false,
            "source": "manual",
            "status": "active",
            "created_at": 1,
            "updated_at": 1,
            "last_reinforced_at": 1
        });
        let memory: CompanionMemory = serde_json::from_value(row).unwrap();
        assert_eq!(memory.scope_kind, "user");
        assert_eq!(memory.scope_companion_id, None);
    }

    #[tokio::test]
    async fn legacy_memory_bundle_validates_then_discards_learn_history() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let archive = dir.path().join("legacy-memory.zip");
        let memory_id = memory_fixture(77);
        let legacy_memory = raw_memory(&memory_id, "preference", "旧备份中的有效记忆", "active");
        let memory_row = format!("{}\n", serde_json::to_string(&legacy_memory).unwrap());
        let legacy_row = format!(
            "{{\"learn_run_id\":\"{}\",\"started_at\":1,\"finished_at\":2,\"status\":\"ok\",\"events_processed\":3,\"memories_added\":1,\"suggestions_added\":1,\"error\":null,\"summary\":\"legacy diary\"}}\n",
            nomifun_common::generate_id()
        );
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &memory_row),
                ("learn_runs.jsonl", &legacy_row),
                ("state.json", r#"{"mood":null}"#),
            ],
        );

        let outcome = import_bundle(
            &store,
            &roster,
            &dir.path().join("shared"),
            &archive,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Memory {
                imported: 1,
                skipped_duplicates: 0,
            }
        );
        assert_eq!(
            store.get_memory(&memory_id).await.unwrap().unwrap().content,
            "旧备份中的有效记忆"
        );
        let retired_table_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'companion_learn_runs'",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        assert_eq!(retired_table_count, 0, "legacy rows must never recreate the retired table");
    }

    #[tokio::test]
    async fn memory_import_rejects_same_id_with_different_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared_a = dir.path().join("shared-a");
        let store_a = CompanionStore::open_memory().await.unwrap();
        let clashing_id = memory_fixture(10);
        store_a
            .insert_memory_raw(&raw_memory(&clashing_id, "knowledge", "来自源机器的知识", "active"))
            .await
            .unwrap();
        let zip_path = dir.path().join("clash.zip");
        export_memory_bundle(&store_a, &shared_a, &zip_path, false).await.unwrap();

        // Target machine already owns mem_clash with different content. Merge
        // must fail rather than silently changing global identity.
        let store_b = CompanionStore::open_memory().await.unwrap();
        store_b
            .insert_memory_raw(&raw_memory(&clashing_id, "knowledge", "本机完全不同的知识", "active"))
            .await
            .unwrap();
        let roster_b = scan_registry(dir.path(), "companions-b");
        let error = import_bundle(&store_b, &roster_b, &dir.path().join("shared-b"), &zip_path)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("memory import ID collision"));
        let all = store_b.dump_memories_all().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].memory_id, clashing_id);
    }

    #[test]
    fn parse_jsonl_rejects_partial_empty_and_unknown_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("memories.jsonl");
        let memory = raw_memory(&memory_fixture(20), "knowledge", "严格 JSONL", "active");
        let line = serde_json::to_string(&memory).unwrap();

        std::fs::write(&path, &line).unwrap();
        let error = parse_jsonl::<CompanionMemory>(&path, "memories.jsonl", true).unwrap_err();
        assert!(error.to_string().contains("末行不完整"), "{error}");

        std::fs::write(&path, format!("{line}\n\n")).unwrap();
        let error = parse_jsonl::<CompanionMemory>(&path, "memories.jsonl", true).unwrap_err();
        assert!(error.to_string().contains("为空记录"), "{error}");

        let mut unknown = serde_json::to_value(&memory).unwrap();
        unknown["retired_field"] = serde_json::json!(true);
        std::fs::write(&path, format!("{}\n", serde_json::to_string(&unknown).unwrap()))
            .unwrap();
        let error = parse_jsonl::<CompanionMemory>(&path, "memories.jsonl", true).unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
    }

    #[tokio::test]
    async fn companion_bundle_roundtrip_keeps_xp_suffixes_name_and_echoes_refs() {
        let dir = tempfile::TempDir::new().unwrap();
        let store_a = CompanionStore::open_memory().await.unwrap();
        let reg_a = scan_registry(dir.path(), "companions-a");
        let created = reg_a.create("毛球", "ink").await.unwrap();
        let provider_id = provider_fixture(1);
        let profile = reg_a
            .patch(
                &created.companion_id,
                serde_json::json!({
                    "persona": {"preset": "sassy", "custom": "喜欢用颜文字"},
                    "model": {"provider_id": provider_id, "model": "claude-fable-5"},
                    "appearance": {"companion_enabled": true, "companion_x": 7, "quiet_start": "22:00", "quiet_end": "08:00"}
                }),
            )
            .await
            .unwrap();
        store_a.add_companion_xp(&profile.companion_id, 57).await.unwrap();

        let zip_path = dir.path().join("companion.zip");
        let summary = export_companion_bundle(&store_a, &profile, &zip_path, &["库甲".into(), "库乙".into()])
            .await
            .unwrap();
        assert_eq!(summary.kind, "companion");
        assert_eq!(summary.file_count, 3);
        assert!(!dir.path().join("companion.zip.tmp").exists());

        // Target roster already has a companion with the same name.
        let store_b = CompanionStore::open_memory().await.unwrap();
        let reg_b = scan_registry(dir.path(), "companions-b");
        reg_b.create("毛球", "mochi").await.unwrap();

        let outcome = import_bundle(&store_b, &reg_b, &dir.path().join("shared-b"), &zip_path)
            .await
            .unwrap();
        let ImportOutcome::Companion {
            companion_id,
            name,
            knowledge_names,
        } = outcome
        else {
            panic!("expected companion outcome");
        };
        assert_eq!(name, "毛球 (2)");
        assert_eq!(knowledge_names, vec!["库甲".to_string(), "库乙".to_string()]);
        assert_ne!(
            companion_id, profile.companion_id,
            "imported companion gets a fresh companion_id"
        );

        let imported = reg_b.get(&companion_id).await.unwrap();
        assert_eq!(imported.name, "毛球 (2)");
        // A fresh local short number is allocated (the bundle's own seq is
        // ignored): "毛球" took 1 on this roster, so the import gets 2.
        assert_eq!(imported.seq, 2);
        assert_eq!(imported.character, "ink");
        assert_eq!(imported.persona, profile.persona);
        assert_eq!(imported.model, profile.model);
        assert_eq!(imported.appearance, profile.appearance);
        assert_eq!(store_b.get_companion_state_i64(&companion_id, "xp").await.unwrap(), 57);

        // Importing again suffixes further.
        let outcome = import_bundle(&store_b, &reg_b, &dir.path().join("shared-b"), &zip_path)
            .await
            .unwrap();
        let ImportOutcome::Companion { name, .. } = outcome else {
            panic!("expected companion outcome");
        };
        assert_eq!(name, "毛球 (3)");
    }

    #[tokio::test]
    async fn import_accepts_only_exact_v3_and_rejects_invalid_envelopes() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let shared = dir.path().join("shared");

        let wrong_format = dir.path().join("format.zip");
        write_test_zip(
            &wrong_format,
            &[
                (
                    "manifest.json",
                    r#"{"format":"other-export","version":3,"kind":"memory","exported_at":0,"app_version":"0.0.0"}"#,
                ),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &wrong_format).await.unwrap_err();
        assert!(err.to_string().contains("不是 NomiFun 导出包"), "{err}");

        let wrong_kind = dir.path().join("kind.zip");
        write_test_zip(&wrong_kind, &[("manifest.json", &manifest_json(3, "knowledge-base"))]);
        let err = import_bundle(&store, &roster, &shared, &wrong_kind).await.unwrap_err();
        assert!(err.to_string().contains("导入包类型不支持"), "{err}");

        let too_new = dir.path().join("future.zip");
        write_test_zip(&too_new, &[("manifest.json", &manifest_json(4, EXPORT_KIND_MEMORY))]);
        let err = import_bundle(&store, &roster, &shared, &too_new).await.unwrap_err();
        assert!(err.to_string().contains("导入包版本过新"), "{err}");

        for (name, manifest) in [
            (
                "missing-version",
                r#"{"format":"nomifun-export","kind":"memory","exported_at":0,"app_version":"0.0.0"}"#,
            ),
            (
                "zero-version",
                r#"{"format":"nomifun-export","version":0,"kind":"memory","exported_at":0,"app_version":"0.0.0"}"#,
            ),
            (
                "low-version",
                r#"{"format":"nomifun-export","version":2,"kind":"memory","exported_at":0,"app_version":"0.0.0"}"#,
            ),
        ] {
            let path = dir.path().join(format!("{name}.zip"));
            write_test_zip(
                &path,
                &[
                    ("manifest.json", manifest),
                    ("memories.jsonl", ""),
                    ("learn_runs.jsonl", ""),
                    ("state.json", r#"{"mood":null}"#),
                ],
            );
            let err = import_bundle(&store, &roster, &shared, &path).await.unwrap_err();
            assert!(
                err.to_string().contains("manifest.json") || err.to_string().contains("版本过旧"),
                "{name}: {err}"
            );
        }

        let unknown_manifest_field = dir.path().join("manifest-extra.zip");
        write_test_zip(
            &unknown_manifest_field,
            &[
                (
                    "manifest.json",
                    r#"{"format":"nomifun-export","version":3,"kind":"memory","exported_at":0,"app_version":"0.0.0","extra":true}"#,
                ),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &unknown_manifest_field)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");

        let not_zip = dir.path().join("garbage.zip");
        std::fs::write(&not_zip, "definitely not a zip").unwrap();
        let err = import_bundle(&store, &roster, &shared, &not_zip).await.unwrap_err();
        assert!(err.to_string().contains("不是 NomiFun 导出包"), "{err}");

        let missing = dir.path().join("missing.zip");
        let err = import_bundle(&store, &roster, &shared, &missing).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");

        // A memory package without memories.jsonl is rejected explicitly.
        let incomplete = dir.path().join("incomplete.zip");
        write_test_zip(
            &incomplete,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &incomplete).await.unwrap_err();
        assert!(err.to_string().contains("文件集合"), "{err}");
        assert_eq!(store.dump_memories_all().await.unwrap().len(), 0);
        assert!(roster.list().await.is_empty());
    }

    #[tokio::test]
    async fn import_requires_strict_state_and_knowledge_ref_schemas() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let shared = dir.path().join("shared");
        let profile = CompanionProfileConfig::new("严格伙伴", "ink", 1);
        let profile_json = serde_json::to_string(&profile).unwrap();

        let memory_state_missing = dir.path().join("memory-state-missing.zip");
        write_test_zip(
            &memory_state_missing,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
            ],
        );
        let error = import_bundle(&store, &roster, &shared, &memory_state_missing)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("文件集合"), "{error}");
        assert!(roster.list().await.is_empty());

        let memory_state_unknown = dir.path().join("memory-state-unknown.zip");
        write_test_zip(
            &memory_state_unknown,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null,"extra":true}"#),
            ],
        );
        let error = import_bundle(&store, &roster, &shared, &memory_state_unknown)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");

        let companion_state_missing = dir.path().join("companion-state-missing.zip");
        write_test_zip(
            &companion_state_missing,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_COMPANION)),
                ("companion.json", &profile_json),
                ("knowledge_refs.json", r#"{"names":[]}"#),
            ],
        );
        let error = import_bundle(&store, &roster, &shared, &companion_state_missing)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("文件集合"), "{error}");
        assert!(roster.list().await.is_empty());

        let companion_state_unknown = dir.path().join("companion-state-unknown.zip");
        write_test_zip(
            &companion_state_unknown,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_COMPANION)),
                ("companion.json", &profile_json),
                ("state.json", r#"{"xp":0,"extra":true}"#),
                ("knowledge_refs.json", r#"{"names":[]}"#),
            ],
        );
        let error = import_bundle(&store, &roster, &shared, &companion_state_unknown)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
        assert!(roster.list().await.is_empty());

        let refs_unknown = dir.path().join("knowledge-refs-unknown.zip");
        write_test_zip(
            &refs_unknown,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_COMPANION)),
                ("companion.json", &profile_json),
                ("state.json", r#"{"xp":0}"#),
                ("knowledge_refs.json", r#"{"names":[],"extra":true}"#),
            ],
        );
        let error = import_bundle(&store, &roster, &shared, &refs_unknown)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown field"), "{error}");
        assert!(roster.list().await.is_empty());
    }

    #[tokio::test]
    async fn memory_import_rolls_back_staged_rows_when_a_later_conflict_is_found() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let conflict_id = memory_fixture(51);
        let local = raw_memory(&conflict_id, "knowledge", "本机原内容", "active");
        store.insert_memory_raw(&local).await.unwrap();

        let imported_first = raw_memory(&memory_fixture(50), "knowledge", "先写入事务再触发冲突", "active");
        let imported_conflict = raw_memory(&conflict_id, "knowledge", "导入包不同内容", "active");
        let memory_lines = format!(
            "{}\n{}\n",
            serde_json::to_string(&imported_first).unwrap(),
            serde_json::to_string(&imported_conflict).unwrap()
        );
        let archive = dir.path().join("rollback.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &memory_lines),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );

        let error = import_bundle(&store, &roster, &dir.path().join("shared"), &archive)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("memory import ID collision"), "{error}");
        assert!(store.get_memory(&imported_first.memory_id).await.unwrap().is_none());
        assert_eq!(store.dump_memories_all().await.unwrap(), vec![local]);
    }

    #[tokio::test]
    async fn import_rejects_zip_slip_and_unknown_entries() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let shared = dir.path().join("shared");

        let evil = dir.path().join("evil.zip");
        write_test_zip(
            &evil,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
                ("../evil.jsonl", "escaped"),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &evil).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert!(!dir.path().join("evil.jsonl").exists());

        let exe = dir.path().join("exe.zip");
        write_test_zip(
            &exe,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
                ("events/payload.exe", "MZ"),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &exe).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");

        let stray = dir.path().join("stray.zip");
        write_test_zip(
            &stray,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", ""),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
                ("extra.txt", "?"),
            ],
        );
        let err = import_bundle(&store, &roster, &shared, &stray).await.unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)), "{err:?}");
        assert_eq!(store.dump_memories_all().await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn memory_import_rejects_corrupt_lines_before_writing() {
        let dir = tempfile::TempDir::new().unwrap();
        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let good = serde_json::to_string(&raw_memory(&memory_fixture(20), "knowledge", "好行", "active")).unwrap();

        let corrupt = dir.path().join("corrupt.zip");
        write_test_zip(
            &corrupt,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &format!("{good}\n{{broken json\n")),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
            ],
        );
        let err = import_bundle(&store, &roster, &dir.path().join("shared"), &corrupt)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("第 2 行"), "{err}");
        assert_eq!(
            store.dump_memories_all().await.unwrap().len(),
            0,
            "a corrupt package must not leave a partial import"
        );
    }

    #[test]
    fn empty_event_import_plan_skips_storage_inspection() {
        let dir = tempfile::TempDir::new().unwrap();
        let events_dir = dir.path().join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        std::fs::write(events_dir.join("not-a-day-file.txt"), "historical anomaly").unwrap();

        ensure_event_import_fits_capacity(dir.path(), &[], 16).unwrap();
    }

    #[tokio::test]
    async fn managed_import_rejects_event_overflow_before_memory_or_event_commit() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = dir.path().join("shared");
        let events_dir = shared.join("events");
        std::fs::create_dir_all(&events_dir).unwrap();
        let existing = events_dir.join("20260601.jsonl");
        std::fs::File::create(&existing)
            .unwrap()
            .set_len(16 * 1024 * 1024)
            .unwrap();

        let memory = raw_memory(&memory_fixture(80), "knowledge", "must stay uncommitted", "active");
        let memory_line = format!("{}\n", serde_json::to_string(&memory).unwrap());
        let incoming_event = event_jsonl(nomifun_common::now_ms());
        let archive = dir.path().join("over-cap.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &memory_line),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
                ("events/20260602.jsonl", &incoming_event),
            ],
        );

        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let mut config = crate::profile::SharedCompanionConfig::default();
        config.collect.event_max_storage_mb = 16;
        let config = std::sync::Arc::new(tokio::sync::RwLock::new(config));
        let event_lock = std::sync::Arc::new(tokio::sync::RwLock::new(()));

        let error = import_bundle_with_event_lock(
            &store,
            &roster,
            &shared,
            &archive,
            event_lock,
            config,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, AppError::BadRequest(_)), "{error:?}");
        assert!(error.to_string().contains("16 MiB"), "{error}");
        assert!(store.dump_memories_all().await.unwrap().is_empty());
        assert!(!events_dir.join("20260602.jsonl").exists());
        assert_eq!(std::fs::metadata(existing).unwrap().len(), 16 * 1024 * 1024);
    }

    #[tokio::test]
    async fn managed_import_prunes_expired_events_immediately_after_commit() {
        let dir = tempfile::TempDir::new().unwrap();
        let shared = dir.path().join("shared");
        let memory = raw_memory(&memory_fixture(81), "knowledge", "commit before cleanup", "active");
        let memory_line = format!("{}\n", serde_json::to_string(&memory).unwrap());
        let old_event = event_jsonl(1);
        let archive = dir.path().join("expired-events.zip");
        write_test_zip(
            &archive,
            &[
                ("manifest.json", &manifest_json(3, EXPORT_KIND_MEMORY)),
                ("memories.jsonl", &memory_line),
                ("learn_runs.jsonl", ""),
                ("state.json", r#"{"mood":null}"#),
                ("events/20200101.jsonl", &old_event),
            ],
        );

        let store = CompanionStore::open_memory().await.unwrap();
        let roster = scan_registry(dir.path(), "companions");
        let mut config = crate::profile::SharedCompanionConfig::default();
        config.learn.enabled = false;
        config.evolve.enabled = false;
        config.collect.event_retention_days = 7;
        let config = std::sync::Arc::new(tokio::sync::RwLock::new(config));
        let event_lock = std::sync::Arc::new(tokio::sync::RwLock::new(()));

        let outcome = import_bundle_with_event_lock(
            &store,
            &roster,
            &shared,
            &archive,
            event_lock,
            config,
        )
        .await
        .unwrap();
        assert_eq!(
            outcome,
            ImportOutcome::Memory {
                imported: 1,
                skipped_duplicates: 0,
            }
        );
        assert_eq!(store.dump_memories_all().await.unwrap(), vec![memory]);
        assert!(!shared.join("events/20200101.jsonl").exists());
    }

    #[test]
    fn event_publication_read_failures_rollback_earlier_links() {
        for fail_imported_read in [false, true] {
            let dir = tempfile::TempDir::new().unwrap();
            let staging = dir.path().join("staging");
            let destination = dir.path().join("events");
            std::fs::create_dir_all(&staging).unwrap();
            std::fs::create_dir_all(&destination).unwrap();
            let source_one = staging.join("20260601.jsonl");
            let source_two = staging.join("20260602.jsonl");
            let target_one = destination.join("20260601.jsonl");
            let target_two = destination.join("20260602.jsonl");
            std::fs::write(&source_one, event_jsonl(1)).unwrap();
            let second_bytes = event_jsonl(2);
            std::fs::write(&source_two, &second_bytes).unwrap();
            std::fs::write(&target_two, &second_bytes).unwrap();
            let plan = vec![
                EventImportPlan {
                    source: source_one,
                    target: target_one.clone(),
                },
                EventImportPlan {
                    source: source_two.clone(),
                    target: target_two.clone(),
                },
            ];
            let failed_path = if fail_imported_read {
                source_two.clone()
            } else {
                target_two.clone()
            };

            let error = publish_event_import_with_io(
                &plan,
                |_| Ok(()),
                |path| {
                    if path == failed_path {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "injected read failure",
                        ))
                    } else {
                        std::fs::read(path)
                    }
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("injected read failure"), "{error}");
            assert!(!target_one.exists(), "an earlier hard link must be rolled back");
            assert_eq!(std::fs::read(&target_two).unwrap(), second_bytes.as_bytes());
        }
    }

    #[test]
    fn event_publication_sync_failure_rolls_back_all_links() {
        let dir = tempfile::TempDir::new().unwrap();
        let source = dir.path().join("staging/20260601.jsonl");
        let target = dir.path().join("events/20260601.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, event_jsonl(1)).unwrap();
        let plan = vec![EventImportPlan {
            source: source.clone(),
            target: target.clone(),
        }];

        let error = publish_event_import_with_sync(&plan, |_| {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "injected sync failure",
            ))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected sync failure"), "{error}");
        assert!(source.exists(), "rollback must not remove staged source files");
        assert!(!target.exists(), "published hard links must be rolled back");
    }

    #[test]
    fn dedup_name_picks_first_free_suffix() {
        let mut existing = HashSet::new();
        assert_eq!(dedup_name(&existing, "宠"), "宠");
        existing.insert("宠".to_owned());
        assert_eq!(dedup_name(&existing, "宠"), "宠 (2)");
        existing.insert("宠 (2)".to_owned());
        existing.insert("宠 (3)".to_owned());
        assert_eq!(dedup_name(&existing, "宠"), "宠 (4)");
    }

    #[test]
    fn safe_zip_entry_path_policy() {
        assert!(safe_zip_entry_path("events/a.jsonl").is_ok());
        assert!(safe_zip_entry_path("./state.json").is_ok());
        assert!(safe_zip_entry_path("../evil.jsonl").is_err());
        assert!(safe_zip_entry_path("events/../../evil.jsonl").is_err());
        assert!(safe_zip_entry_path("/abs.jsonl").is_err());
        assert!(safe_zip_entry_path("events\\win.jsonl").is_err());
        assert!(safe_zip_entry_path("").is_err());
        assert!(safe_zip_entry_path("C:/evil.jsonl").is_err());
    }
}
