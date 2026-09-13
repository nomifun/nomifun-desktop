use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, MiniAppAdditiveMigrationAction, MiniAppBridgeKvRequest, MiniAppDatabaseHandleId,
    MiniAppFilesDirDescriptor, MiniAppFilesHandleId, MiniAppId, MiniAppKvHandleDescriptor,
    MiniAppKvHandleId, MiniAppKvResponse, MiniAppMigration, MiniAppMigrationId,
    MiniAppPrivateDatabaseDescriptor, MiniAppReleaseRef, MiniAppServiceStorageDescriptor,
    StrictJsonValue, digest_payload,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::runtime::{
    PluginRuntimeBackupStorage, PluginRuntimeCallCancellation, PluginRuntimeHostKvPort,
    PluginRuntimePlatformError, PluginRuntimePlatformResult,
};

/// The storage descriptor and ledger resolved for one exact Service run.
///
/// Handles are Host-owned. The descriptor is passed into the canonical
/// `ResolvedMiniAppServiceSpec`; raw database paths never leave the Host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeServiceStorageResolution {
    pub descriptor: MiniAppServiceStorageDescriptor,
    pub migration_ledger: Option<PluginRuntimeMigrationLedger>,
}

/// Isolated storage materialized for one transient Service Test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeServiceTestStorageResolution {
    pub descriptor: MiniAppServiceStorageDescriptor,
    pub copied_kv_digest: DigestHex,
    pub copied_private_database_digest: Option<DigestHex>,
    pub empty_files_dir: Option<bool>,
    pub migration_ledger: Option<PluginRuntimeMigrationLedger>,
}

impl PluginRuntimeServiceStorageResolution {
    pub fn host_kv(miniapp_id: MiniAppId) -> Self {
        Self {
            descriptor: MiniAppServiceStorageDescriptor {
                kv: MiniAppKvHandleDescriptor {
                    handle_id: MiniAppKvHandleId::from(format!(
                        "miniapp-kv-{}",
                        miniapp_id.as_ref()
                    )),
                    miniapp_id,
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
            migration_ledger: None,
        }
    }
}

/// Requests issued by a trusted Node Service back to the Host over the private
/// Service IPC channel. The MiniApp and owner are selected by the Host process
/// binding, never by this payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PluginRuntimeServiceStorageRequest {
    Kv {
        request: MiniAppBridgeKvRequest,
    },
    DatabaseQuery {
        statement: PluginRuntimeDatabaseStatement,
    },
    DatabaseExecute {
        statement: PluginRuntimeDatabaseStatement,
    },
    DatabaseBatch {
        statements: Vec<PluginRuntimeDatabaseStatement>,
    },
}

/// Production/runtime boundary for owner-scoped MiniApp storage.
///
/// The in-memory implementation below remains useful for deterministic
/// contract tests. Production composition supplies the SQLite/filesystem
/// implementation from `managed_storage.rs`.
#[async_trait]
pub trait PluginRuntimeServiceStoragePort: Send + Sync {
    async fn resolve_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceStorageResolution>;

    async fn apply_additive_migrations(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger>;

    async fn handle_service_request(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: PluginRuntimeServiceStorageRequest,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<StrictJsonValue>;

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceTestStorageResolution> {
        let _ = (
            owner_user_id,
            miniapp_id,
            test_id,
            uses_files,
            uses_private_database,
        );
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Test storage is not configured".into(),
        ))
    }

    async fn purge_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
    ) -> PluginRuntimePlatformResult<()> {
        let _ = (owner_user_id, miniapp_id, test_id);
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Test storage is not configured".into(),
        ))
    }

    async fn purge_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<()>;

    async fn export_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeBackupStorage> {
        let _ = (owner_user_id, miniapp_id);
        if uses_files || uses_private_database {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin backup storage is not configured".into(),
            ));
        }
        Ok(PluginRuntimeBackupStorage {
            kv: Vec::new(),
            files: Vec::new(),
            private_database: None,
            migration_ledger: None,
        })
    }

    async fn import_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: PluginRuntimeBackupStorage,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<()> {
        let _ = (owner_user_id, miniapp_id);
        if uses_files || uses_private_database
            || !storage.files.is_empty()
            || storage.private_database.is_some()
            || storage.migration_ledger.is_some()
        {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin backup storage is not configured".into(),
            ));
        }
        Ok(())
    }
}

