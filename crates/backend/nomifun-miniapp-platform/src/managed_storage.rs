use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    DigestHex, MiniAppAdditiveMigrationAction, MiniAppBridgeKvRequest, MiniAppDatabaseHandleId,
    MiniAppFilesDirDescriptor, MiniAppFilesHandleId, MiniAppId, MiniAppKvResponse,
    MiniAppMigration, MiniAppMigrationId, MiniAppPrivateDatabaseDescriptor, MiniAppReleaseRef,
    MiniAppServiceStorageDescriptor, StrictJsonValue, digest_payload,
};
use nomifun_db::{MiniAppKvRow, SqlitePool};
use rusqlite::{
    Connection,
    hooks::{AuthAction, AuthContext, Authorization},
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
};
use serde::Serialize;
use serde_json::Value as JsonValue;
use tokio::sync::{Mutex, RwLock};

use crate::{
    MiniAppCallCancellation, MiniAppDatabaseExecuteResult, MiniAppDatabaseQueryResult,
    MiniAppDatabaseStatement, MiniAppFilesPort, MiniAppHostKvPort, MiniAppMigrationLedger,
    MiniAppPlatformError, MiniAppPlatformResult, MiniAppPrivateDatabasePort,
    MiniAppServiceStoragePort, MiniAppServiceStorageRequest, MiniAppServiceStorageResolution,
};
use crate::MiniAppMigrationLedgerEntry;

const FILES_DIRECTORY: &str = "files";
const DATABASES_DIRECTORY: &str = "databases";
const LEDGER_TABLE: &str = "__nomifun_migration_ledger";
const META_TABLE: &str = "__nomifun_storage_meta";
const SCHEMA_EPOCH_KEY: &str = "schema_epoch";
const MAX_DATABASE_BATCH: usize = 64;
const MAX_DATABASE_RESULT_ROWS: usize = 10_000;
const MAX_DATABASE_RESULT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct RegisteredStorage {
    owner_user_id: String,
    descriptor: MiniAppServiceStorageDescriptor,
    database_path: Option<PathBuf>,
    database_lock: Arc<Mutex<()>>,
}

/// Production owner-scoped MiniApp storage.
///
/// Host KV stays in the canonical M1 SQLite database. Service files live in a
/// stable per-MiniApp directory. Private SQLite is a separate file whose path
/// is never included in the HTTP contract or exposed to the renderer.
#[derive(Clone)]
pub struct SqliteMiniAppManagedStorage {
    root: Arc<PathBuf>,
    pool: SqlitePool,
    registrations: Arc<RwLock<BTreeMap<String, RegisteredStorage>>>,
}

impl std::fmt::Debug for SqliteMiniAppManagedStorage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SqliteMiniAppManagedStorage")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

