use std::{collections::{BTreeMap, HashMap}, fs, path::{Path, PathBuf}, sync::{Arc, Mutex as StdMutex, RwLock, atomic::{AtomicBool, Ordering}}, time::Duration};
use nomi_process_runtime::{ChildProcessBuilder, ManagedChildProcess, ExactProcessIdentity, OrphanTerminationOutcome, capture_child_identity, terminate_verified_orphan, merge_process_path};
use serde::{Deserialize, Serialize};
use tokio::{io::{AsyncRead, AsyncReadExt}, sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;
use crate::{AppDetails, AppError, AppMetadata, AppOperation, AppRecord, AppSettings, AppSettingsPatch, AppState, ImportKind, LocalConnection};
use crate::files::{complete_execution_settings, required_project_runtime, contained_directory, copy_import, detect_settings, ensure_no_links, relative_path, prepare_static_preset, standard_static_web_url};

const OUTPUT_LIMIT: usize = 32 * 1024;
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(30);
const PREPARATION_TIMEOUT: Duration = Duration::from_secs(20 * 60);
const LOCAL_LISTEN_WAIT: Duration = Duration::from_secs(2);
const LOCAL_CONNECT_ATTEMPT: Duration = Duration::from_millis(100);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRecord {
    #[serde(deserialize_with = "read_current_record")]
    pub(crate) record: AppRecord,
    pub(crate) identity: Option<ExactProcessIdentity>,
}

/// Current application registries previously wrote a redundant empty Session
/// ownership field despite having no producer for that ownership. Retire only
/// that exact empty value on read; anything else fails closed without touching
/// files or looking up, converting, or deleting any Agent Session.
fn read_current_record<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<AppRecord, D::Error> {
    let mut value = serde_json::Value::deserialize(deserializer)?;
    if let Some(settings) = value.get_mut("settings").and_then(serde_json::Value::as_object_mut)
        && let Some(retired) = settings.remove("exclusive_session_ids")
        && retired.as_array().is_none_or(|references| !references.is_empty())
    {
        return Err(serde::de::Error::custom("Application registry contains nonempty or invalid retired exclusive_session_ids; files and all canonical Session history are preserved, and no Session ownership conversion is permitted"));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}

struct Running {
    generation: String,
    process: Mutex<ManagedChildProcess>,
    output: Vec<Arc<StdMutex<Vec<u8>>>>,
    readers: Mutex<Vec<JoinHandle<()>>>,
}

struct AppData {
    record: AppRecord,
    identity: Option<ExactProcessIdentity>,
    running: Option<Arc<Running>>,
    cancellation: Option<CancellationToken>,
    deleted: bool,
}

enum SettingsUpdate { Replace(AppSettings), Patch(AppSettingsPatch) }

struct AppSlot {
    /// Long waits hold only this application's gate, never the directory map.
    gate: Mutex<()>,
    data: Mutex<AppData>,
}

pub struct HostingOwner {
    root: PathBuf,
    connection: RwLock<LocalConnection>,
    slots: RwLock<BTreeMap<String, Arc<AppSlot>>>,
    closing: AtomicBool,
}

impl HostingOwner {
    pub async fn new(root: PathBuf, connection: LocalConnection) -> Result<Arc<Self>, AppError> {
        fs::create_dir_all(&root)?;
        let root = dunce::canonicalize(root)?;
        ensure_no_links(&root, &root.join("registry"))?;
        ensure_no_links(&root, &root.join("apps"))?;
        fs::create_dir_all(root.join("registry"))?;
        fs::create_dir_all(root.join("apps"))?;
        let owner = Arc::new(Self { root, connection: RwLock::new(connection), slots: RwLock::new(BTreeMap::new()), closing: AtomicBool::new(false) });
        for entry in fs::read_dir(owner.root.join("registry"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") { continue; }
            owner.ensure_owned_path(&path)?;
            let mut stored: StoredRecord = serde_json::from_slice(&fs::read(&path)?)?;
            validate_id(&stored.record.id)?;
            if path.file_stem().is_none_or(|stem| stem != stored.record.id.as_str()) {
                return Err(AppError::Invalid("application registry identity mismatch".into()));
            }
            let expected = owner.app_directory(&stored.record.id);
            if stored.record.directory != expected {
                return Err(AppError::Invalid("application registry points outside its owned directory".into()));
            }
            let had_process = stored.identity.is_some() || stored.record.state == AppState::Running || stored.record.operation.is_some();
            if let Some(identity) = &stored.identity {
                let identity = identity.clone();
                match tokio::task::spawn_blocking(move || terminate_verified_orphan(&identity, CLEANUP_TIMEOUT)).await {
                    Ok(Ok(OrphanTerminationOutcome::AlreadyDead | OrphanTerminationOutcome::KilledAndProven)) => { stored.identity = None; stored.record.pid = None; }
                    Ok(Ok(OrphanTerminationOutcome::RootGoneCleanupUnproven)) => {
                        stored.record.last_error = Some("Host restarted; previous root exited but its process-group cleanup could not be confirmed".into());
                    }
                    Ok(Err(error)) => stored.record.last_error = Some(format!("Host restarted; previous process cleanup could not be confirmed: {error}")),
                    Err(error) => stored.record.last_error = Some(format!("Host restarted; process reconciliation failed: {error}")),
                }
            }
            if had_process {
                stored.record.state = AppState::Failed;
                stored.record.operation = None;
                stored.record.last_error.get_or_insert_with(|| "Host restarted; the previous application process is no longer running".into());
            }
            let id = stored.record.id.clone();
            let slot = Arc::new(AppSlot { gate: Mutex::new(()), data: Mutex::new(AppData { record: stored.record, identity: stored.identity, running: None, cancellation: None, deleted: false }) });
            owner.persist(&slot).await?;
            owner.slots.write().unwrap_or_else(|e| e.into_inner()).insert(id, slot);
        }
        Ok(owner)
    }

    /// Call after the Host has bound its actual port. Secrets remain in memory.
    pub fn set_local_endpoint(&self, host_url: String) {
        self.connection.write().unwrap_or_else(|e| e.into_inner()).host_url = host_url;
    }

    pub async fn list(&self) -> Result<Vec<AppRecord>, AppError> {
        let slots: Vec<_> = self.slots.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        let mut records = Vec::with_capacity(slots.len());
        for slot in slots {
            let data = slot.data.lock().await;
            if !data.deleted { records.push(self.snapshot(&data)); }
        }
        Ok(records)
    }

    pub async fn get(&self, id: &str) -> Result<AppRecord, AppError> {
        let slot = self.slot(id)?;
        let data = slot.data.lock().await;
        if data.deleted { return Err(AppError::NotFound(id.into())); }
        Ok(self.snapshot(&data))
    }

    /// A new application is an ordinary current Source record. The same owned
    /// directory is its development workspace and its command working tree.
    pub async fn create(&self, name: String, metadata: AppMetadata) -> Result<AppRecord, AppError> {
        self.ensure_open()?;
        let name = validate_name(name)?;
        let metadata = validate_metadata(metadata)?;
        let id = uuid::Uuid::now_v7().to_string();
        let directory = self.app_directory(&id);
        self.ensure_owned_path(&directory)?;
        fs::create_dir(&directory)?;
        if let Err(error) = self.ensure_open() { self.remove_owned_directory(&id)?; return Err(error); }
        let record = AppRecord { id: id.clone(), name, description: metadata.description, tags: metadata.tags,
            development_session_id: None, directory, import_kind: ImportKind::Source, settings: AppSettings::default(),
            state: AppState::Stopped, operation: None, last_error: None, output: String::new(), pid: None,
            preview: false, prepared: false };
        let slot = Arc::new(AppSlot { gate: Mutex::new(()), data: Mutex::new(AppData { record: record.clone(),
            identity: None, running: None, cancellation: None, deleted: false }) });
        if let Err(error) = self.persist(&slot).await { self.remove_owned_directory(&id)?; return Err(error); }
        self.slots.write().unwrap_or_else(|e| e.into_inner()).insert(id, slot);
        Ok(record)
    }

    /// The canonical Session owner validates/reopens the reference, or opens
    /// an ordinary Session for this exact workspace. The application only
    /// stores its returned identity under its existing lifecycle gate.
    pub async fn development_session_with<F, Fut>(&self, id: &str, resolve: F) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord) -> Fut,
        Fut: std::future::Future<Output = Result<String, AppError>>,
    {
        self.ensure_open()?;
        let slot = self.slot(id)?;
        let _gate = slot.gate.lock().await;
        self.ensure_owned_path(&self.app_directory(id))?;
        let record = { let data = slot.data.lock().await;
            if data.deleted { return Err(AppError::NotFound(id.into())); }
            self.snapshot(&data)
        };
        let session_id = resolve(record).await?;
        validate_id(&session_id)?;
        let mut data = slot.data.lock().await;
        let previous = data.record.development_session_id.clone();
        data.record.development_session_id = Some(session_id);
        if let Err(error) = self.persist_data(&data) { data.record.development_session_id = previous; return Err(error); }
        Ok(self.snapshot(&data))
    }

    pub async fn import(self: &Arc<Self>, source: &Path, name: Option<String>, kind: ImportKind) -> Result<AppRecord, AppError> {
        // A cancelled coding-tool waiter does not orphan an unregistered copy.
        // The worker retains ownership through registration or cleanup.
        let owner = self.clone();
        let source = source.to_owned();
        tokio::spawn(async move { owner.import_inner(&source, name, kind).await }).await
            .map_err(|error| AppError::OutcomeUnknown(format!("application import worker failed: {error}")))?
    }

    async fn import_inner(&self, source: &Path, name: Option<String>, kind: ImportKind) -> Result<AppRecord, AppError> {
        self.ensure_open()?;
        let id = uuid::Uuid::now_v7().to_string();
        let directory = self.app_directory(&id);
        self.ensure_owned_path(&directory)?;
        let source_name = source.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let name = validate_name(name.filter(|value| !value.trim().is_empty()).unwrap_or(source_name))?;
        let source = source.to_owned();
        let destination = directory.clone();
        let settings = tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&destination)?;
            copy_import(&source, &destination)?;
            detect_settings(&destination, kind)
        }).await.map_err(|error| AppError::OutcomeUnknown(format!("import worker failed: {error}")))?;
        let settings = match settings {
            Ok(settings) => settings,
            Err(error) => { self.remove_owned_directory(&id)?; return Err(error); }
        };
        if let Err(error) = self.ensure_open() {
            self.remove_owned_directory(&id)?;
            return Err(error);
        }
        let record = AppRecord { id: id.clone(), name, description: String::new(), tags: Vec::new(), development_session_id: None,
            directory, import_kind: kind, settings, state: AppState::Stopped,
            operation: None, last_error: None, output: String::new(), pid: None, preview: false, prepared: false };
        let slot = Arc::new(AppSlot { gate: Mutex::new(()), data: Mutex::new(AppData { record: record.clone(), identity: None, running: None, cancellation: None, deleted: false }) });
        if let Err(error) = self.persist(&slot).await { self.remove_owned_directory(&id)?; return Err(error); }
        self.slots.write().unwrap_or_else(|e| e.into_inner()).insert(id, slot);
        Ok(record)
    }

    /// Import assistance and manual import share the application's exact
    /// current directory. Existing data absent from the new source survives;
    /// commands are detected again from the newly imported source/artifact.
    pub async fn import_into(self: &Arc<Self>, id: &str, source: &Path, name: Option<String>, kind: ImportKind) -> Result<AppRecord, AppError> {
        self.import_into_with(id, source, name, kind, |_| async { Ok(()) }).await
    }

    pub async fn import_into_with<F, Fut>(self: &Arc<Self>, id: &str, source: &Path, name: Option<String>, kind: ImportKind, prepare: F) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), AppError>> + Send + 'static,
    {
        let owner = self.clone();
        let id = id.to_owned();
        let source = source.to_owned();
        tokio::spawn(async move { owner.import_into_inner(&id, &source, name, kind, prepare).await }).await
            .map_err(|error| AppError::OutcomeUnknown(format!("application import worker failed: {error}")))?
    }

    async fn import_into_inner<F, Fut>(&self, id: &str, source: &Path, name: Option<String>, kind: ImportKind, prepare: F) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord) -> Fut,
        Fut: std::future::Future<Output = Result<(), AppError>>,
    {
        self.ensure_open()?;
        let name = name.map(validate_name).transpose()?;
        let slot = self.slot(id)?;
        self.cancel_preparation(&slot).await;
        let _gate = slot.gate.lock().await;
        let current = { let data = slot.data.lock().await;
            if data.deleted { return Err(AppError::NotFound(id.into())); }
            self.snapshot(&data)
        };
        prepare(current).await?;
        self.stop_slot(&slot).await?;
        self.ensure_owned_path(&self.app_directory(id))?;
        let directory = contained_directory(&self.app_directory(id), ".")?;
        let source = dunce::canonicalize(source)?;
        if source.starts_with(&directory) || directory.starts_with(&source) {
            return Err(AppError::Invalid("import source must be separate from the application's current directory".into()));
        }
        {
            let mut data = slot.data.lock().await;
            data.record.prepared = false;
            self.persist_data(&data)?;
        }
        let outcome = match tokio::task::spawn_blocking(move || {
            copy_import(&source, &directory)?;
            detect_settings(&directory, kind)
        }).await {
            Ok(outcome) => outcome,
            Err(error) => Err(AppError::OutcomeUnknown(format!("import worker failed: {error}"))),
        };
        let mut settings = match outcome {
            Ok(settings) => settings,
            Err(error) => { self.record_failure(&slot, &error).await?; return Err(error); }
        };
        let mut data = slot.data.lock().await;
        settings.agent_id = data.record.settings.agent_id.clone();
        settings.agent_binding_id = data.record.settings.agent_binding_id.clone();
        let previous = data.record.clone();
        if let Some(name) = name { data.record.name = name; }
        data.record.import_kind = kind;
        data.record.settings = settings;
        data.record.prepared = false;
        if let Err(error) = self.persist_data(&data) {
            data.record = previous;
            data.record.state = AppState::Failed;
            data.record.last_error = Some(format!("Application files were imported, but their settings could not be saved: {error}"));
            let _ = self.persist_data(&data);
            return Err(error);
        }
        Ok(self.snapshot(&data))
    }

    pub async fn update(&self, id: &str, name: String, settings: AppSettings) -> Result<AppRecord, AppError> {
        self.update_with(id, name, settings, |_, settings| async { Ok(settings) }).await
    }

    pub async fn update_details(&self, id: &str, name: String, details: AppDetails, settings: AppSettings) -> Result<AppRecord, AppError> {
        self.update_details_with_rollback(id, name, details, settings, |_, settings| async { Ok(settings) }, || async { Ok(()) }).await
    }

    /// Apply only this request's edits to the current record under the existing
    /// application gate. Detection configures files without running them.
    pub async fn update_patch_details(&self, id: &str, name: Option<String>, details: AppDetails,
        settings: AppSettingsPatch, detect_settings: bool) -> Result<AppRecord, AppError> {
        self.update_patch_details_with_rollback(id, name, details, settings, detect_settings,
            |_, settings| async { Ok(settings) }, || async { Ok(()) }).await
    }

    pub async fn update_patch_details_with_rollback<F, Fut, R, Undo>(&self, id: &str, name: Option<String>, details: AppDetails,
        settings: AppSettingsPatch, detect_settings: bool, prepare: F, rollback: R) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
        R: FnOnce() -> Undo,
        Undo: std::future::Future<Output = Result<(), AppError>>,
    {
        self.update_guarded_patch_details_with_rollback(id, name, details, settings, detect_settings, |_| Ok(()), prepare, rollback).await
    }

    /// Recheck an existing caller-owned admission fence before inspecting or
    /// modifying files. This check creates no new authorization or ledger.
    pub async fn update_guarded_patch_details_with_rollback<Check, F, Fut, R, Undo>(&self, id: &str, name: Option<String>, details: AppDetails,
        settings: AppSettingsPatch, detect_settings: bool, check: Check, prepare: F, rollback: R) -> Result<AppRecord, AppError>
    where
        Check: FnOnce(&AppRecord) -> Result<(), AppError>,
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
        R: FnOnce() -> Undo,
        Undo: std::future::Future<Output = Result<(), AppError>>,
    {
        self.update_current_details_with_rollback(id, name, details, SettingsUpdate::Patch(settings), detect_settings, check, prepare, rollback).await
    }

    /// Prepare current settings through their existing domain owners while
    /// holding the application's same lifecycle gate. The callback must not
    /// recursively mutate this application through HostingOwner.
    pub async fn update_with<F, Fut>(&self, id: &str, name: String, settings: AppSettings, prepare: F) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
    {
        self.update_with_rollback(id, name, settings, prepare, || async { Ok(()) }).await
    }

    /// A prepared domain change is compensated before releasing this same
    /// lifecycle gate when current settings cannot be committed. The receipt
    /// belongs to the caller's existing owner and remains memory-only.
    pub async fn update_with_rollback<F, Fut, R, Undo>(&self, id: &str, name: String, settings: AppSettings, prepare: F, rollback: R) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
        R: FnOnce() -> Undo,
        Undo: std::future::Future<Output = Result<(), AppError>>,
    {
        self.update_details_with_rollback(id, name, AppDetails::default(), settings, prepare, rollback).await
    }

    pub async fn update_details_with_rollback<F, Fut, R, Undo>(&self, id: &str, name: String, details: AppDetails, settings: AppSettings, prepare: F, rollback: R) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
        R: FnOnce() -> Undo,
        Undo: std::future::Future<Output = Result<(), AppError>>,
    {
        self.update_current_details_with_rollback(id, Some(name), details, SettingsUpdate::Replace(settings), false, |_| Ok(()), prepare, rollback).await
    }

    async fn update_current_details_with_rollback<Check, F, Fut, R, Undo>(&self, id: &str, name: Option<String>, details: AppDetails,
        requested: SettingsUpdate, detect_settings: bool, check: Check, prepare: F, rollback: R) -> Result<AppRecord, AppError>
    where
        Check: FnOnce(&AppRecord) -> Result<(), AppError>,
        F: FnOnce(AppRecord, AppSettings) -> Fut,
        Fut: std::future::Future<Output = Result<AppSettings, AppError>>,
        R: FnOnce() -> Undo,
        Undo: std::future::Future<Output = Result<(), AppError>>,
    {
        self.ensure_open()?;
        let slot = self.slot(id)?;
        let _gate = slot.gate.lock().await;
        let directory = self.app_directory(id);
        self.ensure_owned_path(&directory)?;
        let (current, has_process) = {
            let data = slot.data.lock().await;
            if data.deleted { return Err(AppError::NotFound(id.into())); }
            (data.record.clone(), data.record.state == AppState::Running || data.running.is_some() || data.identity.is_some())
        };
        check(&current)?;
        let name = validate_name(name.unwrap_or_else(|| current.name.clone()))?;
        let preparation_overrides = match &requested {
            SettingsUpdate::Patch(patch) => (patch.install_command.clone(), patch.build_command.clone()),
            SettingsUpdate::Replace(_) => (None, None),
        };
        let mut settings = match requested {
            SettingsUpdate::Replace(settings) => settings,
            SettingsUpdate::Patch(patch) => { let mut settings = current.settings.clone(); patch.apply(&mut settings); settings },
        };
        validate_settings(&directory, &settings)?;
        validate_execution_change(has_process, &current.settings, &settings)?;
        let metadata = validate_metadata(AppMetadata {
            description: details.description.unwrap_or_else(|| current.description.clone()),
            tags: details.tags.unwrap_or_else(|| current.tags.clone()),
        })?;
        let kind = details.import_kind.unwrap_or(current.import_kind);
        if has_process && kind != current.import_kind {
            return Err(AppError::Invalid("stop the application before changing its import kind".into()));
        }
        if detect_settings {
            if has_process && command(&settings.preview_command).is_none() && command(&settings.start_command).is_none() {
                return Err(AppError::Invalid("stop the application before detecting its execution settings".into()));
            }
            let root = directory.clone();
            let preview = current.preview;
            settings = tokio::task::spawn_blocking(move || {
                let mut settings = complete_execution_settings(&root, kind, &settings)?;
                if let Some(command) = preparation_overrides.0 { settings.install_command = command; }
                if let Some(command) = preparation_overrides.1 { settings.build_command = command; }
                if settings.web_url.as_deref().is_none_or(|value| value.trim().is_empty()) {
                    let selected = if preview { command(&settings.preview_command).or_else(|| command(&settings.start_command)) }
                        else { command(&settings.start_command).or_else(|| command(&settings.preview_command)) };
                    settings.web_url = selected.and_then(|selected| standard_static_web_url(&root, &settings, selected));
                }
                Ok::<_, AppError>(settings)
            }).await.map_err(|error| AppError::OutcomeUnknown(format!("project discovery worker failed: {error}")))??;
            validate_settings(&directory, &settings)?;
            validate_execution_change(has_process, &current.settings, &settings)?;
        }
        let mut settings = prepare(current.clone(), settings).await?;
        let prepared = (|| {
            self.ensure_open()?;
            self.ensure_owned_path(&directory)?;
            validate_settings(&directory, &settings)?;
            validate_execution_change(has_process, &current.settings, &settings)?;
            if !has_process { prepare_static_preset(&directory, &mut settings)?; }
            Ok::<_, AppError>(())
        })();
        if let Err(error) = prepared {
            if let Err(undo_error) = rollback().await {
                return Err(self.retain_failed_update(&slot, &settings, &error, &undo_error).await);
            }
            return Err(error);
        }
        let mut data = slot.data.lock().await;
        let previous = data.record.clone();
        data.record.name = name;
        data.record.description = metadata.description;
        data.record.tags = metadata.tags;
        data.record.import_kind = kind;
        data.record.settings = settings.clone();
        if let Err(error) = self.persist_data(&data) {
            data.record = previous;
            drop(data);
            if let Err(undo_error) = rollback().await {
                return Err(self.retain_failed_update(&slot, &settings, &error, &undo_error).await);
            }
            return Err(error);
        }
        Ok(self.snapshot(&data))
    }

    async fn retain_failed_update(&self, slot: &AppSlot, settings: &AppSettings, error: &AppError, undo_error: &AppError) -> AppError {
        let failure = AppError::Command(format!("application settings were not saved: {error}; Agent connection rollback could not be confirmed: {undo_error}"));
        let mut data = slot.data.lock().await;
        // If a newly prepared connection could not be removed, keep its sole
        // reference in the same minimal record for explicit cleanup/retry.
        // A failed clear keeps the previous reference instead.
        if settings.agent_binding_id.is_some() {
            data.record.settings.agent_binding_id = settings.agent_binding_id.clone();
            data.record.settings.agent_id = settings.agent_id.clone();
        }
        data.record.state = AppState::Failed;
        data.record.last_error = Some(self.sanitize(&failure.to_string(), &data.record.settings));
        let _ = self.persist_data(&data);
        failure
    }

    /// Copies over current files after exact process shutdown. It deliberately
    /// preserves application-owned data absent from the replacement source.
    pub async fn overwrite(&self, id: &str, source: &Path) -> Result<AppRecord, AppError> {
        self.overwrite_with(id, source, |_| async { Ok(()) }).await
    }

    pub async fn overwrite_with<F, Fut>(&self, id: &str, source: &Path, prepare: F) -> Result<AppRecord, AppError>
    where
        F: FnOnce(AppRecord) -> Fut,
        Fut: std::future::Future<Output = Result<(), AppError>>,
    {
        self.ensure_open()?;
        let slot = self.slot(id)?;
        self.cancel_preparation(&slot).await;
        let _gate = slot.gate.lock().await;
        let current = { let data = slot.data.lock().await;
            if data.deleted { return Err(AppError::NotFound(id.into())); }
            self.snapshot(&data)
        };
        prepare(current).await?;
        self.stop_slot(&slot).await?;
        self.ensure_owned_path(&self.app_directory(id))?;
        let directory = contained_directory(&self.app_directory(id), ".")?;
        let source = dunce::canonicalize(source)?;
        if source.starts_with(&directory) || directory.starts_with(&source) {
            return Err(AppError::Invalid("overwrite source must be separate from the managed copy".into()));
        }
        {
            let mut data = slot.data.lock().await;
            data.record.prepared = false;
            self.persist_data(&data)?;
        }
        tokio::task::spawn_blocking(move || copy_import(&source, &directory)).await
            .map_err(|error| AppError::Invalid(format!("overwrite worker failed: {error}")))??;
        let mut data = slot.data.lock().await;
        data.record.prepared = false;
        self.persist_data(&data)?;
        Ok(self.snapshot(&data))
    }

    pub async fn operate(self: &Arc<Self>, id: &str, operation: AppOperation) -> Result<AppRecord, AppError> {
        // HTTP disconnects and abandoned Agent waits must not drop the effect
        // owner midway through installation/build. The retained flight still
        // publishes its real result and responds to Stop/Host shutdown.
        let owner = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { owner.operate_inner(&id, operation).await }).await
            .map_err(|error| AppError::OutcomeUnknown(format!("application operation worker failed: {error}")))?
    }

    async fn operate_inner(self: &Arc<Self>, id: &str, operation: AppOperation) -> Result<AppRecord, AppError> {
        if operation != AppOperation::Stop { self.ensure_open()?; }
        let slot = self.slot(id)?;
        if operation == AppOperation::Stop { self.cancel_preparation(&slot).await; }
        let _gate = slot.gate.lock().await;
        if slot.data.lock().await.deleted { return Err(AppError::NotFound(id.into())); }
        if operation != AppOperation::Stop { self.ensure_open()?; }
        if operation == AppOperation::Stop {
            match self.stop_slot(&slot).await {
                Ok(()) => return self.get(id).await,
                Err(error) => { self.record_failure(&slot, &error).await?; return Err(error); }
            }
        }
        let cancellation = CancellationToken::new();
        {
            let mut data = slot.data.lock().await;
            // Publish cancellation under the same data lock that shutdown
            // reads, with its closing fence checked inside that lock. This
            // closes the gap between queued-operation admission and shutdown.
            self.ensure_open()?;
            data.cancellation = Some(cancellation.clone());
        }
        let result = self.run_operation(&slot, operation, &cancellation).await;
        slot.data.lock().await.cancellation = None;
        match result {
            Ok(()) => self.get(id).await,
            Err(error) => { self.record_failure(&slot, &error).await?; Err(error) }
        }
    }

    /// Session references require their canonical owner's workspace fence.
    /// Partial failure retains this same record so deletion can be retried.
    pub async fn delete(&self, id: &str) -> Result<(), AppError> {
        self.delete_with(id, |record| async move {
            if record.development_session_id.is_none() { Ok(()) }
            else { Err(AppError::Invalid("applications with canonical Session references must be deleted through the Session owner's workspace fence".into())) }
        }).await
    }

    /// Session workspace fences and Agent connection cleanup use this same
    /// application gate. Ordinary canonical Sessions are never deleted here.
    pub async fn delete_with<F, Fut>(&self, id: &str, cleanup: F) -> Result<(), AppError>
    where
        F: FnOnce(AppRecord) -> Fut,
        Fut: std::future::Future<Output = Result<(), AppError>>,
    {
        let slot = self.slot(id)?;
        self.cancel_preparation(&slot).await;
        let _gate = slot.gate.lock().await;
        if slot.data.lock().await.deleted { return Err(AppError::NotFound(id.into())); }
        self.stop_slot(&slot).await?;
        self.ensure_owned_path(&self.app_directory(id))?;
        let record = {
            let data = slot.data.lock().await;
            self.snapshot(&data)
        };
        if let Err(error) = cleanup(record).await {
            self.record_failure(&slot, &error).await?;
            return Err(error);
        }
        self.remove_owned_directory(id)?;
        let registry = self.registry_path(id);
        if registry.exists() { fs::remove_file(registry)?; }
        slot.data.lock().await.deleted = true;
        self.slots.write().unwrap_or_else(|e| e.into_inner()).remove(id);
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), AppError> {
        self.closing.store(true, Ordering::Release);
        let slots: Vec<_> = self.slots.read().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for slot in &slots { self.cancel_preparation(slot).await; }
        let mut pending = tokio::task::JoinSet::new();
        // Each owner is stopped independently; a slow tree does not prevent
        // the other applications receiving their shutdown request.
        for slot in slots {
            pending.spawn(async move {
                let _gate = slot.gate.lock().await;
                let running = slot.data.lock().await.running.clone();
                if let Some(running) = running { shutdown_running(&running).await?; }
                Ok::<_, AppError>(slot.clone())
            });
        }
        let mut failure = None;
        while let Some(result) = pending.join_next().await {
            match result {
                Ok(Ok(slot)) => { self.stop_slot(&slot).await?; }
                Ok(Err(error)) => { failure.get_or_insert(error); }
                Err(error) => { failure.get_or_insert_with(|| AppError::Command(format!("shutdown worker failed: {error}"))); }
            }
        }
        failure.map_or(Ok(()), Err)
    }

    fn slot(&self, id: &str) -> Result<Arc<AppSlot>, AppError> {
        self.slots.read().unwrap_or_else(|e| e.into_inner()).get(id).cloned().ok_or_else(|| AppError::NotFound(id.into()))
    }

    fn ensure_open(&self) -> Result<(), AppError> {
        if self.closing.load(Ordering::Acquire) { Err(AppError::ShuttingDown) } else { Ok(()) }
    }

    fn app_directory(&self, id: &str) -> PathBuf { self.root.join("apps").join(id) }
    fn registry_path(&self, id: &str) -> PathBuf { self.root.join("registry").join(format!("{id}.json")) }

    fn ensure_owned_path(&self, path: &Path) -> Result<(), AppError> {
        ensure_no_links(&self.root, path)?;
        if dunce::canonicalize(&self.root)? != self.root {
            return Err(AppError::Invalid("hosting root no longer resolves to its original owned directory".into()));
        }
        Ok(())
    }

    fn remove_owned_directory(&self, id: &str) -> Result<(), AppError> {
        validate_id(id)?;
        let path = self.app_directory(id);
        self.ensure_owned_path(&path)?;
        if path.exists() {
            let resolved = dunce::canonicalize(&path)?;
            let apps = dunce::canonicalize(self.root.join("apps"))?;
            if resolved.parent() != Some(apps.as_path()) || fs::symlink_metadata(&path)?.is_symlink() {
                return Err(AppError::Invalid("refusing deletion outside the application-owned directory".into()));
            }
            fs::remove_dir_all(&path)?;
        }
        Ok(())
    }

    async fn cancel_preparation(&self, slot: &AppSlot) {
        if let Some(token) = &slot.data.lock().await.cancellation { token.cancel(); }
    }

    async fn run_operation(self: &Arc<Self>, slot: &Arc<AppSlot>, operation: AppOperation, cancellation: &CancellationToken) -> Result<(), AppError> {
        let mut record = { let data = slot.data.lock().await; data.record.clone() };
        self.ensure_owned_path(&record.directory)?;
        validate_settings(&record.directory, &record.settings)?;
        if record.state != AppState::Running && command(&record.settings.preview_command).is_none() && command(&record.settings.start_command).is_none() {
            // Unknown prior cleanup must be resolved before writing a preset.
            self.stop_slot(slot).await?;
            let root = record.directory.clone();
            let kind = record.import_kind;
            let current = record.settings.clone();
            let settings = tokio::task::spawn_blocking(move || complete_execution_settings(&root, kind, &current)).await
                .map_err(|error| AppError::OutcomeUnknown(format!("project discovery worker failed: {error}")))??;
            validate_settings(&record.directory, &settings)?;
            let mut data = slot.data.lock().await;
            let previous = data.record.settings.clone();
            data.record.settings = settings;
            if let Err(error) = self.persist_data(&data) { data.record.settings = previous; return Err(error); }
            record = data.record.clone();
        }
        if record.settings.web_url.as_deref().is_none_or(|value| value.trim().is_empty()) {
            let selected = if operation == AppOperation::Preview { command(&record.settings.preview_command).or_else(|| command(&record.settings.start_command)) }
                else { command(&record.settings.start_command) };
            if let Some(url) = selected.and_then(|selected| standard_static_web_url(&record.directory, &record.settings, selected)) {
                let mut data = slot.data.lock().await;
                let previous = data.record.settings.web_url.clone();
                data.record.settings.web_url = Some(url);
                if let Err(error) = self.persist_data(&data) { data.record.settings.web_url = previous; return Err(error); }
                record = data.record.clone();
            }
        }
        if record.state == AppState::Running && !record.preview && matches!(operation, AppOperation::Start | AppOperation::Use) { return Ok(()); }
        if record.state == AppState::Running && record.preview && operation == AppOperation::Preview { return Ok(()); }
        if operation == AppOperation::Use && record.state == AppState::Running && record.preview
            && command(&record.settings.build_command).is_none() && record.settings.preview_command == record.settings.start_command {
            let mut data = slot.data.lock().await;
            data.record.preview = false;
            data.record.prepared = true;
            self.persist_data(&data)?;
            return Ok(());
        }
        self.stop_slot(slot).await?;
        if !record.prepared || operation == AppOperation::Rebuild {
            if operation != AppOperation::Start {
                self.once(slot, "installing", command(&record.settings.install_command), cancellation).await?;
            }
        }
        let preview_command = command(&record.settings.preview_command);
        let preview = operation == AppOperation::Preview;
        if matches!(operation, AppOperation::Use | AppOperation::Rebuild) || (preview && preview_command.is_none()) {
            self.once(slot, "building", command(&record.settings.build_command), cancellation).await?;
        }
        let selected = if preview { preview_command.or_else(|| command(&record.settings.start_command)) } else { command(&record.settings.start_command) };
        let selected = selected.ok_or_else(|| AppError::Invalid("configure a foreground start command before running this application".into()))?;
        if cancellation.is_cancelled() { return Err(AppError::Cancelled); }
        self.set_operation(slot, if preview { "starting_preview" } else { "starting" }).await?;
        let running = self.spawn(slot, selected, true, cancellation).await?;
        // Catch immediate launcher exits and port conflicts truthfully. A live
        // process is an observation, not a business-health assertion.
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Err(error) = wait_for_local_web_listener(record.settings.web_url.as_deref(), &running, cancellation).await {
            self.stop_slot(slot).await?;
            return Err(error);
        }
        if cancellation.is_cancelled() { self.stop_slot(slot).await?; return Err(AppError::Cancelled); }
        let initial_status = running.process.lock().await.try_wait()?;
        if let Some(status) = initial_status {
            shutdown_running(&running).await?;
            self.clear_process(slot, &running).await?;
            return Err(AppError::Command(format!("foreground application exited during startup ({status})")));
        }
        {
            let mut data = slot.data.lock().await;
            data.record.state = AppState::Running;
            data.record.operation = None;
            data.record.last_error = None;
            data.record.preview = preview;
            if matches!(operation, AppOperation::Use | AppOperation::Rebuild) { data.record.prepared = true; }
            self.persist_data(&data)?;
        }
        self.monitor(slot.clone(), running);
        Ok(())
    }

    async fn once(&self, slot: &Arc<AppSlot>, step: &str, selected: Option<&str>, cancellation: &CancellationToken) -> Result<(), AppError> {
        let Some(selected) = selected else { return Ok(()); };
        if cancellation.is_cancelled() { return Err(AppError::Cancelled); }
        self.set_operation(slot, step).await?;
        let running = self.spawn(slot, selected, false, cancellation).await?;
        let outcome = {
            let mut process = running.process.lock().await;
            tokio::select! {
                status = process.wait() => status.map_err(AppError::Io),
                _ = cancellation.cancelled() => Err(AppError::Cancelled),
                _ = tokio::time::sleep(PREPARATION_TIMEOUT) => Err(AppError::Command(format!("{step} exceeded 20 minutes"))),
            }
        };
        shutdown_running(&running).await?;
        self.clear_process(slot, &running).await?;
        let status = outcome?;
        if !status.success() { return Err(AppError::Command(format!("{step} failed ({status}); see the command output"))); }
        Ok(())
    }

    async fn set_operation(&self, slot: &AppSlot, operation: &str) -> Result<(), AppError> {
        let mut data = slot.data.lock().await;
        data.record.operation = Some(operation.into());
        data.record.last_error = None;
        self.persist_data(&data)
    }

    async fn spawn(&self, slot: &Arc<AppSlot>, selected: &str, inject_connection: bool, cancellation: &CancellationToken) -> Result<Arc<Running>, AppError> {
        let record = slot.data.lock().await.record.clone();
        self.ensure_owned_path(&record.directory)?;
        // Preparation can change the current filesystem. Recheck the existing
        // project/output contract after each completed step, before spawning
        // the next command or the static preset against a replaced directory.
        validate_settings(&record.directory, &record.settings)?;
        let directory = contained_directory(&record.directory, &record.settings.project_directory)?;
        let mut configured: HashMap<String, String> = record.settings.environment.iter().map(|(key, value)| (key.clone(), value.clone())).collect();
        let custom_node = record.settings.node_path.as_deref().filter(|path| !path.trim().is_empty()).map(|path| {
            let node = dunce::canonicalize(path).map_err(|error| AppError::Invalid(format!("custom Node executable is unavailable: {error}")))?;
            if !node.is_file() { return Err(AppError::Invalid("custom Node path is not a file".into())); }
            Ok(node)
        }).transpose()?;
        if let Some(node) = &custom_node {
            let parent = node.parent().ok_or_else(|| AppError::Invalid("custom Node path has no directory".into()))?;
            let existing = configured.iter().find(|(key, _)| if cfg!(windows) { key.eq_ignore_ascii_case("PATH") } else { key.as_str() == "PATH" }).map(|(_, value)| value.clone());
            let existing = match existing {
                Some(value) => value,
                None => nomifun_runtime::process_environment(&HashMap::new()).map_err(|cause| AppError::Command(cause.to_string()))?
                    .remove("PATH").unwrap_or_default(),
            };
            let path = merge_process_path([parent.as_os_str(), std::ffi::OsStr::new(&existing)])?;
            configured.retain(|key, _| if cfg!(windows) { !key.eq_ignore_ascii_case("PATH") } else { key != "PATH" });
            configured.insert("PATH".into(), path.to_string_lossy().into_owned());
        }
        let preparation = async {
            if let Some(runtime) = required_project_runtime(&directory, &record.settings) {
                nomifun_runtime::prepare_process_launch(runtime, &[], &directory, &configured).await?;
            }
            // Preserve the existing explicit shell contract, using the same
            // cwd-aware environment preparation as native Agent/Terminal/MCP.
            nomifun_runtime::prepare_process_launch(if cfg!(windows) { "cmd.exe" } else { "/bin/sh" }, &[], &directory, &configured).await
        };
        let prepared = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(AppError::Cancelled),
            result = tokio::time::timeout(PREPARATION_TIMEOUT, preparation) => result
                .map_err(|_| AppError::Command("application runtime preparation timed out before its command started".into()))?
                .map_err(|cause| AppError::Command(format!("could not prepare application environment: {cause}")))?,
        };
        if cancellation.is_cancelled() { return Err(AppError::Cancelled); }
        let environment = prepared.environment;
        let mut builder = shell_command(selected);
        builder.current_dir(&directory).envs(&environment);
        for key in ["NOMIFUN_HOST_URL", "NOMIFUN_LOCAL_TRUST", "NOMIFUN_AGENT_BINDING_ID"] { builder.env_remove(key); }
        if let Some(node) = custom_node {
            if selected.trim_start().starts_with("node ") {
                builder = custom_node_command(&node, selected.trim_start().strip_prefix("node ").unwrap(), &BTreeMap::new())?;
                builder.envs(&environment).env("NOMIFUN_SELECTED_NODE", &node).current_dir(&directory);
                for key in ["NOMIFUN_HOST_URL", "NOMIFUN_LOCAL_TRUST", "NOMIFUN_AGENT_BINDING_ID"] { builder.env_remove(key); }
            }
        }
        if inject_connection {
            let connection = self.connection.read().unwrap_or_else(|e| e.into_inner()).clone();
            builder.env("NOMIFUN_HOST_URL", connection.host_url).env("NOMIFUN_LOCAL_TRUST", connection.local_trust);
            if let Some(binding) = &record.settings.agent_binding_id { builder.env("NOMIFUN_AGENT_BINDING_ID", binding); }
            if selected.contains(".nomifun-static-server.cjs") {
                if let Some(output) = record.settings.output_directory.as_deref() {
                    builder.env("NOMIFUN_STATIC_ROOT", output);
                }
            }
        }
        let mut process = builder.spawn_managed().map_err(|error| AppError::Command(format!("could not run command: {error}")))?;
        let identity = match capture_child_identity(process.child()) {
            Ok(identity) => identity,
            Err(error) => {
                process.shutdown().await.map_err(|cleanup| AppError::OutcomeUnknown(format!("spawned process cleanup could not be confirmed: {cleanup}")))?;
                return Err(AppError::Command(format!("cannot confirm spawned process identity: {error}")));
            }
        };
        let mut output = Vec::new();
        let mut readers = Vec::new();
        if let Some(stdout) = process.stdout.take() {
            let buffer = Arc::new(StdMutex::new(Vec::new()));
            readers.push(read_output(stdout, buffer.clone()));
            output.push(buffer);
        }
        if let Some(stderr) = process.stderr.take() {
            let buffer = Arc::new(StdMutex::new(Vec::new()));
            readers.push(read_output(stderr, buffer.clone()));
            output.push(buffer);
        }
        let running = Arc::new(Running { generation: uuid::Uuid::now_v7().to_string(), process: Mutex::new(process), output, readers: Mutex::new(readers) });
        {
            let mut data = slot.data.lock().await;
            data.record.pid = Some(identity.pid);
            data.identity = Some(identity);
            data.running = Some(running.clone());
            if let Err(error) = self.persist_data(&data) {
                drop(data);
                shutdown_running(&running).await?;
                self.clear_process(slot, &running).await?;
                return Err(error);
            }
        }
        Ok(running)
    }

    fn monitor(self: &Arc<Self>, slot: Arc<AppSlot>, running: Arc<Running>) {
        let owner = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(150)).await;
                let Some(owner) = owner.upgrade() else { return; };
                if !slot.data.lock().await.running.as_ref().is_some_and(|current| current.generation == running.generation) { return; }
                let outcome = { running.process.lock().await.try_wait() };
                let reason = match outcome {
                    Ok(None) => continue,
                    Ok(Some(status)) => format!("foreground application exited unexpectedly ({status})"),
                    Err(error) => format!("cannot confirm application process state: {error}"),
                };
                // Serialize terminal publication with user start/stop. A late
                // observer cannot overwrite the successor process's state.
                let _gate = slot.gate.lock().await;
                if !slot.data.lock().await.running.as_ref().is_some_and(|current| current.generation == running.generation) { return; }
                let error = match shutdown_running(&running).await {
                    Ok(()) => {
                        if let Err(error) = owner.clear_process(&slot, &running).await { error }
                        else { AppError::Command(reason) }
                    }
                    Err(error) => AppError::OutcomeUnknown(format!("{reason}; cleanup not confirmed: {error}")),
                };
                let _ = owner.record_failure(&slot, &error).await;
                return;
            }
        });
    }

    async fn clear_process(&self, slot: &AppSlot, running: &Running) -> Result<(), AppError> {
        let mut data = slot.data.lock().await;
        if data.running.as_ref().is_some_and(|current| current.generation == running.generation) {
            let output = self.sanitized_output(running, &data.record.settings);
            append_output(&mut data.record.output, &output);
            data.running = None;
            data.identity = None;
            data.record.pid = None;
            self.persist_data(&data)?;
        }
        Ok(())
    }

    async fn stop_slot(&self, slot: &AppSlot) -> Result<(), AppError> {
        let (running, identity) = {
            let data = slot.data.lock().await;
            (data.running.clone(), data.identity.clone())
        };
        if let Some(running) = running {
            shutdown_running(&running).await?;
            self.clear_process(slot, &running).await?;
        } else if let Some(identity) = identity {
            let outcome = tokio::task::spawn_blocking(move || terminate_verified_orphan(&identity, CLEANUP_TIMEOUT)).await
                .map_err(|error| AppError::OutcomeUnknown(format!("process cleanup worker failed: {error}")))?
                .map_err(|error| AppError::OutcomeUnknown(format!("previous process cleanup could not be confirmed: {error}")))?;
            if outcome == OrphanTerminationOutcome::RootGoneCleanupUnproven {
                return Err(AppError::OutcomeUnknown("previous process root exited but its contained group cleanup remains unproven".into()));
            }
        }
        let mut data = slot.data.lock().await;
        data.identity = None;
        data.record.pid = None;
        data.record.state = AppState::Stopped;
        data.record.operation = None;
        data.record.preview = false;
        data.record.last_error = None;
        self.persist_data(&data)
    }

    async fn record_failure(&self, slot: &AppSlot, error: &AppError) -> Result<(), AppError> {
        let mut data = slot.data.lock().await;
        data.record.operation = None;
        if matches!(error, AppError::Cancelled) {
            data.record.state = AppState::Stopped;
            data.record.last_error = None;
        } else {
            data.record.state = AppState::Failed;
            data.record.last_error = Some(self.sanitize(&error.to_string(), &data.record.settings));
        }
        self.persist_data(&data)
    }

    fn snapshot(&self, data: &AppData) -> AppRecord {
        let mut record = data.record.clone();
        if let Some(running) = &data.running { append_output(&mut record.output, &self.sanitized_output(running, &record.settings)); }
        record.output = self.sanitize(&record.output, &record.settings);
        record
    }

    fn sanitized_output(&self, running: &Running, settings: &AppSettings) -> String {
        let mut output = String::new();
        // Keep streams separate until redaction so fragmented stdout writes
        // cannot be interleaved with stderr into an unrecognizable credential.
        for buffer in &running.output {
            let raw = buffer.lock().unwrap_or_else(|e| e.into_inner());
            append_output(&mut output, &self.sanitize(&crate::decode_output(&raw), settings));
        }
        output
    }

    fn sanitize(&self, value: &str, settings: &AppSettings) -> String {
        let secret = self.connection.read().unwrap_or_else(|e| e.into_inner()).local_trust.clone();
        let mut text = replace_secret(value, &secret);
        for (key, value) in &settings.environment {
            if !value.is_empty() && ["TOKEN", "SECRET", "PASSWORD", "KEY"].iter().any(|part| key.to_uppercase().contains(part)) {
                text = replace_secret(&text, value);
            }
        }
        nomi_redact::redact_secrets_owned(text)
    }

    async fn persist(&self, slot: &AppSlot) -> Result<(), AppError> {
        let data = slot.data.lock().await;
        self.persist_data(&data)
    }

    fn persist_data(&self, data: &AppData) -> Result<(), AppError> {
        let record = self.snapshot(data);
        let bytes = serde_json::to_vec_pretty(&StoredRecord { record, identity: data.identity.clone() })?;
        let path = self.registry_path(&data.record.id);
        self.ensure_owned_path(&path)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::now_v7()));
        let result = (|| {
            use std::io::Write;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            Ok::<_, std::io::Error>(())
        })();
        if result.is_err() { let _ = fs::remove_file(temporary); }
        result.map_err(AppError::Io)
    }
}