#[async_trait]
pub trait PluginRuntimeFilesPort: Send + Sync {
    async fn resolve(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> PluginRuntimePlatformResult<MiniAppFilesDirDescriptor>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeDatabaseStatement {
    pub sql: String,
    pub parameters: StrictJsonValue,
}

impl PluginRuntimeDatabaseStatement {
    pub fn validate_query(&self) -> PluginRuntimePlatformResult<()> {
        self.validate_common()?;
        if !starts_with_keyword(&self.sql, "SELECT") {
            return Err(PluginRuntimePlatformError::InvalidDatabaseRequest(
                "query accepts a single SELECT statement".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_execute(&self) -> PluginRuntimePlatformResult<()> {
        self.validate_common()?;
        if !["INSERT", "UPDATE", "DELETE", "REPLACE"]
            .iter()
            .any(|keyword| starts_with_keyword(&self.sql, keyword))
        {
            return Err(PluginRuntimePlatformError::InvalidDatabaseRequest(
                "execute accepts a single DML statement".into(),
            ));
        }
        Ok(())
    }

    fn validate_common(&self) -> PluginRuntimePlatformResult<()> {
        let sql = self.sql.trim();
        if sql.is_empty()
            || sql.contains(';')
            || contains_forbidden_sql(sql)
            || !self.parameters.0.is_array()
        {
            return Err(PluginRuntimePlatformError::InvalidDatabaseRequest(
                "statement must be parameterized, single-statement SQL without DDL, PRAGMA, ATTACH, or transaction control".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginRuntimeDatabaseQueryResult {
    pub rows: Vec<StrictJsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeDatabaseExecuteResult {
    pub affected_rows: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeMigrationLedgerEntry {
    pub ordinal: u64,
    pub migration_id: MiniAppMigrationId,
    pub migration_digest: DigestHex,
    pub release: MiniAppReleaseRef,
    pub applied_at_ms: i64,
}

#[derive(Serialize)]
struct PluginRuntimeMigrationLedgerDigestInput<'a> {
    miniapp_id: &'a MiniAppId,
    handle_id: &'a MiniAppDatabaseHandleId,
    schema_epoch: u64,
    entries: &'a [PluginRuntimeMigrationLedgerEntry],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginRuntimeMigrationLedger {
    pub miniapp_id: MiniAppId,
    pub handle_id: MiniAppDatabaseHandleId,
    pub schema_epoch: u64,
    pub entries: Vec<PluginRuntimeMigrationLedgerEntry>,
    pub ledger_digest: DigestHex,
}

impl PluginRuntimeMigrationLedger {
    pub fn empty(
        miniapp_id: MiniAppId,
        handle_id: MiniAppDatabaseHandleId,
        schema_epoch: u64,
    ) -> PluginRuntimePlatformResult<Self> {
        if schema_epoch == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Private Database schema epoch must be positive".into(),
            ));
        }
        let entries = Vec::new();
        let ledger_digest = ledger_digest(&miniapp_id, &handle_id, schema_epoch, &entries)?;
        Ok(Self {
            miniapp_id,
            handle_id,
            schema_epoch,
            entries,
            ledger_digest,
        })
    }

    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.schema_epoch == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Private Database schema epoch must be positive".into(),
            ));
        }
        let expected_schema_epoch = u64::try_from(self.entries.len())
            .ok()
            .and_then(|length| length.checked_add(1))
            .ok_or_else(|| {
                PluginRuntimePlatformError::InvalidState(
                    "Private Database migration ledger is too large".into(),
                )
            })?;
        if self.schema_epoch != expected_schema_epoch {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Private Database schema epoch does not match its migration ledger".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.ordinal != index as u64 + 1
                || entry.applied_at_ms <= 0
                || entry.migration_id.as_ref().trim().is_empty()
                || !is_digest(&entry.migration_digest)
                || entry.release.validate().is_err()
                || !ids.insert(entry.migration_id.clone())
            {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "migration ledger entries must be unique, ordered, and timestamped".into(),
                ));
            }
        }
        let expected = ledger_digest(
            &self.miniapp_id,
            &self.handle_id,
            self.schema_epoch,
            &self.entries,
        )?;
        if expected != self.ledger_digest {
            return Err(PluginRuntimePlatformError::InvalidState(
                "migration ledger digest mismatch".into(),
            ));
        }
        Ok(())
    }

    pub fn append_additive(
        &self,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<Self> {
        self.validate()?;
        release.validate()?;
        if applied_at_ms <= 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "migration application time must be positive".into(),
            ));
        }
        let existing = self
            .entries
            .iter()
            .map(|entry| (entry.migration_id.clone(), entry.migration_digest.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut incoming = BTreeSet::new();
        let mut entries = self.entries.clone();
        for migration in migrations {
            migration.validate()?;
            if !incoming.insert(migration.migration_id.clone()) {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "migration set contains duplicate identities".into(),
                ));
            }
            if let Some(digest) = existing.get(&migration.migration_id) {
                if digest != &migration.migration_digest {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "an applied migration identity cannot change digest".into(),
                    ));
                }
                continue;
            }
            entries.push(PluginRuntimeMigrationLedgerEntry {
                ordinal: entries.len() as u64 + 1,
                migration_id: migration.migration_id.clone(),
                migration_digest: migration.migration_digest.clone(),
                release: release.clone(),
                applied_at_ms,
            });
        }
        if entries.len() == self.entries.len() {
            return Ok(self.clone());
        }
        let schema_epoch = self
            .schema_epoch
            .checked_add(1)
            .ok_or_else(|| PluginRuntimePlatformError::InvalidState(
                "Private Database schema epoch overflow".into(),
            ))?;
        let ledger_digest =
            ledger_digest(&self.miniapp_id, &self.handle_id, schema_epoch, &entries)?;
        let next = Self {
            miniapp_id: self.miniapp_id.clone(),
            handle_id: self.handle_id.clone(),
            schema_epoch,
            entries,
            ledger_digest,
        };
        next.validate()?;
        Ok(next)
    }
}