impl SqliteMiniAppManagedStorage {
    pub fn new(
        root: impl AsRef<Path>,
        pool: SqlitePool,
    ) -> MiniAppPlatformResult<Self> {
        let requested = root.as_ref();
        if !requested.is_absolute() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp managed storage root must be absolute".into(),
            ));
        }
        ensure_directory(requested)?;
        let root = fs::canonicalize(requested).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot canonicalize MiniApp managed storage root: {error}"
            ))
        })?;
        ensure_directory(&root.join(FILES_DIRECTORY))?;
        ensure_directory(&root.join(DATABASES_DIRECTORY))?;
        Ok(Self {
            root: Arc::new(root),
            pool,
            registrations: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    async fn registration(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        descriptor: &MiniAppServiceStorageDescriptor,
    ) -> MiniAppPlatformResult<RegisteredStorage> {
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration.owner_user_id != owner_user_id
            || registration.descriptor != *descriptor
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        Ok(registration)
    }

    async fn registration_for(
        &self,
        miniapp_id: &MiniAppId,
        descriptor: &MiniAppServiceStorageDescriptor,
    ) -> MiniAppPlatformResult<RegisteredStorage> {
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration.descriptor != *descriptor {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        Ok(registration)
    }

    async fn ensure_product_owner(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> MiniAppPlatformResult<()> {
        let found: Option<String> = nomifun_db::sqlx::query_scalar(
            "SELECT kind FROM miniapp_products
             WHERE owner_user_id = ? AND miniapp_id = ?",
        )
        .bind(owner_user_id)
        .bind(miniapp_id.as_ref())
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        if found.as_deref() != Some("service") {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        Ok(())
    }

    async fn execute_kv(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: MiniAppBridgeKvRequest,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        if storage.kv.miniapp_id != *miniapp_id {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let (namespace, key, operation) = match request {
            MiniAppBridgeKvRequest::Get { key } => {
                ("service".to_owned(), key, KvOperation::Get)
            }
            MiniAppBridgeKvRequest::Set { key, value } => (
                "service".to_owned(),
                key,
                KvOperation::Set { value: value.0 },
            ),
            MiniAppBridgeKvRequest::Delete { key } => {
                ("service".to_owned(), key, KvOperation::Delete)
            }
            MiniAppBridgeKvRequest::CompareAndSwap {
                key,
                expected_revision,
                value,
            } => (
                "service".to_owned(),
                key,
                KvOperation::CompareAndSwap {
                    expected_revision: expected_revision
                        .map(|value| {
                            i64::try_from(value).map_err(|_| {
                                MiniAppPlatformError::KvRevisionOverflow
                            })
                        })
                        .transpose()?,
                    value: value.map(|value| value.0),
                },
            ),
        };
        validate_visible_key(&namespace, 128)?;
        validate_visible_key(&key, 256)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        let current = fetch_kv(
            &mut transaction,
            owner_user_id,
            miniapp_id,
            &namespace,
            &key,
        )
        .await?;
        let now_ms = nomifun_common::now_ms().max(1);
        let response = match operation {
            KvOperation::Get => {
                let value = current
                    .as_ref()
                    .filter(|row| !row.is_tombstone)
                    .map(|row| {
                        serde_json::from_str(&row.value_json).map_err(|error| {
                            MiniAppPlatformError::Database(format!(
                                "MiniApp KV value is invalid JSON: {error}"
                            ))
                        })
                    })
                    .transpose()?;
                MiniAppKvResponse::Value {
                    value: value.map(StrictJsonValue),
                    revision: current
                        .as_ref()
                        .map(|row| positive_revision(row.revision))
                        .transpose()?,
                }
            }
            KvOperation::Set { value } => {
                let value_json = serde_json::to_string(&value).map_err(|error| {
                    MiniAppPlatformError::Database(format!(
                        "MiniApp KV value cannot be serialized: {error}"
                    ))
                })?;
                let revision = write_live_kv(
                    &mut transaction,
                    owner_user_id,
                    miniapp_id,
                    &namespace,
                    &key,
                    &value_json,
                    current.as_ref(),
                    now_ms,
                )
                .await?;
                MiniAppKvResponse::Written {
                    revision: positive_revision(revision)?,
                }
            }
            KvOperation::Delete => {
                let existed = match current.as_ref() {
                    Some(row) if !row.is_tombstone => {
                        tombstone_kv(
                            &mut transaction,
                            owner_user_id,
                            miniapp_id,
                            &namespace,
                            &key,
                            row,
                            now_ms,
                        )
                        .await?;
                        true
                    }
                    _ => false,
                };
                MiniAppKvResponse::Deleted { existed }
            }
            KvOperation::CompareAndSwap {
                expected_revision,
                value,
            } => {
                let observed = current
                    .as_ref()
                    .map(|row| positive_revision(row.revision))
                    .transpose()?;
                if observed.map(|value| i64::try_from(value).unwrap_or(i64::MAX))
                    != expected_revision
                {
                    MiniAppKvResponse::CompareAndSwap {
                        applied: false,
                        current_revision: observed,
                    }
                } else {
                    let revision = match value {
                        Some(value) => {
                            let value_json =
                                serde_json::to_string(&value).map_err(|error| {
                                    MiniAppPlatformError::Database(format!(
                                        "MiniApp KV value cannot be serialized: {error}"
                                    ))
                                })?;
                            write_live_kv(
                                &mut transaction,
                                owner_user_id,
                                miniapp_id,
                                &namespace,
                                &key,
                                &value_json,
                                current.as_ref(),
                                now_ms,
                            )
                            .await?
                        }
                        None => match current.as_ref() {
                            Some(row) if !row.is_tombstone => {
                                tombstone_kv(
                                    &mut transaction,
                                    owner_user_id,
                                    miniapp_id,
                                    &namespace,
                                    &key,
                                    row,
                                    now_ms,
                                )
                                .await?
                            }
                            Some(row) => row.revision,
                            None => 0,
                        },
                    };
                    MiniAppKvResponse::CompareAndSwap {
                        applied: true,
                        current_revision: (revision > 0)
                            .then(|| positive_revision(revision))
                            .transpose()?,
                    }
                }
            }
        };
        transaction
            .commit()
            .await
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        serde_json::to_value(response)
            .map(StrictJsonValue)
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))
    }

    async fn with_database<T, F>(
        &self,
        registration: RegisteredStorage,
        operation: F,
    ) -> MiniAppPlatformResult<T>
    where
        T: Send + 'static,
        F: FnOnce(Connection, Arc<AtomicBool>) -> Result<T, MiniAppPlatformError>
            + Send
            + 'static,
    {
        let path = registration
            .database_path
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        let _guard = registration.database_lock.lock().await;
        tokio::task::spawn_blocking(move || {
            let (connection, internal_mode) = open_private_database(&path)?;
            operation(connection, internal_mode)
        })
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?
    }
}

#[async_trait]
impl MiniAppServiceStoragePort for SqliteMiniAppManagedStorage {
    async fn resolve_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppServiceStorageResolution> {
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        let files_dir = if uses_files {
            validate_path_component(owner_user_id, "owner_user_id")?;
            validate_path_component(miniapp_id.as_ref(), "miniapp_id")?;
            let path = self
                .root
                .join(FILES_DIRECTORY)
                .join(owner_user_id)
                .join(miniapp_id.as_ref());
            ensure_directory(&path)?;
            let canonical = fs::canonicalize(&path).map_err(|error| {
                MiniAppPlatformError::Runtime(format!(
                    "cannot canonicalize MiniApp filesDir: {error}"
                ))
            })?;
            ensure_within(self.root(), &canonical)?;
            Some(MiniAppFilesDirDescriptor {
                handle_id: MiniAppFilesHandleId::from(format!(
                    "miniapp-files-{}",
                    miniapp_id.as_ref()
                )),
                miniapp_id: miniapp_id.clone(),
                absolute_path: canonical.display().to_string(),
            })
        } else {
            None
        };

        let (private_database, migration_ledger, database_path) =
            if uses_private_database {
                let path = self
                    .root
                    .join(DATABASES_DIRECTORY)
                    .join(owner_user_id)
                    .join(format!("{}.sqlite", miniapp_id.as_ref()));
                ensure_database_path(&path)?;
                ensure_within(
                    self.root(),
                    path.parent().ok_or_else(|| {
                        MiniAppPlatformError::InvalidState(
                            "MiniApp private database has no parent".into(),
                        )
                    })?,
                )?;
                let ledger = tokio::task::spawn_blocking({
                    let path = path.clone();
                    let miniapp_id = miniapp_id.clone();
                    move || {
                        let (connection, internal_mode) = open_private_database(&path)?;
                        with_internal_mode(&internal_mode, || {
                            read_ledger(&connection, &miniapp_id, &MiniAppDatabaseHandleId::from(
                                format!("miniapp-db-{}", miniapp_id.as_ref()),
                            ))
                        })
                    }
                })
                .await
                .map_err(|error| MiniAppPlatformError::Database(error.to_string()))??;
                let path = fs::canonicalize(&path).map_err(|error| {
                    MiniAppPlatformError::Runtime(format!(
                        "cannot canonicalize MiniApp private database: {error}"
                    ))
                })?;
                ensure_within(self.root(), &path)?;
                (
                    Some(MiniAppPrivateDatabaseDescriptor {
                        handle_id: MiniAppDatabaseHandleId::from(format!(
                            "miniapp-db-{}",
                            miniapp_id.as_ref()
                        )),
                        miniapp_id: miniapp_id.clone(),
                        schema_epoch: ledger.schema_epoch,
                        migration_ledger_digest: ledger.ledger_digest.clone(),
                    }),
                    Some(ledger),
                    Some(path),
                )
            } else {
                (None, None, None)
            };

        let descriptor = MiniAppServiceStorageDescriptor {
            kv: crate::MiniAppServiceStorageResolution::host_kv(miniapp_id.clone())
                .descriptor
                .kv,
            files_dir,
            private_database,
        };
        let registration = RegisteredStorage {
            owner_user_id: owner_user_id.to_owned(),
            descriptor: descriptor.clone(),
            database_path,
            database_lock: Arc::new(Mutex::new(())),
        };
        let mut registrations = self.registrations.write().await;
        if let Some(existing) = registrations.get(miniapp_id.as_ref())
            && existing.owner_user_id != owner_user_id
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        registrations.insert(miniapp_id.as_ref().to_owned(), registration);
        Ok(MiniAppServiceStorageResolution {
            descriptor,
            migration_ledger,
        })
    }

    async fn apply_additive_migrations(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        let registration = self
            .registration(owner_user_id, miniapp_id, storage)
            .await?;
        let database_path = registration
            .database_path
            .clone()
            .ok_or_else(|| {
                MiniAppPlatformError::InvalidState(
                    "MiniApp migrations require a Private Database".into(),
                )
            })?;
        let handle_id = storage
            .private_database
            .as_ref()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?
            .handle_id
            .clone();
        let _guard = registration.database_lock.lock().await;
        let miniapp_id_owned = miniapp_id.clone();
        let expected_ledger_digest_owned = expected_ledger_digest.clone();
        let release_owned = release.clone();
        let migrations_owned = migrations.to_vec();
        let next = tokio::task::spawn_blocking(move || {
            let (mut connection, internal_mode) = open_private_database(&database_path)?;
            with_internal_mode(&internal_mode, || {
                let current = read_ledger(&connection, &miniapp_id_owned, &handle_id)?;
                if current.ledger_digest != expected_ledger_digest_owned {
                    return Err(MiniAppPlatformError::StorageConflict);
                }
                let next =
                    current.append_additive(&release_owned, &migrations_owned, applied_at_ms)?;
                if next.entries.len() == current.entries.len() {
                    return Ok(current);
                }
                let existing = current
                    .entries
                    .iter()
                    .map(|entry| entry.migration_id.clone())
                    .collect::<BTreeSet<_>>();
                let transaction = connection.transaction().map_err(database_error)?;
                for migration in &migrations_owned {
                    if existing.contains(&migration.migration_id) {
                        continue;
                    }
                    for action in &migration.actions {
                        let sql = migration_sql(action)?;
                        transaction.execute_batch(&sql).map_err(database_error)?;
                    }
                    let entry = next
                        .entries
                        .iter()
                        .find(|entry| entry.migration_id == migration.migration_id)
                        .ok_or_else(|| {
                            MiniAppPlatformError::Database(
                                "migration ledger entry was not materialized".into(),
                            )
                        })?;
                    let release_json =
                        serde_json::to_string(&entry.release).map_err(database_error)?;
                    transaction
                        .execute(
                            &format!(
                                "INSERT INTO {LEDGER_TABLE}
                                 (ordinal, migration_id, migration_digest, release_json, applied_at_ms)
                                 VALUES (?1, ?2, ?3, ?4, ?5)"
                            ),
                            rusqlite::params![
                                i64::try_from(entry.ordinal).map_err(|_| {
                                    MiniAppPlatformError::Database(
                                        "migration ordinal overflow".into(),
                                    )
                                })?,
                                entry.migration_id.as_ref(),
                                entry.migration_digest.as_ref(),
                                release_json,
                                entry.applied_at_ms,
                            ],
                        )
                        .map_err(database_error)?;
                }
                transaction
                    .execute(
                        &format!(
                            "UPDATE {META_TABLE} SET value = ?1 WHERE key = ?2"
                        ),
                        rusqlite::params![
                            i64::try_from(next.schema_epoch).map_err(|_| {
                                MiniAppPlatformError::Database(
                                    "migration schema epoch overflow".into(),
                                )
                            })?,
                            SCHEMA_EPOCH_KEY,
                        ],
                    )
                    .map_err(database_error)?;
                transaction.commit().map_err(database_error)?;
                read_ledger(&connection, &miniapp_id_owned, &handle_id)
            })
        })
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))??;

        let mut registrations = self.registrations.write().await;
        let current = registrations
            .get_mut(miniapp_id.as_ref())
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        let database = current
            .descriptor
            .private_database
            .as_mut()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        database.schema_epoch = next.schema_epoch;
        database.migration_ledger_digest = next.ledger_digest.clone();
        Ok(next)
    }

    async fn handle_service_request(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: MiniAppServiceStorageRequest,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        let registration = self.registration_for(miniapp_id, storage).await?;
        match request {
            MiniAppServiceStorageRequest::Kv { request } => {
                self.execute_kv(
                    &registration.owner_user_id,
                    miniapp_id,
                    storage,
                    request,
                )
                .await
            }
            MiniAppServiceStorageRequest::DatabaseQuery { statement } => {
                let result = MiniAppPrivateDatabasePort::query(
                    self,
                        miniapp_id,
                        &storage
                            .private_database
                            .as_ref()
                            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?
                            .handle_id,
                        statement,
                        cancellation,
                    )
                    .await?;
                serde_json::to_value(result)
                    .map(StrictJsonValue)
                    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))
            }
            MiniAppServiceStorageRequest::DatabaseExecute { statement } => {
                let result = MiniAppPrivateDatabasePort::execute(
                    self,
                        miniapp_id,
                        &storage
                            .private_database
                            .as_ref()
                            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?
                            .handle_id,
                        statement,
                        cancellation,
                    )
                    .await?;
                serde_json::to_value(result)
                    .map(StrictJsonValue)
                    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))
            }
            MiniAppServiceStorageRequest::DatabaseBatch { statements } => {
                let result = MiniAppPrivateDatabasePort::batch(
                    self,
                        miniapp_id,
                        &storage
                            .private_database
                            .as_ref()
                            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?
                            .handle_id,
                        statements,
                        cancellation,
                    )
                    .await?;
                serde_json::to_value(result)
                    .map(StrictJsonValue)
                    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))
            }
        }
    }
}