fn command(value: &Option<String>) -> Option<&str> { value.as_deref().map(str::trim).filter(|s| !s.is_empty()) }

fn local_web_addresses(value: Option<&str>) -> Vec<std::net::SocketAddr> {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    let Some(url) = value.and_then(|value| url::Url::parse(value).ok()) else { return Vec::new(); };
    if !matches!(url.scheme(), "http" | "https") { return Vec::new(); }
    let Some(port) = url.port_or_known_default() else { return Vec::new(); };
    let addresses = match url.host() {
        Some(url::Host::Ipv4(address)) if address.is_loopback() => vec![IpAddr::V4(address)],
        Some(url::Host::Ipv6(address)) if address.is_loopback() => vec![IpAddr::V6(address)],
        // Never perform hostname resolution or follow a remote address. A
        // localhost URL may serve either local IP family on this machine.
        Some(url::Host::Domain("localhost")) => vec![IpAddr::V4(Ipv4Addr::LOCALHOST), IpAddr::V6(Ipv6Addr::LOCALHOST)],
        _ => Vec::new(),
    };
    addresses.into_iter().map(|address| SocketAddr::new(address, port)).collect()
}

async fn wait_for_local_web_listener(value: Option<&str>, running: &Running, cancellation: &CancellationToken) -> Result<(), AppError> {
    let addresses = local_web_addresses(value);
    if addresses.is_empty() { return Ok(()); }
    let wait = async {
        loop {
            if cancellation.is_cancelled() { return Err(AppError::Cancelled); }
            // Preserve normal process-exit handling below rather than waiting
            // for a dead launcher or calling a TCP observation application health.
            if running.process.lock().await.try_wait()?.is_some() { return Ok(()); }
            for address in &addresses {
                tokio::select! {
                    _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                    connected = tokio::time::timeout(LOCAL_CONNECT_ATTEMPT, tokio::net::TcpStream::connect(address)) => {
                        if let Ok(Ok(stream)) = connected { drop(stream); return Ok(()); }
                    }
                }
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Err(AppError::Cancelled),
                _ = tokio::time::sleep(LOCAL_CONNECT_ATTEMPT) => {}
            }
        }
    };
    // A bounded TCP listen observation only prevents the common first-frame
    // connection-refused race. No HTTP/TLS body or credentials are sent, and
    // timeout keeps the existing process-based Running semantics.
    match tokio::time::timeout(LOCAL_LISTEN_WAIT, wait).await {
        Ok(result) => result,
        Err(_) => Ok(()),
    }
}