fn is_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[async_trait]
pub trait PluginRuntimePrivateDatabasePort: Send + Sync {
    async fn query(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: PluginRuntimeDatabaseStatement,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<PluginRuntimeDatabaseQueryResult>;

    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: PluginRuntimeDatabaseStatement,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<PluginRuntimeDatabaseExecuteResult>;

    async fn batch(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statements: Vec<PluginRuntimeDatabaseStatement>,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<Vec<PluginRuntimeDatabaseExecuteResult>>;

    async fn apply_additive_migrations(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger>;

    async fn ledger(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger>;
}

#[derive(Clone)]
struct KvNamespace {
    descriptor: MiniAppKvHandleDescriptor,
    values: BTreeMap<String, KvCell>,
}

#[derive(Clone)]
struct KvCell {
    revision: u64,
    value: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct InMemoryTestStorageKey {
    owner_user_id: String,
    miniapp_id: String,
    test_id: String,
}

#[derive(Clone)]
struct DatabaseState {
    descriptor: MiniAppPrivateDatabaseDescriptor,
    ledger: PluginRuntimeMigrationLedger,
    statements: Vec<PluginRuntimeDatabaseStatement>,
}

#[derive(Default)]
struct ManagedStorageState {
    kv: BTreeMap<MiniAppKvHandleId, KvNamespace>,
    files: BTreeMap<MiniAppFilesHandleId, MiniAppFilesDirDescriptor>,
    databases: BTreeMap<MiniAppDatabaseHandleId, DatabaseState>,
    test_storage: BTreeMap<InMemoryTestStorageKey, MiniAppServiceStorageDescriptor>,
}

/// Contract-focused in-memory storage. It enforces owner/handle isolation,
/// KV CAS, SQL admission, and append-only migration ledgers. It deliberately
/// does not pretend to be SQLite; the production adapter remains a later port.
#[derive(Default)]
pub struct InMemoryPluginRuntimeManagedStorage {
    state: Mutex<ManagedStorageState>,
}

impl InMemoryPluginRuntimeManagedStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(
        &self,
        storage: MiniAppServiceStorageDescriptor,
        ledger: Option<PluginRuntimeMigrationLedger>,
    ) -> PluginRuntimePlatformResult<()> {
        let owner = storage.kv.miniapp_id.clone();
        if storage
            .files_dir
            .as_ref()
            .is_some_and(|descriptor| descriptor.miniapp_id != owner)
            || storage
                .private_database
                .as_ref()
                .is_some_and(|descriptor| descriptor.miniapp_id != owner)
        {
            return Err(PluginRuntimePlatformError::UnknownStorageHandle);
        }
        let database = match (&storage.private_database, &ledger) {
            (Some(database), Some(ledger)) => {
                ledger.validate()?;
                if ledger.miniapp_id != owner
                    || ledger.handle_id != database.handle_id
                    || ledger.schema_epoch != database.schema_epoch
                    || ledger.ledger_digest != database.migration_ledger_digest
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Private Database descriptor and migration ledger differ".into(),
                    ));
                }
                Some((database.clone(), ledger.clone()))
            }
            (None, None) => None,
            _ => {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "Private Database registration requires its exact migration ledger".into(),
                ));
            }
        };

        let mut state = self.state.lock().await;
        if state
            .kv
            .get(&storage.kv.handle_id)
            .is_some_and(|namespace| namespace.descriptor != storage.kv)
            || storage.files_dir.as_ref().is_some_and(|files| {
                state
                    .files
                    .get(&files.handle_id)
                    .is_some_and(|existing| existing != files)
            })
            || database.as_ref().is_some_and(|(descriptor, ledger)| {
                state
                    .databases
                    .get(&descriptor.handle_id)
                    .is_some_and(|existing| {
                        existing.descriptor != *descriptor || existing.ledger != *ledger
                    })
            })
        {
            return Err(PluginRuntimePlatformError::UnknownStorageHandle);
        }
        state
            .kv
            .entry(storage.kv.handle_id.clone())
            .or_insert_with(|| KvNamespace {
                descriptor: storage.kv,
                values: BTreeMap::new(),
            });
        if let Some(files) = storage.files_dir {
            state.files.entry(files.handle_id.clone()).or_insert(files);
        }
        if let Some((database, ledger)) = database {
            state
                .databases
                .entry(database.handle_id.clone())
                .or_insert_with(|| DatabaseState {
                    descriptor: database,
                    ledger,
                    statements: Vec::new(),
                });
        }
        Ok(())
    }

    pub async fn recorded_statements(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> PluginRuntimePlatformResult<Vec<PluginRuntimeDatabaseStatement>> {
        let state = self.state.lock().await;
        let database = owned_database(&state, miniapp_id, handle_id)?;
        Ok(database.statements.clone())
    }
}