#[async_trait]
impl MiniAppHostKvPort for SqliteMiniAppManagedStorage {
    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: &MiniAppBridgeKvRequest,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        let owner = self
            .registration_for(miniapp_id, storage)
            .await?
            .owner_user_id;
        self.execute_kv(&owner, miniapp_id, storage, request.clone())
            .await
    }
}

#[async_trait]
impl MiniAppFilesPort for SqliteMiniAppManagedStorage {
    async fn resolve(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> MiniAppPlatformResult<MiniAppFilesDirDescriptor> {
        self.registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .and_then(|registration| registration.descriptor.files_dir.clone())
            .filter(|descriptor| &descriptor.handle_id == handle_id)
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)
    }
}

#[async_trait]
impl MiniAppPrivateDatabasePort for SqliteMiniAppManagedStorage {
    async fn query(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseQueryResult> {
        statement.validate_query()?;
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration
            .descriptor
            .private_database
            .as_ref()
            .is_none_or(|database| &database.handle_id != handle_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        ensure_not_canceled(&cancellation)?;
        self.with_database(registration, move |connection, _| {
            ensure_not_canceled(&cancellation)?;
            let values = parameter_values(&statement.parameters)?;
            let mut prepared = connection
                .prepare(&statement.sql)
                .map_err(database_error)?;
            let mut rows = prepared
                .query(params_from_iter(values.iter()))
                .map_err(database_error)?;
            let mut result = Vec::new();
            let mut result_bytes = 0usize;
            while let Some(row) = rows.next().map_err(database_error)? {
                if result.len() >= MAX_DATABASE_RESULT_ROWS {
                    return Err(MiniAppPlatformError::Database(
                        "database query result exceeds the row limit".into(),
                    ));
                }
                let value = row_to_json(row).map(StrictJsonValue)?;
                let encoded = serde_json::to_vec(&value)
                    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
                result_bytes = result_bytes.saturating_add(encoded.len());
                if result_bytes > MAX_DATABASE_RESULT_BYTES {
                    return Err(MiniAppPlatformError::Database(
                        "database query result exceeds the byte limit".into(),
                    ));
                }
                result.push(value);
            }
            ensure_not_canceled(&cancellation)?;
            Ok(MiniAppDatabaseQueryResult { rows: result })
        })
        .await
    }

    async fn execute(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statement: MiniAppDatabaseStatement,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<MiniAppDatabaseExecuteResult> {
        statement.validate_execute()?;
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration
            .descriptor
            .private_database
            .as_ref()
            .is_none_or(|database| &database.handle_id != handle_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        ensure_not_canceled(&cancellation)?;
        self.with_database(registration, move |connection, _| {
            ensure_not_canceled(&cancellation)?;
            let values = parameter_values(&statement.parameters)?;
            let affected = connection
                .execute(&statement.sql, params_from_iter(values.iter()))
                .map_err(database_error)?;
            ensure_not_canceled(&cancellation)?;
            Ok(MiniAppDatabaseExecuteResult {
                affected_rows: affected as u64,
            })
        })
        .await
    }

    async fn batch(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
        statements: Vec<MiniAppDatabaseStatement>,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<Vec<MiniAppDatabaseExecuteResult>> {
        if statements.is_empty() || statements.len() > MAX_DATABASE_BATCH {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(format!(
                "batch requires 1..={MAX_DATABASE_BATCH} DML statements"
            )));
        }
        for statement in &statements {
            statement.validate_execute()?;
        }
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration
            .descriptor
            .private_database
            .as_ref()
            .is_none_or(|database| &database.handle_id != handle_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        ensure_not_canceled(&cancellation)?;
        self.with_database(registration, move |mut connection, internal_mode| {
            ensure_not_canceled(&cancellation)?;
            internal_mode.store(true, Ordering::Release);
            let result = (|| {
                let transaction = connection.transaction().map_err(database_error)?;
                let mut output = Vec::with_capacity(statements.len());
                for statement in &statements {
                    ensure_not_canceled(&cancellation)?;
                    let values = parameter_values(&statement.parameters)?;
                    let affected = transaction
                        .execute(&statement.sql, params_from_iter(values.iter()))
                        .map_err(database_error)?;
                    output.push(MiniAppDatabaseExecuteResult {
                        affected_rows: affected as u64,
                    });
                }
                ensure_not_canceled(&cancellation)?;
                transaction.commit().map_err(database_error)?;
                Ok(output)
            })();
            internal_mode.store(false, Ordering::Release);
            result
        })
        .await
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
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        let descriptor = registration
            .descriptor
            .private_database
            .as_ref()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if &descriptor.handle_id != handle_id {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let owner = registration.owner_user_id;
        MiniAppServiceStoragePort::apply_additive_migrations(
            self,
            &owner,
            miniapp_id,
            &registration.descriptor,
            expected_ledger_digest,
            release,
            migrations,
            applied_at_ms,
        )
        .await
    }

    async fn ledger(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        let registration = self
            .registrations
            .read()
            .await
            .get(miniapp_id.as_ref())
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        let descriptor = registration
            .descriptor
            .private_database
            .as_ref()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if &descriptor.handle_id != handle_id {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let path = registration
            .database_path
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        let _guard = registration.database_lock.lock().await;
        let miniapp_id_owned = miniapp_id.clone();
        let handle_id_owned = handle_id.clone();
        tokio::task::spawn_blocking(move || {
            let (connection, internal_mode) = open_private_database(&path)?;
            with_internal_mode(&internal_mode, || {
                read_ledger(&connection, &miniapp_id_owned, &handle_id_owned)
            })
        })
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?
    }
}

fn ensure_directory(path: &Path) -> MiniAppPlatformResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp managed storage path is not a regular directory: {}",
                path.display()
            )));
        }
        return Ok(());
    }
    fs::create_dir_all(path).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot create MiniApp managed storage directory {}: {error}",
            path.display()
        ))
    })?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot inspect MiniApp managed storage directory {}: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(format!(
            "MiniApp managed storage path is not a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_database_path(path: &Path) -> MiniAppPlatformResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp private database path is not a regular file: {}",
                path.display()
            )));
        }
    }
    if let Some(parent) = path.parent() {
        ensure_directory(parent)?;
    }
    Ok(())
}