fn validate_execution_change(has_process: bool, current: &AppSettings, requested: &AppSettings) -> Result<(), AppError> {
    if has_process && (current.project_directory != requested.project_directory
        || current.install_command != requested.install_command
        || current.preview_command != requested.preview_command
        || current.build_command != requested.build_command
        || current.output_directory != requested.output_directory
        || current.start_command != requested.start_command
        || current.node_path != requested.node_path
        || current.environment != requested.environment
        || current.agent_id != requested.agent_id
        || current.agent_binding_id != requested.agent_binding_id) {
        return Err(AppError::Invalid("stop the application before changing its execution settings".into()));
    }
    Ok(())
}

pub(crate) fn validate_name(value: String) -> Result<String, AppError> {
    let value = value.trim().to_owned();
    if value.is_empty() || value.contains('\0') {
        return Err(AppError::Invalid("application name is required and cannot contain a null byte".into()));
    }
    Ok(value)
}

pub(crate) fn validate_metadata(mut value: AppMetadata) -> Result<AppMetadata, AppError> {
    if value.description.chars().count() > 4000 || value.description.contains('\0') {
        return Err(AppError::Invalid("application description exceeds 4000 characters or contains a null byte".into()));
    }
    if value.tags.len() > 20 { return Err(AppError::Invalid("an application supports at most 20 tags".into())); }
    let mut tags = Vec::new();
    for tag in value.tags {
        let tag = tag.trim().to_owned();
        if tag.is_empty() || tag.chars().count() > 64 || tag.contains(['\0', '\n', '\r']) {
            return Err(AppError::Invalid("application tags must contain 1 to 64 characters on one line".into()));
        }
        if !tags.contains(&tag) { tags.push(tag); }
    }
    value.tags = tags;
    Ok(value)
}