#[async_trait]
impl PluginRuntimeHostKvPort for InMemoryPluginRuntimeManagedStorage {
    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: &MiniAppBridgeKvRequest,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        if &storage.kv.miniapp_id != miniapp_id {
            return Err(PluginRuntimePlatformError::UnknownStorageHandle);
        }
        let mut state = self.state.lock().await;
        let namespace = state
            .kv
            .get_mut(&storage.kv.handle_id)
            .filter(|namespace| {
                namespace.descriptor == storage.kv
                    && &namespace.descriptor.miniapp_id == miniapp_id
            })
            .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
        let response = match request {
            MiniAppBridgeKvRequest::Get { key } => {
                let value = namespace.values.get(key);
                MiniAppKvResponse::Value {
                    value: value.map(|cell| cell.value.clone()),
                    revision: value.map(|cell| cell.revision),
                }
            }
            MiniAppBridgeKvRequest::Set { key, value } => {
                let revision = next_kv_revision(namespace.values.get(key).map(|cell| cell.revision))?;
                namespace.values.insert(
                    key.clone(),
                    KvCell {
                        revision,
                        value: value.clone(),
                    },
                );
                MiniAppKvResponse::Written { revision }
            }
            MiniAppBridgeKvRequest::Delete { key } => MiniAppKvResponse::Deleted {
                existed: namespace.values.remove(key).is_some(),
            },
            MiniAppBridgeKvRequest::CompareAndSwap {
                key,
                expected_revision,
                value,
            } => {
                let observed = namespace.values.get(key).map(|cell| cell.revision);
                let applied = &observed == expected_revision;
                if applied {
                    match value {
                        Some(value) => {
                            let revision = next_kv_revision(observed)?;
                            namespace.values.insert(
                                key.clone(),
                                KvCell {
                                    revision,
                                    value: value.clone(),
                                },
                            );
                        }
                        None => {
                            namespace.values.remove(key);
                        }
                    }
                }
                MiniAppKvResponse::CompareAndSwap {
                    applied,
                    current_revision: namespace.values.get(key).map(|cell| cell.revision),
                }
            }
        };
        Ok(StrictJsonValue(serde_json::to_value(response).map_err(
            |error| PluginRuntimePlatformError::Runtime(error.to_string()),
        )?))
    }
}

pub(crate) fn next_kv_revision(current_revision: Option<u64>) -> PluginRuntimePlatformResult<u64> {
    current_revision
        .map_or(Some(1), |revision| revision.checked_add(1))
        .ok_or(PluginRuntimePlatformError::KvRevisionOverflow)
}

#[async_trait]
impl PluginRuntimeFilesPort for InMemoryPluginRuntimeManagedStorage {
    async fn resolve(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> PluginRuntimePlatformResult<MiniAppFilesDirDescriptor> {
        self.state
            .lock()
            .await
            .files
            .get(handle_id)
            .filter(|descriptor| &descriptor.miniapp_id == miniapp_id)
            .cloned()
            .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)
    }
}