fn ensure_within(root: &Path, child: &Path) -> MiniAppPlatformResult<()> {
    if child.starts_with(root) {
        Ok(())
    } else {
        Err(MiniAppPlatformError::InvalidState(
            "MiniApp managed storage path escaped its owner root".into(),
        ))
    }
}

fn validate_path_component(value: &str, field: &str) -> MiniAppPlatformResult<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(MiniAppPlatformError::InvalidState(format!(
            "{field} cannot be used as a managed storage path component"
        )));
    }
    Ok(())
}

fn validate_visible_key(value: &str, maximum: usize) -> MiniAppPlatformResult<()> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(MiniAppPlatformError::InvalidDatabaseRequest(format!(
            "storage key must contain 1 to {maximum} visible ASCII bytes"
        )));
    }
    Ok(())
}

fn positive_revision(value: i64) -> MiniAppPlatformResult<u64> {
    u64::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| MiniAppPlatformError::Database("KV revision is invalid".into()))
}

async fn fetch_kv(
    transaction: &mut nomifun_db::sqlx::Transaction<'_, nomifun_db::sqlx::Sqlite>,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    namespace: &str,
    key: &str,
) -> MiniAppPlatformResult<Option<MiniAppKvRow>> {
    let row = nomifun_db::sqlx::query_as::<_, MiniAppKvRow>(
        "SELECT * FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND namespace = ? AND key = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(namespace)
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    if let Some(row) = &row
        && (row.revision < 1
            || row.key_generation < 1
            || row.key_generation > row.revision
            || (row.is_tombstone && row.value_json != "null"))
    {
        return Err(MiniAppPlatformError::Database(
            "MiniApp KV row violates its tombstone contract".into(),
        ));
    }
    Ok(row)
}

async fn write_live_kv(
    transaction: &mut nomifun_db::sqlx::Transaction<'_, nomifun_db::sqlx::Sqlite>,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    namespace: &str,
    key: &str,
    value_json: &str,
    current: Option<&MiniAppKvRow>,
    updated_at: i64,
) -> MiniAppPlatformResult<i64> {
    let next_revision = current
        .map(|row| row.revision)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(MiniAppPlatformError::KvRevisionOverflow)?;
    if let Some(row) = current {
        let changed = nomifun_db::sqlx::query(
            "UPDATE miniapp_kv
             SET value_json = ?, revision = ?, is_tombstone = 0,
                 updated_at = ?
             WHERE owner_user_id = ? AND miniapp_id = ?
               AND namespace = ? AND key = ? AND revision = ?",
        )
        .bind(value_json)
        .bind(next_revision)
        .bind(updated_at)
        .bind(owner_user_id)
        .bind(miniapp_id.as_ref())
        .bind(namespace)
        .bind(key)
        .bind(row.revision)
        .execute(&mut **transaction)
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        if changed.rows_affected() != 1 {
            return Err(MiniAppPlatformError::StorageConflict);
        }
    } else {
        let changed = nomifun_db::sqlx::query(
            "INSERT INTO miniapp_kv (
                miniapp_id, owner_user_id, namespace, key, value_json,
                revision, key_generation, is_tombstone, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, 1, 1, 0, ?, ?)",
        )
        .bind(miniapp_id.as_ref())
        .bind(owner_user_id)
        .bind(namespace)
        .bind(key)
        .bind(value_json)
        .bind(updated_at)
        .bind(updated_at)
        .execute(&mut **transaction)
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        if changed.rows_affected() != 1 {
            return Err(MiniAppPlatformError::StorageConflict);
        }
    }
    Ok(next_revision)
}