pub(crate) fn validate_id(id: &str) -> Result<(), AppError> {
    let identity = uuid::Uuid::parse_str(id).map_err(|_| AppError::Invalid("invalid application identity".into()))?;
    if identity.get_version_num() != 7 || identity.to_string() != id {
        return Err(AppError::Invalid("application identity must be a bare canonical UUIDv7".into()));
    }
    Ok(())
}

pub(crate) fn validate_settings(root: &Path, settings: &AppSettings) -> Result<(), AppError> {
    contained_directory(root, &settings.project_directory)?;
    if let Some(output) = settings.output_directory.as_deref().filter(|s| !s.is_empty()) {
        let project = root.join(relative_path(&settings.project_directory)?);
        let output = project.join(relative_path(output)?);
        ensure_no_links(root, &output)?;
        if output.exists() && !output.is_dir() {
            return Err(AppError::Invalid("output directory is not a directory".into()));
        }
    }
    for key in settings.environment.keys() {
        if key.is_empty() || key.contains(['=', '\0']) || matches!(key.to_uppercase().as_str(),
            "NOMIFUN_HOST_URL" | "NOMIFUN_LOCAL_TRUST" | "NOMIFUN_AGENT_BINDING_ID"
            | "NOMIFUN_HOSTING_COMMAND" | "NOMIFUN_SELECTED_NODE" | "NOMIFUN_STATIC_ROOT" | "NOMIFUN_VERSION_COMMAND") {
            return Err(AppError::Invalid(format!("environment variable is reserved or invalid: {key}")));
        }
    }
    for value in settings.environment.values() {
        if value.contains('\0') { return Err(AppError::Invalid("environment value contains a null byte".into())); }
    }
    Ok(())
}