#[async_trait]
impl PluginRuntimePrivateDatabasePort for InMemoryPluginRuntimeManagedStorage {
    async fn query(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: PluginRuntimeDatabaseStatement,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<PluginRuntimeDatabaseQueryResult> {
        statement.validate_query()?;
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        commit_database_effect(&cancellation, || {
            database.statements.push(statement);
            PluginRuntimeDatabaseQueryResult { rows: Vec::new() }
        })
    }

    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: PluginRuntimeDatabaseStatement,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<PluginRuntimeDatabaseExecuteResult> {
        statement.validate_execute()?;
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        commit_database_effect(&cancellation, || {
            database.statements.push(statement);
            PluginRuntimeDatabaseExecuteResult { affected_rows: 0 }
        })
    }

    async fn batch(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statements: Vec<PluginRuntimeDatabaseStatement>,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<Vec<PluginRuntimeDatabaseExecuteResult>> {
        if statements.is_empty() || statements.len() > 64 {
            return Err(PluginRuntimePlatformError::InvalidDatabaseRequest(
                "batch requires 1..=64 DML statements".into(),
            ));
        }
        for statement in &statements {
            statement.validate_execute()?;
        }
        let count = statements.len();
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        commit_database_effect(&cancellation, || {
            database.statements.extend(statements);
            vec![
                PluginRuntimeDatabaseExecuteResult { affected_rows: 0 };
                count
            ]
        })
    }

    async fn apply_additive_migrations(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        if &database.ledger.ledger_digest != expected_ledger_digest {
            return Err(PluginRuntimePlatformError::StorageConflict);
        }
        validate_additive_actions(migrations)?;
        let next = database
            .ledger
            .append_additive(release, migrations, applied_at_ms)?;
        database.descriptor.schema_epoch = next.schema_epoch;
        database.descriptor.migration_ledger_digest = next.ledger_digest.clone();
        database.ledger = next.clone();
        Ok(next)
    }

    async fn ledger(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
        let state = self.state.lock().await;
        Ok(owned_database(&state, miniapp_id, handle_id)?.ledger.clone())
    }
}

#[async_trait]
impl PluginRuntimeServiceStoragePort for InMemoryPluginRuntimeManagedStorage {
    async fn resolve_service_storage(
        &self,
        _owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceStorageResolution> {
        let mut resolution = PluginRuntimeServiceStorageResolution::host_kv(miniapp_id.clone());
        if uses_files {
            let path = std::env::temp_dir()
                .join("nomifun-miniapp-memory")
                .join(miniapp_id.as_ref())
                .join("files");
            std::fs::create_dir_all(&path).map_err(|error| {
                PluginRuntimePlatformError::Runtime(format!(
                    "cannot create in-memory filesDir fixture: {error}"
                ))
            })?;
            resolution.descriptor.files_dir = Some(MiniAppFilesDirDescriptor {
                handle_id: MiniAppFilesHandleId::from(format!(
                    "miniapp-files-{}",
                    miniapp_id.as_ref()
                )),
                miniapp_id: miniapp_id.clone(),
                absolute_path: path.display().to_string(),
            });
        }
        if uses_private_database {
            let handle_id =
                MiniAppDatabaseHandleId::from(format!("miniapp-db-{}", miniapp_id.as_ref()));
            let ledger = PluginRuntimeMigrationLedger::empty(
                miniapp_id.clone(),
                handle_id.clone(),
                1,
            )?;
            resolution.descriptor.private_database =
                Some(MiniAppPrivateDatabaseDescriptor {
                    handle_id,
                    miniapp_id: miniapp_id.clone(),
                    schema_epoch: ledger.schema_epoch,
                    migration_ledger_digest: ledger.ledger_digest.clone(),
                });
            resolution.migration_ledger = Some(ledger);
        }
        self.register(
            resolution.descriptor.clone(),
            resolution.migration_ledger.clone(),
        )
        .await?;
        Ok(resolution)
    }

    async fn apply_additive_migrations(
        &self,
        _owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
        let database = storage
            .private_database
            .as_ref()
            .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
        PluginRuntimePrivateDatabasePort::apply_additive_migrations(
            self,
            miniapp_id,
            &database.handle_id,
            expected_ledger_digest,
            release,
            migrations,
            applied_at_ms,
        )
        .await
    }

    async fn handle_service_request(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: PluginRuntimeServiceStorageRequest,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        let value = match request {
            PluginRuntimeServiceStorageRequest::Kv { request } => {
                return PluginRuntimeHostKvPort::execute(self, miniapp_id, storage, &request).await;
            }
            PluginRuntimeServiceStorageRequest::DatabaseQuery { statement } => {
                let database = storage
                    .private_database
                    .as_ref()
                    .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
                serde_json::to_value(
                    PluginRuntimePrivateDatabasePort::query(
                        self,
                        miniapp_id,
                        &database.handle_id,
                        statement,
                        cancellation,
                    )
                    .await?,
                )
            }
            PluginRuntimeServiceStorageRequest::DatabaseExecute { statement } => {
                let database = storage
                    .private_database
                    .as_ref()
                    .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
                serde_json::to_value(
                    PluginRuntimePrivateDatabasePort::execute(
                        self,
                        miniapp_id,
                        &database.handle_id,
                        statement,
                        cancellation,
                    )
                    .await?,
                )
            }
            PluginRuntimeServiceStorageRequest::DatabaseBatch { statements } => {
                let database = storage
                    .private_database
                    .as_ref()
                    .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
                serde_json::to_value(
                    PluginRuntimePrivateDatabasePort::batch(
                        self,
                        miniapp_id,
                        &database.handle_id,
                        statements,
                        cancellation,
                    )
                    .await?,
                )
            }
        }
        .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))?;
        Ok(StrictJsonValue(value))
    }

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceTestStorageResolution> {
        validate_service_test_id(test_id)?;
        let production = self
            .resolve_service_storage(
                owner_user_id,
                miniapp_id,
                uses_files,
                uses_private_database,
            )
            .await?;
        self.purge_service_test_storage(owner_user_id, miniapp_id, test_id)
            .await?;

        let test_key = InMemoryTestStorageKey {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.as_ref().to_owned(),
            test_id: test_id.to_owned(),
        };
        let kv_handle_id = MiniAppKvHandleId::from(format!(
            "miniapp-test-kv-{}-{test_id}",
            miniapp_id.as_ref()
        ));
        let mut state = self.state.lock().await;
        let production_kv = state
            .kv
            .get(&production.descriptor.kv.handle_id)
            .filter(|namespace| namespace.descriptor == production.descriptor.kv)
            .cloned()
            .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
        let copied_kv_digest = service_test_kv_digest(
            production_kv
                .values
                .iter()
                .map(|(key, cell)| PluginRuntimeServiceTestKvSnapshotEntry {
                    key: key.clone(),
                    value: cell.value.clone(),
                    revision: cell.revision,
                    key_generation: 1,
                    is_tombstone: false,
                })
                .collect(),
        )?;
        let kv = MiniAppKvHandleDescriptor {
            handle_id: kv_handle_id.clone(),
            miniapp_id: miniapp_id.clone(),
            namespace_revision: 1,
        };
        state.kv.insert(
            kv_handle_id,
            KvNamespace {
                descriptor: kv.clone(),
                values: production_kv.values,
            },
        );

        let files_dir = if uses_files {
            let path =
                in_memory_service_test_files_path(owner_user_id, miniapp_id, test_id);
            std::fs::create_dir_all(&path).map_err(|error| {
                PluginRuntimePlatformError::Runtime(format!(
                    "cannot create in-memory Service Test filesDir fixture: {error}"
                ))
            })?;
            let descriptor = MiniAppFilesDirDescriptor {
                handle_id: MiniAppFilesHandleId::from(format!(
                    "miniapp-test-files-{}-{test_id}",
                    miniapp_id.as_ref()
                )),
                miniapp_id: miniapp_id.clone(),
                absolute_path: path.display().to_string(),
            };
            state
                .files
                .insert(descriptor.handle_id.clone(), descriptor.clone());
            Some(descriptor)
        } else {
            None
        };

        let (private_database, copied_private_database_digest, migration_ledger) =
            if uses_private_database {
                let production_database = production
                    .descriptor
                    .private_database
                    .as_ref()
                    .and_then(|descriptor| state.databases.get(&descriptor.handle_id))
                    .cloned()
                    .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)?;
                let copied_private_database_digest =
                    digest_payload(&InMemoryDatabaseSnapshotDigestInput {
                        ledger: &production_database.ledger,
                        statements: &production_database.statements,
                    })
                    .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))?;
                let handle_id = MiniAppDatabaseHandleId::from(format!(
                    "miniapp-test-db-{}-{test_id}",
                    miniapp_id.as_ref()
                ));
                let ledger = rebind_migration_ledger(&production_database.ledger, handle_id.clone())?;
                let descriptor = MiniAppPrivateDatabaseDescriptor {
                    handle_id: handle_id.clone(),
                    miniapp_id: miniapp_id.clone(),
                    schema_epoch: ledger.schema_epoch,
                    migration_ledger_digest: ledger.ledger_digest.clone(),
                };
                state.databases.insert(
                    handle_id,
                    DatabaseState {
                        descriptor: descriptor.clone(),
                        ledger: ledger.clone(),
                        statements: production_database.statements,
                    },
                );
                (Some(descriptor), Some(copied_private_database_digest), Some(ledger))
            } else {
                (None, None, None)
            };
        let descriptor = MiniAppServiceStorageDescriptor {
            kv,
            files_dir,
            private_database,
        };
        state.test_storage.insert(test_key, descriptor.clone());
        Ok(PluginRuntimeServiceTestStorageResolution {
            descriptor,
            copied_kv_digest,
            copied_private_database_digest,
            empty_files_dir: uses_files.then_some(true),
            migration_ledger,
        })
    }

    async fn purge_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
    ) -> PluginRuntimePlatformResult<()> {
        validate_service_test_id(test_id)?;
        let key = InMemoryTestStorageKey {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.as_ref().to_owned(),
            test_id: test_id.to_owned(),
        };
        let descriptor = self.state.lock().await.test_storage.get(&key).cloned();
        let file_path = descriptor
            .as_ref()
            .and_then(|storage| storage.files_dir.as_ref())
            .map(|files| std::path::PathBuf::from(&files.absolute_path))
            .unwrap_or_else(|| {
                in_memory_service_test_files_path(owner_user_id, miniapp_id, test_id)
            });
        remove_in_memory_managed_directory(&file_path)?;
        let mut state = self.state.lock().await;
        let descriptor = state.test_storage.remove(&key);
        if let Some(storage) = descriptor {
            state.kv.remove(&storage.kv.handle_id);
            if let Some(files) = storage.files_dir {
                state.files.remove(&files.handle_id);
            }
            if let Some(database) = storage.private_database {
                state.databases.remove(&database.handle_id);
            }
        }
        Ok(())
    }

    async fn purge_service_storage(
        &self,
        _owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<()> {
        let mut state = self.state.lock().await;
        let file_paths = state
            .files
            .values()
            .filter(|descriptor| &descriptor.miniapp_id == miniapp_id)
            .map(|descriptor| descriptor.absolute_path.clone())
            .collect::<Vec<_>>();
        state
            .test_storage
            .retain(|key, _| key.miniapp_id != miniapp_id.as_ref());
        state
            .kv
            .retain(|_, namespace| &namespace.descriptor.miniapp_id != miniapp_id);
        state
            .files
            .retain(|_, descriptor| &descriptor.miniapp_id != miniapp_id);
        state
            .databases
            .retain(|_, database| &database.descriptor.miniapp_id != miniapp_id);
        drop(state);
        for path in file_paths {
            remove_in_memory_managed_directory(std::path::Path::new(&path))?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct InMemoryDatabaseSnapshotDigestInput<'a> {
    ledger: &'a PluginRuntimeMigrationLedger,
    statements: &'a [PluginRuntimeDatabaseStatement],
}

#[derive(Serialize)]
pub(crate) struct PluginRuntimeServiceTestKvSnapshotEntry {
    pub key: String,
    pub value: StrictJsonValue,
    pub revision: u64,
    pub key_generation: u64,
    pub is_tombstone: bool,
}

pub(crate) fn service_test_kv_digest(
    entries: Vec<PluginRuntimeServiceTestKvSnapshotEntry>,
) -> PluginRuntimePlatformResult<DigestHex> {
    digest_payload(&entries).map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))
}