async fn tombstone_kv(
    transaction: &mut nomifun_db::sqlx::Transaction<'_, nomifun_db::sqlx::Sqlite>,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    namespace: &str,
    key: &str,
    current: &MiniAppKvRow,
    updated_at: i64,
) -> MiniAppPlatformResult<i64> {
    let next_revision = current
        .revision
        .checked_add(1)
        .ok_or(MiniAppPlatformError::KvRevisionOverflow)?;
    let next_generation = current
        .key_generation
        .checked_add(1)
        .ok_or(MiniAppPlatformError::KvRevisionOverflow)?;
    let changed = nomifun_db::sqlx::query(
        "UPDATE miniapp_kv
         SET value_json = 'null', revision = ?, key_generation = ?,
             is_tombstone = 1, updated_at = ?
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND namespace = ? AND key = ? AND revision = ?
           AND key_generation = ? AND is_tombstone = 0",
    )
    .bind(next_revision)
    .bind(next_generation)
    .bind(updated_at)
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(namespace)
    .bind(key)
    .bind(current.revision)
    .bind(current.key_generation)
    .execute(&mut **transaction)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    if changed.rows_affected() != 1 {
        return Err(MiniAppPlatformError::StorageConflict);
    }
    Ok(next_revision)
}

#[derive(Clone, Debug)]
enum KvOperation {
    Get,
    Set { value: JsonValue },
    Delete,
    CompareAndSwap {
        expected_revision: Option<i64>,
        value: Option<JsonValue>,
    },
}