fn shell_command(selected: &str) -> ChildProcessBuilder {
    #[cfg(windows)]
    {
        let mut builder = ChildProcessBuilder::clean_cli("cmd.exe");
        builder.env_clear();
        // Rust's normal argv quoting inserts backslashes before quotes; cmd
        // treats those as literal characters. Expand one environment argument
        // so user command quoting reaches cmd unchanged.
        builder.env("NOMIFUN_HOSTING_COMMAND", selected)
            .args(["/d", "/s", "/c", "%NOMIFUN_HOSTING_COMMAND%"]);
        builder
    }
    #[cfg(not(windows))]
    {
        let mut builder = ChildProcessBuilder::clean_cli("/bin/sh");
        builder.env_clear();
        builder.args(["-c", selected]);
        builder
    }
}

fn custom_node_command(node: &Path, arguments: &str, environment: &BTreeMap<String, String>) -> Result<ChildProcessBuilder, AppError> {
    // Keep ordinary shell argument semantics while substituting only the
    // selected executable. Its path is passed through environment, not quoted
    // into user command text.
    #[cfg(windows)]
    let selected = format!("\"{}\" {arguments}", node.display());
    #[cfg(not(windows))]
    let selected = format!("\"$NOMIFUN_SELECTED_NODE\" {arguments}");
    let mut builder = shell_command(&selected);
    builder.envs(environment).env("NOMIFUN_SELECTED_NODE", node);
    Ok(builder)
}

