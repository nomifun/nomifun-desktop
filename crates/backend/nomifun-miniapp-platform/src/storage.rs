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

use crate::{
    MiniAppCallCancellation, MiniAppHostKvPort, MiniAppPlatformError, MiniAppPlatformResult,
};

#[async_trait]
pub trait MiniAppFilesPort: Send + Sync {
    async fn resolve(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> MiniAppPlatformResult<MiniAppFilesDirDescriptor>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppDatabaseStatement {
    pub sql: String,
    pub parameters: StrictJsonValue,
}

impl MiniAppDatabaseStatement {
    pub fn validate_query(&self) -> MiniAppPlatformResult<()> {
        self.validate_common()?;
        if !starts_with_keyword(&self.sql, "SELECT") {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(
                "query accepts a single SELECT statement".into(),
            ));
        }
        Ok(())
    }

    pub fn validate_execute(&self) -> MiniAppPlatformResult<()> {
        self.validate_common()?;
        if !["INSERT", "UPDATE", "DELETE", "REPLACE"]
            .iter()
            .any(|keyword| starts_with_keyword(&self.sql, keyword))
        {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(
                "execute accepts a single DML statement".into(),
            ));
        }
        Ok(())
    }

    fn validate_common(&self) -> MiniAppPlatformResult<()> {
        let sql = self.sql.trim();
        if sql.is_empty()
            || sql.contains(';')
            || contains_forbidden_sql(sql)
            || !self.parameters.0.is_array()
        {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(
                "statement must be parameterized, single-statement SQL without DDL, PRAGMA, ATTACH, or transaction control".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MiniAppDatabaseQueryResult {
    pub rows: Vec<StrictJsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppDatabaseExecuteResult {
    pub affected_rows: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppMigrationLedgerEntry {
    pub ordinal: u64,
    pub migration_id: MiniAppMigrationId,
    pub migration_digest: DigestHex,
    pub release: MiniAppReleaseRef,
    pub applied_at_ms: i64,
}

#[derive(Serialize)]
struct MiniAppMigrationLedgerDigestInput<'a> {
    miniapp_id: &'a MiniAppId,
    handle_id: &'a MiniAppDatabaseHandleId,
    schema_epoch: u64,
    entries: &'a [MiniAppMigrationLedgerEntry],
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniAppMigrationLedger {
    pub miniapp_id: MiniAppId,
    pub handle_id: MiniAppDatabaseHandleId,
    pub schema_epoch: u64,
    pub entries: Vec<MiniAppMigrationLedgerEntry>,
    pub ledger_digest: DigestHex,
}

impl MiniAppMigrationLedger {
    pub fn empty(
        miniapp_id: MiniAppId,
        handle_id: MiniAppDatabaseHandleId,
        schema_epoch: u64,
    ) -> MiniAppPlatformResult<Self> {
        if schema_epoch == 0 {
            return Err(MiniAppPlatformError::InvalidState(
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

    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        if self.schema_epoch == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "Private Database schema epoch must be positive".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if entry.ordinal != index as u64 + 1
                || entry.applied_at_ms <= 0
                || !ids.insert(entry.migration_id.clone())
            {
                return Err(MiniAppPlatformError::InvalidState(
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
            return Err(MiniAppPlatformError::InvalidState(
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
    ) -> MiniAppPlatformResult<Self> {
        self.validate()?;
        if applied_at_ms <= 0 {
            return Err(MiniAppPlatformError::InvalidState(
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
                return Err(MiniAppPlatformError::InvalidState(
                    "migration set contains duplicate identities".into(),
                ));
            }
            if let Some(digest) = existing.get(&migration.migration_id) {
                if digest != &migration.migration_digest {
                    return Err(MiniAppPlatformError::InvalidState(
                        "an applied migration identity cannot change digest".into(),
                    ));
                }
                continue;
            }
            entries.push(MiniAppMigrationLedgerEntry {
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
            .ok_or_else(|| MiniAppPlatformError::InvalidState(
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

#[async_trait]
pub trait MiniAppPrivateDatabasePort: Send + Sync {
    async fn query(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseQueryResult>;

    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseExecuteResult>;

    async fn batch(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statements: Vec<MiniAppDatabaseStatement>,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<Vec<MiniAppDatabaseExecuteResult>>;

    async fn apply_additive_migrations(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger>;

    async fn ledger(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger>;
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

#[derive(Clone)]
struct DatabaseState {
    descriptor: MiniAppPrivateDatabaseDescriptor,
    ledger: MiniAppMigrationLedger,
    statements: Vec<MiniAppDatabaseStatement>,
}

#[derive(Default)]
struct ManagedStorageState {
    kv: BTreeMap<MiniAppKvHandleId, KvNamespace>,
    files: BTreeMap<MiniAppFilesHandleId, MiniAppFilesDirDescriptor>,
    databases: BTreeMap<MiniAppDatabaseHandleId, DatabaseState>,
}

/// Contract-focused in-memory storage. It enforces owner/handle isolation,
/// KV CAS, SQL admission, and append-only migration ledgers. It deliberately
/// does not pretend to be SQLite; the production adapter remains a later port.
#[derive(Default)]
pub struct InMemoryMiniAppManagedStorage {
    state: Mutex<ManagedStorageState>,
}

impl InMemoryMiniAppManagedStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(
        &self,
        storage: MiniAppServiceStorageDescriptor,
        ledger: Option<MiniAppMigrationLedger>,
    ) -> MiniAppPlatformResult<()> {
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
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let database = match (&storage.private_database, &ledger) {
            (Some(database), Some(ledger)) => {
                ledger.validate()?;
                if ledger.miniapp_id != owner
                    || ledger.handle_id != database.handle_id
                    || ledger.schema_epoch != database.schema_epoch
                    || ledger.ledger_digest != database.migration_ledger_digest
                {
                    return Err(MiniAppPlatformError::InvalidState(
                        "Private Database descriptor and migration ledger differ".into(),
                    ));
                }
                Some((database.clone(), ledger.clone()))
            }
            (None, None) => None,
            _ => {
                return Err(MiniAppPlatformError::InvalidState(
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
            return Err(MiniAppPlatformError::UnknownStorageHandle);
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
    ) -> MiniAppPlatformResult<Vec<MiniAppDatabaseStatement>> {
        let state = self.state.lock().await;
        let database = owned_database(&state, miniapp_id, handle_id)?;
        Ok(database.statements.clone())
    }
}

#[async_trait]
impl MiniAppHostKvPort for InMemoryMiniAppManagedStorage {
    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: &MiniAppBridgeKvRequest,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        if &storage.kv.miniapp_id != miniapp_id {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let mut state = self.state.lock().await;
        let namespace = state
            .kv
            .get_mut(&storage.kv.handle_id)
            .filter(|namespace| {
                namespace.descriptor == storage.kv
                    && &namespace.descriptor.miniapp_id == miniapp_id
            })
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
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
            |error| MiniAppPlatformError::Runtime(error.to_string()),
        )?))
    }
}

pub(crate) fn next_kv_revision(current_revision: Option<u64>) -> MiniAppPlatformResult<u64> {
    current_revision
        .map_or(Some(1), |revision| revision.checked_add(1))
        .ok_or(MiniAppPlatformError::KvRevisionOverflow)
}

#[async_trait]
impl MiniAppFilesPort for InMemoryMiniAppManagedStorage {
    async fn resolve(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> MiniAppPlatformResult<MiniAppFilesDirDescriptor> {
        self.state
            .lock()
            .await
            .files
            .get(handle_id)
            .filter(|descriptor| &descriptor.miniapp_id == miniapp_id)
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)
    }
}

#[async_trait]
impl MiniAppPrivateDatabasePort for InMemoryMiniAppManagedStorage {
    async fn query(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseQueryResult> {
        statement.validate_query()?;
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        commit_database_effect(&cancellation, || {
            database.statements.push(statement);
            MiniAppDatabaseQueryResult { rows: Vec::new() }
        })
    }

    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseExecuteResult> {
        statement.validate_execute()?;
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        commit_database_effect(&cancellation, || {
            database.statements.push(statement);
            MiniAppDatabaseExecuteResult { affected_rows: 0 }
        })
    }

    async fn batch(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statements: Vec<MiniAppDatabaseStatement>,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<Vec<MiniAppDatabaseExecuteResult>> {
        if statements.is_empty() || statements.len() > 64 {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(
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
                MiniAppDatabaseExecuteResult { affected_rows: 0 };
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
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        let mut state = self.state.lock().await;
        let database = owned_database_mut(&mut state, miniapp_id, handle_id)?;
        if &database.ledger.ledger_digest != expected_ledger_digest {
            return Err(MiniAppPlatformError::StorageConflict);
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
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        let state = self.state.lock().await;
        Ok(owned_database(&state, miniapp_id, handle_id)?.ledger.clone())
    }
}

fn owned_database<'a>(
    state: &'a ManagedStorageState,
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
) -> MiniAppPlatformResult<&'a DatabaseState> {
    state
        .databases
        .get(handle_id)
        .filter(|database| &database.descriptor.miniapp_id == miniapp_id)
        .ok_or(MiniAppPlatformError::UnknownStorageHandle)
}

fn owned_database_mut<'a>(
    state: &'a mut ManagedStorageState,
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
) -> MiniAppPlatformResult<&'a mut DatabaseState> {
    state
        .databases
        .get_mut(handle_id)
        .filter(|database| &database.descriptor.miniapp_id == miniapp_id)
        .ok_or(MiniAppPlatformError::UnknownStorageHandle)
}

fn ensure_not_canceled(cancellation: &MiniAppCallCancellation) -> MiniAppPlatformResult<()> {
    if cancellation.is_canceled() {
        Err(MiniAppPlatformError::Canceled)
    } else {
        Ok(())
    }
}

fn commit_database_effect<T>(
    cancellation: &MiniAppCallCancellation,
    commit: impl FnOnce() -> T,
) -> MiniAppPlatformResult<T> {
    ensure_not_canceled(cancellation)?;
    Ok(commit())
}

#[cfg(test)]
pub(crate) fn test_database_commit_boundary(
    cancellation: &MiniAppCallCancellation,
) -> MiniAppPlatformResult<()> {
    commit_database_effect(cancellation, || cancellation.cancel())
}

fn ledger_digest(
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
    schema_epoch: u64,
    entries: &[MiniAppMigrationLedgerEntry],
) -> MiniAppPlatformResult<DigestHex> {
    digest_payload(&MiniAppMigrationLedgerDigestInput {
        miniapp_id,
        handle_id,
        schema_epoch,
        entries,
    })
    .map_err(|error| MiniAppPlatformError::Runtime(error.to_string()))
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

fn validate_additive_actions(migrations: &[MiniAppMigration]) -> MiniAppPlatformResult<()> {
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