fn open_private_database(
    path: &Path,
) -> MiniAppPlatformResult<(Connection, Arc<AtomicBool>)> {
    ensure_database_path(path)?;
    let connection = Connection::open(path).map_err(database_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(database_error)?;
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(database_error)?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(database_error)?;
    connection
        .pragma_update(None, "trusted_schema", "OFF")
        .map_err(database_error)?;
    connection
        .execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {META_TABLE} (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL
             );
             INSERT OR IGNORE INTO {META_TABLE} (key, value)
             VALUES ('{SCHEMA_EPOCH_KEY}', '1');
             CREATE TABLE IF NOT EXISTS {LEDGER_TABLE} (
                ordinal INTEGER PRIMARY KEY NOT NULL,
                migration_id TEXT NOT NULL UNIQUE,
                migration_digest TEXT NOT NULL,
                release_json TEXT NOT NULL,
                applied_at_ms INTEGER NOT NULL
             );"
        ))
        .map_err(database_error)?;
    let internal_mode = Arc::new(AtomicBool::new(false));
    install_authorizer(&connection, Arc::clone(&internal_mode));
    Ok((connection, internal_mode))
}

fn install_authorizer(connection: &Connection, internal_mode: Arc<AtomicBool>) {
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        let internal = internal_mode.load(Ordering::Acquire);
        let main_database = context
            .database_name
            .is_none_or(|database| database == "main");
        if !main_database {
            return Authorization::Deny;
        }
        if internal {
            return match context.action {
                AuthAction::Attach { .. }
                | AuthAction::Detach { .. }
                | AuthAction::Pragma { .. }
                | AuthAction::CreateTempIndex { .. }
                | AuthAction::CreateTempTable { .. }
                | AuthAction::CreateTempTrigger { .. }
                | AuthAction::CreateTempView { .. }
                | AuthAction::CreateTrigger { .. }
                | AuthAction::CreateView { .. }
                | AuthAction::DropIndex { .. }
                | AuthAction::DropTable { .. }
                | AuthAction::DropTempIndex { .. }
                | AuthAction::DropTempTable { .. }
                | AuthAction::DropTempTrigger { .. }
                | AuthAction::DropTempView { .. }
                | AuthAction::DropTrigger { .. }
                | AuthAction::DropView { .. }
                | AuthAction::CreateVtable { .. }
                | AuthAction::DropVtable { .. }
                | AuthAction::Reindex { .. }
                | AuthAction::Analyze { .. }
                | AuthAction::Recursive
                | AuthAction::Unknown { .. } => Authorization::Deny,
                _ => Authorization::Allow,
            };
        }
        match context.action {
            AuthAction::Attach { .. }
            | AuthAction::Detach { .. }
            | AuthAction::Pragma { .. }
            | AuthAction::CreateTempIndex { .. }
            | AuthAction::CreateTempTable { .. }
            | AuthAction::CreateTempTrigger { .. }
            | AuthAction::CreateTempView { .. }
            | AuthAction::CreateTrigger { .. }
            | AuthAction::CreateView { .. }
            | AuthAction::DropIndex { .. }
            | AuthAction::DropTable { .. }
            | AuthAction::DropTempIndex { .. }
            | AuthAction::DropTempTable { .. }
            | AuthAction::DropTempTrigger { .. }
            | AuthAction::DropTempView { .. }
            | AuthAction::DropTrigger { .. }
            | AuthAction::DropView { .. }
            | AuthAction::CreateVtable { .. }
            | AuthAction::DropVtable { .. }
            | AuthAction::Reindex { .. }
            | AuthAction::Analyze { .. }
            | AuthAction::Recursive
            | AuthAction::Unknown { .. } => Authorization::Deny,
            AuthAction::Function { function_name }
                if function_name.eq_ignore_ascii_case("load_extension")
                    || function_name.eq_ignore_ascii_case("fts3_tokenizer") =>
            {
                Authorization::Deny
            }
            AuthAction::Transaction { .. } | AuthAction::Savepoint { .. } => {
                if internal {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::CreateTable { table_name }
            | AuthAction::CreateIndex { index_name: table_name, .. }
            | AuthAction::AlterTable { table_name, .. } => {
                if internal && !table_name.starts_with("sqlite_") {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Read { table_name, .. }
            | AuthAction::Insert { table_name }
            | AuthAction::Update { table_name, .. }
            | AuthAction::Delete { table_name } => {
                if !internal && !is_user_table_name(table_name) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            AuthAction::Select | AuthAction::Function { .. } => Authorization::Allow,
            _ => Authorization::Deny,
        }
    }));
}

fn with_internal_mode<T>(
    mode: &Arc<AtomicBool>,
    operation: impl FnOnce() -> MiniAppPlatformResult<T>,
) -> MiniAppPlatformResult<T> {
    mode.store(true, Ordering::Release);
    let result = operation();
    mode.store(false, Ordering::Release);
    result
}

fn read_ledger(
    connection: &Connection,
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
    let schema_epoch: u64 = connection
        .query_row(
            &format!(
                "SELECT value FROM {META_TABLE} WHERE key = ?1"
            ),
            rusqlite::params![SCHEMA_EPOCH_KEY],
            |row| row.get::<_, String>(0),
        )
        .map_err(database_error)?
        .parse()
        .map_err(|error| {
            MiniAppPlatformError::Database(format!(
                "private database schema epoch is invalid: {error}"
            ))
        })?;
    let mut statement = connection
        .prepare(&format!(
            "SELECT ordinal, migration_id, migration_digest,
                    release_json, applied_at_ms
             FROM {LEDGER_TABLE} ORDER BY ordinal"
        ))
        .map_err(database_error)?;
    let rows = statement
        .query_map([], |row| {
            let release_json: String = row.get(3)?;
            let release = serde_json::from_str(&release_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(MiniAppMigrationLedgerEntry {
                ordinal: row.get::<_, i64>(0)? as u64,
                migration_id: MiniAppMigrationId::from(row.get::<_, String>(1)?),
                migration_digest: DigestHex::from(row.get::<_, String>(2)?),
                release,
                applied_at_ms: row.get(4)?,
            })
        })
        .map_err(database_error)?;
    let entries = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    let ledger_digest = ledger_digest(miniapp_id, handle_id, schema_epoch, &entries)?;
    let ledger = MiniAppMigrationLedger {
        miniapp_id: miniapp_id.clone(),
        handle_id: handle_id.clone(),
        schema_epoch,
        entries,
        ledger_digest,
    };
    ledger.validate()?;
    Ok(ledger)
}

fn ledger_digest(
    miniapp_id: &MiniAppId,
    handle_id: &MiniAppDatabaseHandleId,
    schema_epoch: u64,
    entries: &[MiniAppMigrationLedgerEntry],
) -> MiniAppPlatformResult<DigestHex> {
    #[derive(Serialize)]
    struct DigestInput<'a> {
        miniapp_id: &'a MiniAppId,
        handle_id: &'a MiniAppDatabaseHandleId,
        schema_epoch: u64,
        entries: &'a [MiniAppMigrationLedgerEntry],
    }
    digest_payload(&DigestInput {
        miniapp_id,
        handle_id,
        schema_epoch,
        entries,
    })
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))
}