fn read_output<R: AsyncRead + Unpin + Send + 'static>(mut reader: R, output: Arc<StdMutex<Vec<u8>>>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut chunk = [0u8; 4096];
        loop {
            let count = match reader.read(&mut chunk).await { Ok(0) | Err(_) => break, Ok(count) => count };
            let mut buffer = output.lock().unwrap_or_else(|e| e.into_inner());
            buffer.extend_from_slice(&chunk[..count]);
            if buffer.len() > OUTPUT_LIMIT {
                // Drop complete lines, never a prefix of a credential. A
                // single pathological line is discarded rather than leaked.
                let excess = buffer.len() - OUTPUT_LIMIT;
                if let Some(newline) = buffer[excess..].iter().position(|b| *b == b'\n') { buffer.drain(..excess + newline + 1); }
                else { buffer.clear(); }
            }
        }
    })
}

fn append_output(target: &mut String, next: &str) {
    target.push_str(next);
    if target.len() > OUTPUT_LIMIT {
        let mut cut = target.len() - OUTPUT_LIMIT;
        while !target.is_char_boundary(cut) { cut += 1; }
        target.drain(..cut);
    }
}

fn replace_secret(value: &str, secret: &str) -> String {
    if secret.is_empty() { return value.to_owned(); }
    let mut text = value.replace(secret, "[REDACTED_SECRET]");
    // A snapshot can land between two writes of the same credential. Withhold
    // a trailing prefix until the next write instead of exposing it in logs.
    for boundary in (1..secret.len()).rev() {
        if secret.is_char_boundary(boundary) && text.ends_with(&secret[..boundary]) {
            text.truncate(text.len() - boundary);
            text.push_str("[REDACTED_SECRET]");
            break;
        }
    }
    text
}