pub(crate) fn validate_service_test_id(test_id: &str) -> PluginRuntimePlatformResult<()> {
    if test_id.is_empty()
        || test_id.len() > 64
        || !test_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(PluginRuntimePlatformError::InvalidState(
            "Service Test identity must contain 1 to 64 ASCII letters, digits, '-' or '_'".into(),
        ));
    }
    Ok(())
}

fn rebind_migration_ledger(
    source: &PluginRuntimeMigrationLedger,
    handle_id: MiniAppDatabaseHandleId,
) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
    let ledger_digest = ledger_digest(
        &source.miniapp_id,
        &handle_id,
        source.schema_epoch,
        &source.entries,
    )?;
    let rebound = PluginRuntimeMigrationLedger {
        miniapp_id: source.miniapp_id.clone(),
        handle_id,
        schema_epoch: source.schema_epoch,
        entries: source.entries.clone(),
        ledger_digest,
    };
    rebound.validate()?;
    Ok(rebound)
}

pub(crate) fn rebind_migration_ledger_for_target(
    source: &PluginRuntimeMigrationLedger,
    miniapp_id: MiniAppId,
    handle_id: MiniAppDatabaseHandleId,
) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
    rebind_migration_ledger_for_target_with_releases(
        source,
        miniapp_id,
        handle_id,
        &BTreeMap::new(),
    )
}