fn migration_sql(
    action: &MiniAppAdditiveMigrationAction,
) -> MiniAppPlatformResult<String> {
    match action {
        MiniAppAdditiveMigrationAction::CreateTable {
            table_name,
            columns,
            primary_key_columns,
        } => {
            validate_migration_identifier(table_name, "migration.table_name")?;
            let mut definitions = columns
                .iter()
                .map(|column| -> MiniAppPlatformResult<String> {
                    validate_migration_column(column)?;
                    let mut value = format!(
                        "{} {}",
                        quote_identifier(&column.name),
                        column.declared_type
                    );
                    if !column.nullable {
                        value.push_str(" NOT NULL");
                    }
                    if let Some(default_literal) = &column.default_literal {
                        value.push_str(" DEFAULT ");
                        value.push_str(default_literal);
                    }
                    Ok(value)
                })
                .collect::<MiniAppPlatformResult<Vec<_>>>()?;
            if !primary_key_columns.is_empty() {
                definitions.push(format!(
                    "PRIMARY KEY ({})",
                    primary_key_columns
                        .iter()
                        .map(|column| quote_identifier(column))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            Ok(format!(
                "CREATE TABLE {} ({})",
                quote_identifier(table_name),
                definitions.join(", ")
            ))
        }
        MiniAppAdditiveMigrationAction::CreateIndex {
            index_name,
            table_name,
            columns,
            unique,
        } => {
            validate_migration_identifier(index_name, "migration.index_name")?;
            validate_migration_identifier(table_name, "migration.table_name")?;
            if columns.is_empty() {
                return Err(MiniAppPlatformError::InvalidDatabaseRequest(
                    "migration index requires at least one column".into(),
                ));
            }
            for column in columns {
                validate_migration_identifier(column, "migration.index_column")?;
            }
            Ok(format!(
                "CREATE {}INDEX {} ON {} ({})",
                if *unique { "UNIQUE " } else { "" },
                quote_identifier(index_name),
                quote_identifier(table_name),
                columns
                    .iter()
                    .map(|column| quote_identifier(column))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
        MiniAppAdditiveMigrationAction::AddColumn { table_name, column } => {
            validate_migration_identifier(table_name, "migration.table_name")?;
            validate_migration_column(column)?;
            let mut definition = format!(
                "{} {}",
                quote_identifier(&column.name),
                column.declared_type
            );
            if !column.nullable {
                definition.push_str(" NOT NULL");
            }
            if let Some(default_literal) = &column.default_literal {
                definition.push_str(" DEFAULT ");
                definition.push_str(default_literal);
            }
            Ok(format!(
                "ALTER TABLE {} ADD COLUMN {}",
                quote_identifier(table_name),
                definition
            ))
        }
    }
}

fn validate_migration_identifier(
    value: &str,
    field: &str,
) -> MiniAppPlatformResult<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(MiniAppPlatformError::InvalidDatabaseRequest(format!(
            "{field} is not a safe SQL identifier"
        )));
    }
    if value.starts_with("sqlite_") || value.starts_with("__nomifun_") {
        return Err(MiniAppPlatformError::InvalidDatabaseRequest(format!(
            "{field} uses a reserved Host namespace"
        )));
    }
    Ok(())
}

fn validate_migration_column(
    column: &nomifun_agent_contracts::MiniAppMigrationColumn,
) -> MiniAppPlatformResult<()> {
    validate_migration_identifier(&column.name, "migration.column.name")?;
    validate_sql_type(&column.declared_type)?;
    if let Some(default_literal) = &column.default_literal {
        validate_default_literal(default_literal)?;
    }
    Ok(())
}

fn validate_sql_type(value: &str) -> MiniAppPlatformResult<()> {
    let value = value.trim();
    let upper = value.to_ascii_uppercase();
    let (base, suffix) = upper
        .split_once('(')
        .map_or((upper.as_str(), None), |(base, suffix)| {
            (base.trim_end(), Some(suffix))
        });
    let allowed = [
        "INTEGER", "INT", "REAL", "FLOAT", "DOUBLE", "TEXT", "CLOB", "BLOB",
        "NUMERIC", "DECIMAL", "BOOLEAN", "DATE", "DATETIME", "CHAR",
        "VARCHAR", "NVARCHAR",
    ];
    if !allowed.contains(&base)
        && !matches!(upper.as_str(), "DOUBLE PRECISION" | "UNSIGNED BIG INT")
    {
        return Err(MiniAppPlatformError::InvalidDatabaseRequest(
            "migration declared_type must be a SQLite storage type".into(),
        ));
    }
    if let Some(suffix) = suffix {
        if !suffix.ends_with(')')
            || !suffix[..suffix.len() - 1]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b',')
        {
            return Err(MiniAppPlatformError::InvalidDatabaseRequest(
                "migration declared_type dimensions are invalid".into(),
            ));
        }
    }
    Ok(())
}

fn validate_default_literal(value: &str) -> MiniAppPlatformResult<()> {
    let value = value.trim();
    let upper = value.to_ascii_uppercase();
    if matches!(upper.as_str(), "NULL" | "CURRENT_TIMESTAMP" | "CURRENT_DATE" | "CURRENT_TIME")
        || is_numeric_literal(value)
        || is_quoted_sql_string(value)
    {
        return Ok(());
    }
    Err(MiniAppPlatformError::InvalidDatabaseRequest(
        "migration default_literal must be a scalar SQL literal".into(),
    ))
}

fn is_numeric_literal(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E'))
        && value.bytes().any(|byte| byte.is_ascii_digit())
}

fn is_quoted_sql_string(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'\'' || bytes[bytes.len() - 1] != b'\'' {
        return false;
    }
    let mut index = 1;
    while index + 1 < bytes.len() {
        if bytes[index] == b'\'' {
            if bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            return false;
        }
        index += 1;
    }
    true
}

fn is_user_table_name(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with("sqlite_")
        && !value.starts_with("__nomifun_")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn parameter_values(parameters: &StrictJsonValue) -> MiniAppPlatformResult<Vec<SqlValue>> {
    let values = parameters.0.as_array().ok_or_else(|| {
        MiniAppPlatformError::InvalidDatabaseRequest(
            "database parameters must be a JSON array".into(),
        )
    })?;
    values
        .iter()
        .map(|value| match value {
            JsonValue::Null => Ok(SqlValue::Null),
            JsonValue::Bool(value) => Ok(SqlValue::Integer(i64::from(*value))),
            JsonValue::Number(value) => value
                .as_i64()
                .map(SqlValue::Integer)
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()).map(SqlValue::Integer))
                .or_else(|| value.as_f64().map(SqlValue::Real))
                .ok_or_else(|| {
                    MiniAppPlatformError::InvalidDatabaseRequest(
                        "database numeric parameter is out of range".into(),
                    )
                }),
            JsonValue::String(value) => Ok(SqlValue::Text(value.clone())),
            JsonValue::Array(_) | JsonValue::Object(_) => serde_json::to_string(value)
                .map(SqlValue::Text)
                .map_err(|error| MiniAppPlatformError::InvalidDatabaseRequest(error.to_string())),
        })
        .collect()
}

fn row_to_json(row: &rusqlite::Row<'_>) -> MiniAppPlatformResult<JsonValue> {
    let mut object = serde_json::Map::new();
    for index in 0..row.as_ref().column_count() {
        let name = row
            .as_ref()
            .column_name(index)
            .map(str::to_owned)
            .unwrap_or_else(|_| format!("column_{index}"));
        let value = match row.get_ref(index).map_err(database_error)? {
            ValueRef::Null => JsonValue::Null,
            ValueRef::Integer(value) => JsonValue::from(value),
            ValueRef::Real(value) => serde_json::Number::from_f64(value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            ValueRef::Text(value) => JsonValue::String(
                String::from_utf8_lossy(value).into_owned(),
            ),
            ValueRef::Blob(value) => JsonValue::String(hex::encode(value)),
        };
        object.insert(name, value);
    }
    Ok(JsonValue::Object(object))
}

fn database_error(error: impl std::fmt::Display) -> MiniAppPlatformError {
    MiniAppPlatformError::Database(error.to_string())
}

fn ensure_not_canceled(
    cancellation: &MiniAppCallCancellation,
) -> MiniAppPlatformResult<()> {
    if cancellation.is_canceled() {
        Err(MiniAppPlatformError::Canceled)
    } else {
        Ok(())
    }
}