async fn shutdown_running(running: &Running) -> Result<(), AppError> {
    tokio::time::timeout(CLEANUP_TIMEOUT, running.process.lock().await.shutdown()).await
        .map_err(|_| AppError::OutcomeUnknown("process tree shutdown timed out; cleanup is not confirmed".into()))?
        .map_err(|error| AppError::OutcomeUnknown(format!("process tree shutdown failed; cleanup is not confirmed: {error}")))?;
    let readers = std::mem::take(&mut *running.readers.lock().await);
    for mut reader in readers {
        if tokio::time::timeout(Duration::from_secs(2), &mut reader).await.is_err() { reader.abort(); }
    }
    Ok(())
}

#[cfg(test)]
mod local_listen_tests {
    use super::local_web_addresses;

    #[test]
    fn local_listener_addresses_accept_only_http_loopback_without_dns_or_url_content() {
        for url in ["http://127.0.0.1:8123/path", "http://127.1:8123", "https://localhost:8123", "http://[::1]:8123"] {
            let addresses = local_web_addresses(Some(url));
            assert!(!addresses.is_empty());
            assert!(addresses.iter().all(|address| address.ip().is_loopback() && address.port() == 8123));
        }
        assert_eq!(local_web_addresses(Some("http://localhost"))[0].port(), 80);
        assert_eq!(local_web_addresses(Some("https://localhost"))[0].port(), 443);
        assert_eq!(local_web_addresses(Some("https://user:credential@localhost:8123/path?secret=ignored")),
            local_web_addresses(Some("https://localhost:8123")), "connection targets never include credentials, path or query");
        for url in [None, Some("invalid"), Some("ftp://localhost:8123"), Some("http://example.invalid:8123"), Some("http://192.0.2.1:8123"), Some("http://localhost.example.invalid:8123")] {
            assert!(local_web_addresses(url).is_empty());
        }
    }
}