pub(crate) fn rebind_migration_ledger_for_target_with_releases(
    source: &PluginRuntimeMigrationLedger,
    miniapp_id: MiniAppId,
    handle_id: MiniAppDatabaseHandleId,
    release_refs: &BTreeMap<String, MiniAppReleaseRef>,
) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
    source.validate()?;
    let entries = source
        .entries
        .iter()
        .map(|entry| {
            let release = if release_refs.is_empty() {
                entry.release.clone()
            } else {
                release_refs
                    .get(entry.release.release_id.as_ref())
                    .cloned()
                    .ok_or_else(|| {
                        PluginRuntimePlatformError::InvalidState(format!(
                            "migration ledger references a Release absent from the imported backup: {}",
                            entry.release.release_id.as_ref()
                        ))
                    })?
            };
            Ok(PluginRuntimeMigrationLedgerEntry {
                ordinal: entry.ordinal,
                migration_id: entry.migration_id.clone(),
                migration_digest: entry.migration_digest.clone(),
                release,
                applied_at_ms: entry.applied_at_ms,
            })
        })
        .collect::<PluginRuntimePlatformResult<Vec<_>>>()?;
    let ledger_digest = ledger_digest(
        &miniapp_id,
        &handle_id,
        source.schema_epoch,
        &entries,
    )?;
    let rebound = PluginRuntimeMigrationLedger {
        miniapp_id,
        handle_id,
        schema_epoch: source.schema_epoch,
        entries,
        ledger_digest,
    };
    rebound.validate()?;
    Ok(rebound)
}

fn remove_in_memory_managed_directory(path: &std::path::Path) -> PluginRuntimePlatformResult<()> {
    let root = std::env::temp_dir().join("nomifun-miniapp-memory");
    let relative = path.strip_prefix(&root).map_err(|_| {
        PluginRuntimePlatformError::InvalidState(
            "in-memory filesDir fixture escaped its managed root".into(),
        )
    })?;
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if in_memory_reparse_or_symlink(&metadata) || !metadata.is_dir() => {
            return Err(PluginRuntimePlatformError::InvalidState(
                "in-memory filesDir fixture root is not a regular directory".into(),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(PluginRuntimePlatformError::Runtime(format!(
                "cannot inspect in-memory filesDir fixture root: {error}"
            )));
        }
    }
    let mut current = root;
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(PluginRuntimePlatformError::InvalidState(
                "in-memory filesDir fixture contains a non-normal component".into(),
            ));
        };
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata)
                if in_memory_reparse_or_symlink(&metadata) || !metadata.is_dir() =>
            {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "in-memory filesDir fixture changed before purge".into(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(PluginRuntimePlatformError::Runtime(format!(
                    "cannot inspect in-memory filesDir fixture: {error}"
                )));
            }
        }
    }
    validate_in_memory_removal_tree(path)?;
    std::fs::remove_dir_all(path).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!(
            "cannot purge in-memory filesDir fixture: {error}"
        ))
    })
}

fn validate_in_memory_removal_tree(path: &std::path::Path) -> PluginRuntimePlatformResult<()> {
    for entry in std::fs::read_dir(path).map_err(|error| {
        PluginRuntimePlatformError::Runtime(format!(
            "cannot read in-memory filesDir fixture: {error}"
        ))
    })? {
        let entry = entry.map_err(|error| {
            PluginRuntimePlatformError::Runtime(format!(
                "cannot inspect in-memory filesDir fixture entry: {error}"
            ))
        })?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            PluginRuntimePlatformError::Runtime(format!(
                "cannot inspect in-memory filesDir fixture entry: {error}"
            ))
        })?;
        if in_memory_reparse_or_symlink(&metadata) {
            return Err(PluginRuntimePlatformError::InvalidState(
                "in-memory filesDir fixture contains a symlink or reparse point".into(),
            ));
        }
        if metadata.is_dir() {
            validate_in_memory_removal_tree(&entry.path())?;
        } else if !metadata.is_file() {
            return Err(PluginRuntimePlatformError::InvalidState(
                "in-memory filesDir fixture contains a special file".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn in_memory_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn in_memory_reparse_or_symlink(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn in_memory_service_test_files_path(
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    test_id: &str,
) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("nomifun-miniapp-memory")
        .join(owner_user_id)
        .join(miniapp_id.as_ref())
        .join("service-tests")
        .join(test_id)
        .join("files")
}

fn owned_database<'a>(
    state: &'a ManagedStorageState,
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
) -> PluginRuntimePlatformResult<&'a DatabaseState> {
    state
        .databases
        .get(handle_id)
        .filter(|database| &database.descriptor.miniapp_id == miniapp_id)
        .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)
}

fn owned_database_mut<'a>(
    state: &'a mut ManagedStorageState,
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
) -> PluginRuntimePlatformResult<&'a mut DatabaseState> {
    state
        .databases
        .get_mut(handle_id)
        .filter(|database| &database.descriptor.miniapp_id == miniapp_id)
        .ok_or(PluginRuntimePlatformError::UnknownStorageHandle)
}

fn ensure_not_canceled(cancellation: &PluginRuntimeCallCancellation) -> PluginRuntimePlatformResult<()> {
    if cancellation.is_canceled() {
        Err(PluginRuntimePlatformError::Canceled)
    } else {
        Ok(())
    }
}

fn commit_database_effect<T>(
    cancellation: &PluginRuntimeCallCancellation,
    commit: impl FnOnce() -> T,
) -> PluginRuntimePlatformResult<T> {
    ensure_not_canceled(cancellation)?;
    Ok(commit())
}

#[cfg(test)]
pub(crate) fn test_database_commit_boundary(
    cancellation: &PluginRuntimeCallCancellation,
) -> PluginRuntimePlatformResult<()> {
    commit_database_effect(cancellation, || cancellation.cancel())
}

fn ledger_digest(
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
    schema_epoch: u64,
    entries: &[PluginRuntimeMigrationLedgerEntry],
) -> PluginRuntimePlatformResult<DigestHex> {
    digest_payload(&PluginRuntimeMigrationLedgerDigestInput {
        miniapp_id,
        handle_id,
        schema_epoch,
        entries,
    })
    .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))
}

fn starts_with_keyword(sql: &str, keyword: &str) -> bool {
    sql.trim_start()
        .get(..keyword.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(keyword))
        && sql
            .trim_start()
            .as_bytes()
            .get(keyword.len())
            .is_none_or(u8::is_ascii_whitespace)
}

fn contains_forbidden_sql(sql: &str) -> bool {
    let upper = sql.to_ascii_uppercase();
    [
        "ATTACH",
        "DETACH",
        "PRAGMA",
        "CREATE",
        "ALTER",
        "DROP",
        "VACUUM",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "SAVEPOINT",
        "RELEASE",
        "LOAD_EXTENSION",
    ]
    .iter()
    .any(|keyword| {
        upper
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|token| token == *keyword)
    })
}

fn validate_additive_actions(migrations: &[MiniAppMigration]) -> PluginRuntimePlatformResult<()> {
    for migration in migrations {
        migration.validate()?;
        for action in &migration.actions {
            match action {
                MiniAppAdditiveMigrationAction::CreateTable { .. }
                | MiniAppAdditiveMigrationAction::CreateIndex { .. }
                | MiniAppAdditiveMigrationAction::AddColumn { .. } => {}
            }
        }
    }
    Ok(())
}
