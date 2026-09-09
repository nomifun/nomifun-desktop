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
    MiniAppServiceStorageDescriptor, StrictJsonValue, digest_bytes, digest_payload,
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
use uuid::Uuid;

use crate::{
    MiniAppBackupFile, MiniAppBackupStorage, MiniAppCallCancellation,
    MiniAppDatabaseExecuteResult, MiniAppDatabaseQueryResult, MiniAppDatabaseStatement,
    MiniAppFilesPort, MiniAppHostKvPort, MiniAppMigrationLedger, MiniAppPlatformError,
    MiniAppPlatformResult, MiniAppPrivateDatabasePort,
    MiniAppServiceStoragePort, MiniAppServiceStorageRequest, MiniAppServiceStorageResolution,
    MiniAppServiceTestKvSnapshotEntry, MiniAppServiceTestStorageResolution,
    rebind_migration_ledger_for_target, service_test_kv_digest, validate_service_test_id,
};
use crate::MiniAppMigrationLedgerEntry;

const FILES_DIRECTORY: &str = "files";
const DATABASES_DIRECTORY: &str = "databases";
const SERVICE_TESTS_DIRECTORY: &str = "service-tests";
const PRODUCTION_KV_NAMESPACE: &str = "service";
const SERVICE_TEST_KV_NAMESPACE_PREFIX: &str = "service-test:";
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
    kv_namespace: String,
    database_path: Option<PathBuf>,
    database_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct StorageKey {
    owner_user_id: String,
    miniapp_id: String,
    test_id: Option<String>,
}

struct SqliteAuthorizationState {
    allow_schema: AtomicBool,
    allow_transactions: AtomicBool,
    allow_dml: AtomicBool,
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
    registrations: Arc<RwLock<BTreeMap<StorageKey, RegisteredStorage>>>,
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
        ensure_directory(&root.join(SERVICE_TESTS_DIRECTORY))?;
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
            .values()
            .find(|registration| {
                registration.owner_user_id == owner_user_id
                    && registration.descriptor.kv.miniapp_id == *miniapp_id
                    && registration.descriptor == *descriptor
            })
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
            .iter()
            .filter(|(key, value)| {
                key.miniapp_id == miniapp_id.as_ref() && value.descriptor == *descriptor
            })
            .map(|(_, value)| value)
            .next()
            .cloned()
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
        if registration.descriptor != *descriptor {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        Ok(registration)
    }

    async fn registration_for_files_handle(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppFilesHandleId,
    ) -> MiniAppPlatformResult<RegisteredStorage> {
        self.registrations
            .read()
            .await
            .iter()
            .find(|(_, registration)| {
                registration.descriptor.kv.miniapp_id == *miniapp_id
                    && registration
                        .descriptor
                        .files_dir
                        .as_ref()
                        .is_some_and(|files| &files.handle_id == handle_id)
            })
            .map(|(_, registration)| registration.clone())
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)
    }

    async fn registration_for_database_handle(
        &self,
        miniapp_id: &MiniAppId,
        handle_id: &MiniAppDatabaseHandleId,
    ) -> MiniAppPlatformResult<RegisteredStorage> {
        self.registrations
            .read()
            .await
            .iter()
            .find(|(_, registration)| {
                registration.descriptor.kv.miniapp_id == *miniapp_id
                    && registration
                        .descriptor
                        .private_database
                        .as_ref()
                        .is_some_and(|database| &database.handle_id == handle_id)
            })
            .map(|(_, registration)| registration.clone())
            .ok_or(MiniAppPlatformError::UnknownStorageHandle)
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
        namespace: &str,
        request: MiniAppBridgeKvRequest,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        ensure_not_canceled(&cancellation)?;
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        if storage.kv.miniapp_id != *miniapp_id {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let (key, operation) = match request {
            MiniAppBridgeKvRequest::Get { key } => {
                (key, KvOperation::Get)
            }
            MiniAppBridgeKvRequest::Set { key, value } => {
                (key, KvOperation::Set { value: value.0 })
            }
            MiniAppBridgeKvRequest::Delete { key } => {
                (key, KvOperation::Delete)
            }
            MiniAppBridgeKvRequest::CompareAndSwap {
                key,
                expected_revision,
                value,
            } => (
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
        validate_visible_key(namespace, 128)?;
        validate_visible_key(&key, 256)?;

        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
        ensure_not_canceled(&cancellation)?;
        let current = fetch_kv(
            &mut transaction,
            owner_user_id,
            miniapp_id,
            namespace,
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
                    namespace,
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
                            namespace,
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
                                namespace,
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
                                    namespace,
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
        ensure_not_canceled(&cancellation)?;
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
        F: FnOnce(Connection, Arc<SqliteAuthorizationState>) -> Result<T, MiniAppPlatformError>
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
        validate_path_component(owner_user_id, "owner_user_id")?;
        validate_path_component(miniapp_id.as_ref(), "miniapp_id")?;
        let files_dir = if uses_files {
            let canonical = ensure_managed_directory(
                self.root(),
                &[FILES_DIRECTORY, owner_user_id, miniapp_id.as_ref()],
            )?;
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
                let database_parent = ensure_managed_directory(
                    self.root(),
                    &[DATABASES_DIRECTORY, owner_user_id],
                )?;
                let path = database_parent.join(format!("{}.sqlite", miniapp_id.as_ref()));
                ensure_database_path(&path)?;
                let ledger = tokio::task::spawn_blocking({
                    let path = path.clone();
                    let miniapp_id = miniapp_id.clone();
                    move || {
                        let (connection, internal_mode) = open_private_database(&path)?;
                        with_authorizer_mode(&internal_mode, true, true, true, || {
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
        let mut registrations = self.registrations.write().await;
        let storage_key = StorageKey {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.as_ref().to_owned(),
            test_id: None,
        };
        if let Some(existing) = registrations.get(&storage_key)
            && existing.owner_user_id != owner_user_id
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        let database_lock = registrations
            .get(&storage_key)
            .map(|existing| Arc::clone(&existing.database_lock))
            .unwrap_or_else(|| Arc::new(Mutex::new(())));
        let registration = RegisteredStorage {
            owner_user_id: owner_user_id.to_owned(),
            descriptor: descriptor.clone(),
            kv_namespace: PRODUCTION_KV_NAMESPACE.to_owned(),
            database_path,
            database_lock,
        };
        registrations.insert(storage_key, registration);
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
            with_authorizer_mode(&internal_mode, true, true, true, || {
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
            .values_mut()
            .find(|registration| {
                registration.owner_user_id == owner_user_id
                    && registration.descriptor.kv.miniapp_id == *miniapp_id
                    && registration.descriptor == *storage
            })
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
                    &registration.kv_namespace,
                    request,
                    cancellation,
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

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppServiceTestStorageResolution> {
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        validate_path_component(owner_user_id, "owner_user_id")?;
        validate_path_component(miniapp_id.as_ref(), "miniapp_id")?;
        validate_service_test_id(test_id)?;
        self.purge_service_test_storage(owner_user_id, miniapp_id, test_id)
            .await?;

        let production = self
            .resolve_service_storage(
                owner_user_id,
                miniapp_id,
                uses_files,
                uses_private_database,
            )
            .await?;
        let production_registration = self
            .registration(owner_user_id, miniapp_id, &production.descriptor)
            .await?;
        let kv_namespace = service_test_kv_namespace(test_id)?;
        let test_root = ensure_managed_directory(
            self.root(),
            &[
                SERVICE_TESTS_DIRECTORY,
                owner_user_id,
                miniapp_id.as_ref(),
                test_id,
            ],
        )?;
        let materialized: MiniAppPlatformResult<_> = async {
            let files_dir = if uses_files {
                let canonical = ensure_managed_directory(&test_root, &["files"])?;
                Some(MiniAppFilesDirDescriptor {
                    handle_id: MiniAppFilesHandleId::from(format!(
                        "miniapp-test-files-{}-{test_id}",
                        miniapp_id.as_ref()
                    )),
                    miniapp_id: miniapp_id.clone(),
                    absolute_path: canonical.display().to_string(),
                })
            } else {
                None
            };
            let copied_kv_digest = copy_service_test_kv(
                &self.pool,
                owner_user_id,
                miniapp_id,
                &kv_namespace,
            )
            .await?;
            let (
                private_database,
                copied_private_database_digest,
                migration_ledger,
                database_path,
            ) = if uses_private_database {
                let source_path = production_registration
                    .database_path
                    .clone()
                    .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
                let target_path = test_root.join("private.sqlite");
                ensure_database_path(&target_path)?;
                let handle_id = MiniAppDatabaseHandleId::from(format!(
                    "miniapp-test-db-{}-{test_id}",
                    miniapp_id.as_ref()
                ));
                let miniapp_id_owned = miniapp_id.clone();
                let handle_id_owned = handle_id.clone();
                let source_lock = Arc::clone(&production_registration.database_lock);
                let (database_digest, ledger) = tokio::task::spawn_blocking(move || {
                    let _guard = source_lock.blocking_lock();
                    create_private_database_snapshot(
                        &source_path,
                        &target_path,
                        &miniapp_id_owned,
                        &handle_id_owned,
                    )
                })
                .await
                .map_err(|error| MiniAppPlatformError::Database(error.to_string()))??;
                let canonical_path = fs::canonicalize(test_root.join("private.sqlite"))
                    .map_err(|error| {
                        MiniAppPlatformError::Runtime(format!(
                            "cannot canonicalize Service Test private database: {error}"
                        ))
                    })?;
                ensure_within(self.root(), &canonical_path)?;
                (
                    Some(MiniAppPrivateDatabaseDescriptor {
                        handle_id,
                        miniapp_id: miniapp_id.clone(),
                        schema_epoch: ledger.schema_epoch,
                        migration_ledger_digest: ledger.ledger_digest.clone(),
                    }),
                    Some(database_digest),
                    Some(ledger),
                    Some(canonical_path),
                )
            } else {
                (None, None, None, None)
            };
            Ok((
                files_dir,
                copied_kv_digest,
                private_database,
                copied_private_database_digest,
                migration_ledger,
                database_path,
            ))
        }
        .await;
        let (
            files_dir,
            copied_kv_digest,
            private_database,
            copied_private_database_digest,
            migration_ledger,
            database_path,
        ) = match materialized {
            Ok(materialized) => materialized,
            Err(error) => {
                let directory_cleanup = remove_managed_directory(self.root(), &test_root);
                let kv_cleanup = delete_service_test_kv(
                    &self.pool,
                    owner_user_id,
                    miniapp_id,
                    &kv_namespace,
                )
                .await;
                if let Err(cleanup_error) = directory_cleanup.and(kv_cleanup) {
                    return Err(MiniAppPlatformError::Runtime(format!(
                        "Service Test storage creation failed: {error}; cleanup failed: {cleanup_error}"
                    )));
                }
                return Err(error);
            }
        };

        let descriptor = MiniAppServiceStorageDescriptor {
            kv: nomifun_agent_contracts::MiniAppKvHandleDescriptor {
                handle_id: nomifun_agent_contracts::MiniAppKvHandleId::from(format!(
                    "miniapp-test-kv-{}-{test_id}",
                    miniapp_id.as_ref()
                )),
                miniapp_id: miniapp_id.clone(),
                namespace_revision: 1,
            },
            files_dir,
            private_database,
        };
        let registration = RegisteredStorage {
            owner_user_id: owner_user_id.to_owned(),
            descriptor: descriptor.clone(),
            kv_namespace,
            database_path,
            database_lock: Arc::new(Mutex::new(())),
        };
        self.registrations.write().await.insert(
            StorageKey {
                owner_user_id: owner_user_id.to_owned(),
                miniapp_id: miniapp_id.as_ref().to_owned(),
                test_id: Some(test_id.to_owned()),
            },
            registration,
        );
        Ok(MiniAppServiceTestStorageResolution {
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
    ) -> MiniAppPlatformResult<()> {
        validate_path_component(owner_user_id, "owner_user_id")?;
        validate_path_component(miniapp_id.as_ref(), "miniapp_id")?;
        validate_service_test_id(test_id)?;
        let storage_key = StorageKey {
            owner_user_id: owner_user_id.to_owned(),
            miniapp_id: miniapp_id.as_ref().to_owned(),
            test_id: Some(test_id.to_owned()),
        };
        let registration = self
            .registrations
            .read()
            .await
            .get(&storage_key)
            .cloned();
        let _database_guard = match registration {
            Some(registration) => Some(registration.database_lock.lock_owned().await),
            None => None,
        };
        let test_root = self
            .root
            .join(SERVICE_TESTS_DIRECTORY)
            .join(owner_user_id)
            .join(miniapp_id.as_ref())
            .join(test_id);
        remove_managed_directory(self.root(), &test_root)?;
        delete_service_test_kv(
            &self.pool,
            owner_user_id,
            miniapp_id,
            &service_test_kv_namespace(test_id)?,
        )
        .await?;
        self.registrations.write().await.remove(&storage_key);
        Ok(())
    }

    async fn purge_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> MiniAppPlatformResult<()> {
        validate_path_component(owner_user_id, "owner_user_id")?;
        validate_path_component(miniapp_id.as_ref(), "miniapp_id")?;
        let registrations = {
            let registrations = self.registrations.read().await;
            registrations
                .iter()
                .filter(|(key, _)| {
                    key.owner_user_id == owner_user_id
                        && key.miniapp_id == miniapp_id.as_ref()
                })
                .map(|(_, registration)| registration.clone())
                .collect::<Vec<_>>()
        };
        let mut _database_guards = Vec::with_capacity(registrations.len());
        for registration in registrations {
            _database_guards.push(registration.database_lock.lock_owned().await);
        }
        let database_path = self
            .root
            .join(DATABASES_DIRECTORY)
            .join(owner_user_id)
            .join(format!("{}.sqlite", miniapp_id.as_ref()));
        let files_path = self
            .root
            .join(FILES_DIRECTORY)
            .join(owner_user_id)
            .join(miniapp_id.as_ref());
        let tests_path = self
            .root
            .join(SERVICE_TESTS_DIRECTORY)
            .join(owner_user_id)
            .join(miniapp_id.as_ref());
        remove_private_database_files(self.root(), &database_path)?;
        remove_managed_directory(self.root(), &files_path)?;
        remove_managed_directory(self.root(), &tests_path)?;
        delete_all_service_kv(&self.pool, owner_user_id, miniapp_id).await?;
        self.registrations.write().await.retain(|key, _| {
            key.owner_user_id != owner_user_id || key.miniapp_id != miniapp_id.as_ref()
        });
        Ok(())
    }

    async fn export_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppBackupStorage> {
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        let resolution = self
            .resolve_service_storage(
                owner_user_id,
                miniapp_id,
                uses_files,
                uses_private_database,
            )
            .await?;
        let registration = self
            .registration(owner_user_id, miniapp_id, &resolution.descriptor)
            .await?;
        let _guard = registration.database_lock.lock().await;
        let files = match resolution.descriptor.files_dir.as_ref() {
            Some(files) => read_backup_files(
                Path::new(&files.absolute_path),
                self.root(),
            )?,
            None => Vec::new(),
        };
        let (private_database, migration_ledger) =
            if let Some(database) = resolution.descriptor.private_database.as_ref() {
                let source_path = registration
                    .database_path
                    .clone()
                    .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
                let temporary_root = ensure_managed_directory(
                    self.root(),
                    &[
                        SERVICE_TESTS_DIRECTORY,
                        owner_user_id,
                        miniapp_id.as_ref(),
                    ],
                )?;
                let temporary_path =
                    temporary_root.join(format!(".backup-{}.sqlite", Uuid::now_v7()));
                let miniapp_id_owned = miniapp_id.clone();
                let handle_id = database.handle_id.clone();
                let temporary_path_for_task = temporary_path.clone();
                let ledger = tokio::task::spawn_blocking(move || {
                    let (_, ledger) = create_private_database_snapshot(
                        &source_path,
                        &temporary_path_for_task,
                        &miniapp_id_owned,
                        &handle_id,
                    )?;
                    Ok::<_, MiniAppPlatformError>(ledger)
                })
                .await
                .map_err(|error| MiniAppPlatformError::Database(error.to_string()))??;
                let bytes = fs::read(&temporary_path).map_err(|error| {
                    MiniAppPlatformError::Runtime(format!(
                        "cannot read MiniApp backup database snapshot: {error}"
                    ))
                })?;
                remove_private_database_files(self.root(), &temporary_path)?;
                (Some(bytes), Some(ledger))
            } else {
                (None, None)
            };
        Ok(MiniAppBackupStorage {
            kv: Vec::new(),
            files,
            private_database,
            migration_ledger,
        })
    }

    async fn import_backup_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: MiniAppBackupStorage,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<()> {
        self.ensure_product_owner(owner_user_id, miniapp_id).await?;
        if !storage.kv.is_empty() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp backup KV must be restored by the DB repository".into(),
            ));
        }
        if uses_files {
            let resolution = self
                .resolve_service_storage(owner_user_id, miniapp_id, true, uses_private_database)
                .await?;
            let files_dir = resolution
                .descriptor
                .files_dir
                .as_ref()
                .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
            let root = PathBuf::from(&files_dir.absolute_path);
            restore_backup_files_atomically(&root, &storage.files, self.root())?;
        } else if !storage.files.is_empty() {
            return Err(MiniAppPlatformError::InvalidState(
                "backup contains Files for a Service without Files capability".into(),
            ));
        }

        if uses_private_database {
            let resolution = self
                .resolve_service_storage(owner_user_id, miniapp_id, uses_files, true)
                .await?;
            let database = resolution
                .descriptor
                .private_database
                .as_ref()
                .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
            let bytes = storage
                .private_database
                .as_deref()
                .ok_or_else(|| {
                    MiniAppPlatformError::InvalidState(
                        "backup is missing the required Private Database".into(),
                    )
                })?;
            let ledger = storage.migration_ledger.as_ref().ok_or_else(|| {
                MiniAppPlatformError::InvalidState(
                    "backup is missing the required migration ledger".into(),
                )
            })?;
            ledger.validate()?;
            let registration = self
                .registration(owner_user_id, miniapp_id, &resolution.descriptor)
                .await?;
            let target_path = registration
                .database_path
                .clone()
                .ok_or(MiniAppPlatformError::UnknownStorageHandle)?;
            let parent = target_path.parent().ok_or_else(|| {
                MiniAppPlatformError::InvalidState(
                    "MiniApp backup database target has no parent".into(),
                )
            })?;
            let temporary_path = parent.join(format!(".restore-{}.sqlite", Uuid::now_v7()));
            write_new_regular_file(&temporary_path, bytes)?;
            let lock = Arc::clone(&registration.database_lock);
            let miniapp_id_owned = miniapp_id.clone();
            let handle_id = database.handle_id.clone();
            let target_path_owned = target_path.clone();
            let storage_root = self.root().to_path_buf();
            let expected_ledger = rebind_migration_ledger_for_target(
                ledger,
                miniapp_id.clone(),
                handle_id.clone(),
            )?;
            let result = tokio::task::spawn_blocking(move || {
                let _guard = lock.blocking_lock();
                let (mut connection, internal_mode) =
                    open_private_database(&temporary_path)?;
                with_authorizer_mode(&internal_mode, true, true, true, || {
                    let transaction = connection.transaction().map_err(database_error)?;
                    for entry in &expected_ledger.entries {
                        let release_json =
                            serde_json::to_string(&entry.release).map_err(database_error)?;
                        let changed = transaction
                            .execute(
                                &format!(
                                    "UPDATE {LEDGER_TABLE}
                                     SET release_json = ?1
                                     WHERE ordinal = ?2"
                                ),
                                rusqlite::params![
                                    release_json,
                                    i64::try_from(entry.ordinal).map_err(|_| {
                                        MiniAppPlatformError::Database(
                                            "migration ordinal overflow".into(),
                                        )
                                    })?,
                                ],
                            )
                            .map_err(database_error)?;
                        if changed != 1 {
                            return Err(MiniAppPlatformError::StorageConflict);
                        }
                    }
                    transaction.commit().map_err(database_error)?;
                    let observed = read_ledger(&connection, &miniapp_id_owned, &handle_id)?;
                    if observed != expected_ledger {
                        return Err(MiniAppPlatformError::StorageConflict);
                    }
                    Ok(())
                })?;
                drop(connection);
                replace_private_database_atomically(
                    &storage_root,
                    &temporary_path,
                    &target_path_owned,
                )
            })
            .await
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
            result?;
        } else if storage.private_database.is_some() || storage.migration_ledger.is_some() {
            return Err(MiniAppPlatformError::InvalidState(
                "backup contains a Private Database for a Service without that capability".into(),
            ));
        }
        Ok(())
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
            .await?;
        self.execute_kv(
            &owner.owner_user_id,
            miniapp_id,
            storage,
            &owner.kv_namespace,
            request.clone(),
            MiniAppCallCancellation::default(),
        )
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
        self.registration_for_files_handle(miniapp_id, handle_id)
            .await?
            .descriptor
            .files_dir
            .clone()
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
            .registration_for_database_handle(miniapp_id, handle_id)
            .await?;
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
            install_cancellation_handler(&connection, cancellation.clone());
            let values = parameter_values(&statement.parameters)?;
            let mut prepared = connection
                .prepare(&statement.sql)
                .map_err(|error| canceled_database_error(error, &cancellation))?;
            let mut rows = prepared
                .query(params_from_iter(values.iter()))
                .map_err(|error| canceled_database_error(error, &cancellation))?;
            let mut result = Vec::new();
            let mut result_bytes = 0usize;
            while let Some(row) = rows
                .next()
                .map_err(|error| canceled_database_error(error, &cancellation))?
            {
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
            .registration_for_database_handle(miniapp_id, handle_id)
            .await?;
        if registration
            .descriptor
            .private_database
            .as_ref()
            .is_none_or(|database| &database.handle_id != handle_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        ensure_not_canceled(&cancellation)?;
        self.with_database(registration, move |connection, authorization| {
            ensure_not_canceled(&cancellation)?;
            install_cancellation_handler(&connection, cancellation.clone());
            with_authorizer_mode(&authorization, false, false, true, || {
                let values = parameter_values(&statement.parameters)?;
                let affected = connection
                    .execute(&statement.sql, params_from_iter(values.iter()))
                    .map_err(|error| canceled_database_error(error, &cancellation))?;
                ensure_not_canceled(&cancellation)?;
                Ok(MiniAppDatabaseExecuteResult {
                    affected_rows: affected as u64,
                })
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
            .registration_for_database_handle(miniapp_id, handle_id)
            .await?;
        if registration
            .descriptor
            .private_database
            .as_ref()
            .is_none_or(|database| &database.handle_id != handle_id)
        {
            return Err(MiniAppPlatformError::UnknownStorageHandle);
        }
        ensure_not_canceled(&cancellation)?;
        self.with_database(registration, move |mut connection, authorization| {
            ensure_not_canceled(&cancellation)?;
            install_cancellation_handler(&connection, cancellation.clone());
            let result = with_authorizer_mode(&authorization, false, true, true, || {
                let transaction = connection
                    .transaction()
                    .map_err(|error| canceled_database_error(error, &cancellation))?;
                let mut output = Vec::with_capacity(statements.len());
                for statement in &statements {
                    ensure_not_canceled(&cancellation)?;
                    let values = parameter_values(&statement.parameters)?;
                    let affected = transaction
                        .execute(&statement.sql, params_from_iter(values.iter()))
                        .map_err(|error| canceled_database_error(error, &cancellation))?;
                    output.push(MiniAppDatabaseExecuteResult {
                        affected_rows: affected as u64,
                    });
                }
                ensure_not_canceled(&cancellation)?;
                transaction
                    .commit()
                    .map_err(|error| canceled_database_error(error, &cancellation))?;
                Ok(output)
            });
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
            .registration_for_database_handle(miniapp_id, handle_id)
            .await?;
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
            .registration_for_database_handle(miniapp_id, handle_id)
            .await?;
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
            with_authorizer_mode(&internal_mode, true, true, true, || {
                read_ledger(&connection, &miniapp_id_owned, &handle_id_owned)
            })
        })
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?
    }
}

fn service_test_kv_namespace(test_id: &str) -> MiniAppPlatformResult<String> {
    validate_service_test_id(test_id)?;
    let namespace = format!("{SERVICE_TEST_KV_NAMESPACE_PREFIX}{test_id}");
    validate_visible_key(&namespace, 128)?;
    Ok(namespace)
}

async fn copy_service_test_kv(
    pool: &SqlitePool,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    test_namespace: &str,
) -> MiniAppPlatformResult<DigestHex> {
    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    nomifun_db::sqlx::query(
        "DELETE FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ? AND namespace = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(test_namespace)
    .execute(&mut *transaction)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    let rows = nomifun_db::sqlx::query_as::<_, MiniAppKvRow>(
        "SELECT * FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ? AND namespace = ?
         ORDER BY key",
    )
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(PRODUCTION_KV_NAMESPACE)
    .fetch_all(&mut *transaction)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    let mut digest_entries = Vec::with_capacity(rows.len());
    for row in rows {
        if row.revision < 1
            || row.key_generation < 1
            || row.key_generation > row.revision
            || (row.is_tombstone && row.value_json != "null")
        {
            return Err(MiniAppPlatformError::Database(
                "MiniApp KV row violates its tombstone contract".into(),
            ));
        }
        let value = serde_json::from_str(&row.value_json).map_err(|error| {
            MiniAppPlatformError::Database(format!(
                "MiniApp KV value is invalid JSON: {error}"
            ))
        })?;
        digest_entries.push(MiniAppServiceTestKvSnapshotEntry {
            key: row.key.clone(),
            value: StrictJsonValue(value),
            revision: u64::try_from(row.revision)
                .map_err(|_| MiniAppPlatformError::KvRevisionOverflow)?,
            key_generation: u64::try_from(row.key_generation)
                .map_err(|_| MiniAppPlatformError::KvRevisionOverflow)?,
            is_tombstone: row.is_tombstone,
        });
        nomifun_db::sqlx::query(
            "INSERT INTO miniapp_kv (
                miniapp_id, owner_user_id, namespace, key, value_json,
                revision, key_generation, is_tombstone, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(miniapp_id.as_ref())
        .bind(owner_user_id)
        .bind(test_namespace)
        .bind(&row.key)
        .bind(&row.value_json)
        .bind(row.revision)
        .bind(row.key_generation)
        .bind(row.is_tombstone)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    }
    let digest = service_test_kv_digest(digest_entries)?;
    transaction
        .commit()
        .await
        .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    Ok(digest)
}

async fn delete_service_test_kv(
    pool: &SqlitePool,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
    test_namespace: &str,
) -> MiniAppPlatformResult<()> {
    nomifun_db::sqlx::query(
        "DELETE FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ? AND namespace = ?",
    )
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(test_namespace)
    .execute(pool)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    Ok(())
}

async fn delete_all_service_kv(
    pool: &SqlitePool,
    owner_user_id: &str,
    miniapp_id: &MiniAppId,
) -> MiniAppPlatformResult<()> {
    nomifun_db::sqlx::query(
        "DELETE FROM miniapp_kv
         WHERE owner_user_id = ? AND miniapp_id = ?
           AND (namespace = ? OR namespace LIKE ?)",
    )
    .bind(owner_user_id)
    .bind(miniapp_id.as_ref())
    .bind(PRODUCTION_KV_NAMESPACE)
    .bind(format!("{SERVICE_TEST_KV_NAMESPACE_PREFIX}%"))
    .execute(pool)
    .await
    .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    Ok(())
}

fn create_private_database_snapshot(
    source_path: &Path,
    target_path: &Path,
    miniapp_id: &MiniAppId,
    test_handle_id: &MiniAppDatabaseHandleId,
) -> MiniAppPlatformResult<(DigestHex, MiniAppMigrationLedger)> {
    ensure_database_path(source_path)?;
    if target_path.exists() {
        return Err(MiniAppPlatformError::StorageConflict);
    }
    let source = Connection::open(source_path).map_err(database_error)?;
    let target = target_path.to_string_lossy().into_owned();
    source
        .execute("VACUUM main INTO ?1", rusqlite::params![target])
        .map_err(database_error)?;
    drop(source);
    ensure_database_path(target_path)?;
    let copied_digest = fs::read(target_path)
        .map(|bytes| digest_bytes(&bytes))
        .map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot hash Service Test private database snapshot: {error}"
            ))
        })?;
    let snapshot = Connection::open(target_path).map_err(database_error)?;
    let ledger = read_ledger(&snapshot, miniapp_id, test_handle_id)?;
    Ok((copied_digest, ledger))
}

fn ensure_directory(path: &Path) -> MiniAppPlatformResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
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
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(format!(
            "MiniApp managed storage path is not a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_database_path(path: &Path) -> MiniAppPlatformResult<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if is_reparse_or_symlink(&metadata) || !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp private database path is not a regular file: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn write_new_regular_file(path: &Path, bytes: &[u8]) -> MiniAppPlatformResult<()> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot create MiniApp storage staging file {}: {error}",
                path.display()
            ))
        })?;
    use std::io::Write;
    file.write_all(bytes).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot write MiniApp storage staging file {}: {error}",
            path.display()
        ))
    })?;
    file.sync_all().map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot sync MiniApp storage staging file {}: {error}",
            path.display()
        ))
    })
}

fn read_backup_files(
    root: &Path,
    managed_root: &Path,
) -> MiniAppPlatformResult<Vec<MiniAppBackupFile>> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp backup Files root {}: {error}",
            root.display()
        ))
    })?;
    ensure_within(managed_root, &canonical_root)?;
    let mut files = Vec::new();
    collect_backup_files(&canonical_root, &canonical_root, &mut files)?;
    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

fn collect_backup_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<MiniAppBackupFile>,
) -> MiniAppPlatformResult<()> {
    for entry in fs::read_dir(current).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot read MiniApp backup Files directory {}: {error}",
            current.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp backup Files entry: {error}"
            ))
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp backup Files entry {}: {error}",
                path.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp backup Files cannot contain symlinks or reparse points".into(),
            ));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            MiniAppPlatformError::InvalidState(
                "MiniApp backup Files entry escaped its root".into(),
            )
        })?;
        let relative = relative
            .components()
            .map(|component| match component {
                std::path::Component::Normal(value) => value.to_str().ok_or_else(|| {
                    MiniAppPlatformError::InvalidState(
                        "MiniApp backup Files path must be UTF-8".into(),
                    )
                }),
                _ => Err(MiniAppPlatformError::InvalidState(
                    "MiniApp backup Files path contains a non-normal component".into(),
                )),
            })
            .collect::<MiniAppPlatformResult<Vec<_>>>()?
            .join("/");
        if metadata.is_dir() {
            collect_backup_files(root, &path, output)?;
        } else if metadata.is_file() {
            output.push(MiniAppBackupFile::new(
                relative,
                fs::read(&path).map_err(|error| {
                    MiniAppPlatformError::Runtime(format!(
                        "cannot read MiniApp backup File {}: {error}",
                        path.display()
                    ))
                })?,
            ));
        } else {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp backup Files cannot contain special entries".into(),
            ));
        }
    }
    Ok(())
}

fn write_backup_files(
    root: &Path,
    files: &[MiniAppBackupFile],
    managed_root: &Path,
) -> MiniAppPlatformResult<()> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp restore Files root {}: {error}",
            root.display()
        ))
    })?;
    ensure_within(managed_root, &canonical_root)?;
    let mut seen = BTreeSet::new();
    for file in files {
        if file.relative_path.is_empty()
            || file.relative_path.contains('\\')
            || file.relative_path.contains(':')
            || file.relative_path.split('/').any(|part| {
                part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
            })
            || !seen.insert(file.relative_path.to_ascii_lowercase())
        {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp backup Files contain an invalid or duplicate path".into(),
            ));
        }
        let target = file
            .relative_path
            .split('/')
            .fold(canonical_root.clone(), |path, component| path.join(component));
        if !target.starts_with(&canonical_root) {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp backup File escaped its target root".into(),
            ));
        }
        let parent = target.parent().ok_or_else(|| {
            MiniAppPlatformError::InvalidState(
                "MiniApp backup File has no parent directory".into(),
            )
        })?;
        ensure_directory(parent)?;
        write_new_regular_file(&target, &file.bytes)?;
    }
    Ok(())
}

fn restore_backup_files_atomically(
    root: &Path,
    files: &[MiniAppBackupFile],
    managed_root: &Path,
) -> MiniAppPlatformResult<()> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp restore Files root {}: {error}",
            root.display()
        ))
    })?;
    ensure_within(managed_root, &canonical_root)?;
    let parent = canonical_root.parent().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp restore Files root has no parent directory".into(),
        )
    })?;
    ensure_directory(parent)?;
    let staging = parent.join(format!(".restore-files-{}", Uuid::now_v7()));
    fs::create_dir(&staging).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot create MiniApp restore Files staging directory {}: {error}",
            staging.display()
        ))
    })?;

    let result = (|| {
        write_backup_files(&staging, files, managed_root)?;
        let mut expected = files.to_vec();
        expected.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        let mut observed = read_backup_files(&staging, managed_root)?;
        observed.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        if observed != expected {
            return Err(MiniAppPlatformError::StorageConflict);
        }
        sync_storage_tree(&staging)?;

        let quarantine = parent.join(format!(".restore-files-old-{}", Uuid::now_v7()));
        fs::rename(&canonical_root, &quarantine).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot quarantine existing MiniApp Files directory: {error}"
            ))
        })?;
        if let Err(error) = fs::rename(&staging, &canonical_root) {
            let rollback = fs::rename(&quarantine, &canonical_root);
            if rollback.is_err() {
                return Err(MiniAppPlatformError::Runtime(format!(
                    "cannot install MiniApp Files restore ({error}); rollback also failed"
                )));
            }
            return Err(MiniAppPlatformError::Runtime(format!(
                "cannot install MiniApp Files restore: {error}"
            )));
        }
        let _ = sync_directory_if_supported(parent);
        if validate_removal_tree(&quarantine).is_ok() {
            let _ = fs::remove_dir_all(&quarantine);
        }
        Ok(())
    })();

    if result.is_err() {
        if fs::symlink_metadata(&staging).is_ok() {
            let _ = validate_removal_tree(&staging).and_then(|()| {
                fs::remove_dir_all(&staging).map_err(|error| {
                    MiniAppPlatformError::Runtime(format!(
                        "cannot clean MiniApp Files restore staging: {error}"
                    ))
                })
            });
        }
    }
    result
}

fn remove_private_database_files(root: &Path, path: &Path) -> MiniAppPlatformResult<()> {
    let parent = path.parent().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp private database cleanup target has no parent".into(),
        )
    })?;
    let Some(canonical_parent) = canonical_existing_directory_chain(root, parent)? else {
        return Ok(());
    };
    let file_name = path.file_name().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp private database cleanup target has no file name".into(),
        )
    })?;
    let base_path = canonical_parent.join(file_name);
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let target = if suffix.is_empty() {
            base_path.clone()
        } else {
            let mut value = base_path.as_os_str().to_os_string();
            value.push(suffix);
            PathBuf::from(value)
        };
        match fs::symlink_metadata(&target) {
            Ok(metadata) if is_reparse_or_symlink(&metadata) => {
                return Err(MiniAppPlatformError::InvalidState(format!(
                    "MiniApp private database cleanup encountered a symlink or reparse point: {}",
                    target.display()
                )));
            }
            Ok(metadata) if metadata.is_file() => {
                fs::remove_file(&target).map_err(|error| {
                    MiniAppPlatformError::Runtime(format!(
                        "cannot remove MiniApp private database file {}: {error}",
                        target.display()
                    ))
                })?
            }
            Ok(_) => {
                return Err(MiniAppPlatformError::InvalidState(format!(
                    "MiniApp private database cleanup target is not a regular file: {}",
                    target.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(MiniAppPlatformError::Runtime(format!(
                    "cannot inspect MiniApp private database file {}: {error}",
                    target.display()
                )));
            }
        }
    }
    Ok(())
}

fn replace_private_database_atomically(
    managed_root: &Path,
    temporary_path: &Path,
    target_path: &Path,
) -> MiniAppPlatformResult<()> {
    let parent = target_path.parent().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp private database target has no parent".into(),
        )
    })?;
    let canonical_parent = canonical_existing_directory_chain(managed_root, parent)?
        .ok_or_else(|| {
            MiniAppPlatformError::InvalidState(
                "MiniApp private database target parent is missing".into(),
            )
        })?;
    ensure_database_path(temporary_path)?;
    let target_name = target_path.file_name().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp private database target has no file name".into(),
        )
    })?;
    let target = canonical_parent.join(target_name);
    let quarantine_base =
        canonical_parent.join(format!(".restore-database-old-{}", Uuid::now_v7()));
    let mut moved = Vec::new();

    for suffix in ["", "-wal", "-shm", "-journal"] {
        let original = database_companion_path(&target, suffix);
        let metadata = match fs::symlink_metadata(&original) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(MiniAppPlatformError::Runtime(format!(
                    "cannot inspect MiniApp private database file {}: {error}",
                    original.display()
                )));
            }
        };
        if is_reparse_or_symlink(&metadata) || !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp private database replacement target is not a regular file: {}",
                original.display()
            )));
        }
        let quarantine = database_companion_path(&quarantine_base, suffix);
        if let Err(error) = fs::rename(&original, &quarantine) {
            for (rollback_original, rollback_quarantine) in moved.iter().rev() {
                let _ = fs::rename(rollback_quarantine, rollback_original);
            }
            return Err(MiniAppPlatformError::Runtime(format!(
                "cannot quarantine MiniApp private database file {}: {error}",
                original.display()
            )));
        }
        moved.push((original, quarantine));
    }

    if let Err(error) = fs::rename(temporary_path, &target) {
        let mut rollback_failed = false;
        for (original, quarantine) in moved.iter().rev() {
            if fs::rename(quarantine, original).is_err() {
                rollback_failed = true;
            }
        }
        let _ = remove_private_database_files(managed_root, temporary_path);
        if rollback_failed {
            return Err(MiniAppPlatformError::Runtime(format!(
                "cannot install MiniApp private database restore ({error}); rollback also failed"
            )));
        }
        return Err(MiniAppPlatformError::Runtime(format!(
            "cannot install MiniApp private database restore: {error}"
        )));
    }

    let _ = sync_directory_if_supported(&canonical_parent);
    for (_, quarantine) in moved {
        let _ = fs::remove_file(quarantine);
    }
    Ok(())
}

fn database_companion_path(base: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        base.to_path_buf()
    } else {
        let mut value = base.as_os_str().to_os_string();
        value.push(suffix);
        PathBuf::from(value)
    }
}

fn sync_storage_tree(root: &Path) -> MiniAppPlatformResult<()> {
    let mut directories = Vec::new();
    collect_storage_directories(root, &mut directories)?;
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        sync_directory_if_supported(&directory)?;
    }
    Ok(())
}

fn collect_storage_directories(
    root: &Path,
    directories: &mut Vec<PathBuf>,
) -> MiniAppPlatformResult<()> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot inspect MiniApp storage staging directory {}: {error}",
            root.display()
        ))
    })?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(format!(
            "MiniApp storage staging path is not a regular directory: {}",
            root.display()
        )));
    }
    directories.push(root.to_path_buf());
    for entry in fs::read_dir(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot read MiniApp storage staging directory {}: {error}",
            root.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp storage staging entry: {error}"
            ))
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp storage staging entry {}: {error}",
                path.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp storage staging cannot contain symlinks or reparse points".into(),
            ));
        }
        if metadata.is_dir() {
            collect_storage_directories(&path, directories)?;
        } else if !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp storage staging cannot contain special files".into(),
            ));
        }
    }
    Ok(())
}

fn sync_directory_if_supported(path: &Path) -> MiniAppPlatformResult<()> {
    #[cfg(unix)]
    {
        match fs::File::open(path).and_then(|file| file.sync_all()) {
            Ok(()) => Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(MiniAppPlatformError::Runtime(format!(
                "cannot sync MiniApp storage directory {}: {error}",
                path.display()
            ))),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn remove_managed_directory(root: &Path, path: &Path) -> MiniAppPlatformResult<()> {
    let parent = path.parent().ok_or_else(|| {
        MiniAppPlatformError::InvalidState(
            "MiniApp filesDir cleanup target has no parent".into(),
        )
    })?;
    let Some(canonical_parent) = canonical_existing_directory_chain(root, parent)? else {
        return Ok(());
    };
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp filesDir during cleanup: {error}"
            )));
        }
    };
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(
            "MiniApp filesDir is not a regular managed directory".into(),
        ));
    }
    let canonical = fs::canonicalize(path).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp filesDir during cleanup: {error}"
        ))
    })?;
    if canonical.parent() != Some(canonical_parent.as_path()) {
        return Err(MiniAppPlatformError::InvalidState(
            "MiniApp filesDir escaped its owner boundary".into(),
        ));
    }
    validate_removal_tree(&canonical)?;
    fs::remove_dir_all(&canonical).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot remove MiniApp filesDir {}: {error}",
            canonical.display()
        ))
    })
}

fn canonical_existing_directory_chain(
    root: &Path,
    target: &Path,
) -> MiniAppPlatformResult<Option<PathBuf>> {
    let relative = target.strip_prefix(root).map_err(|_| {
        MiniAppPlatformError::InvalidState(
            "MiniApp managed storage path escaped its root".into(),
        )
    })?;
    let root_metadata = fs::symlink_metadata(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot inspect MiniApp managed storage root: {error}"
        ))
    })?;
    if is_reparse_or_symlink(&root_metadata) || !root_metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(
            "MiniApp managed storage root is not a regular directory".into(),
        ));
    }
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp managed storage root: {error}"
        ))
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(value) = component else {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp managed storage path contains a non-normal component".into(),
            ));
        };
        current.push(value);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(MiniAppPlatformError::Runtime(format!(
                    "cannot inspect MiniApp managed storage directory {}: {error}",
                    current.display()
                )));
            }
        };
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp managed storage directory is not regular: {}",
                current.display()
            )));
        }
    }
    let canonical_target = fs::canonicalize(target).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot canonicalize MiniApp managed storage directory {}: {error}",
            target.display()
        ))
    })?;
    ensure_within(&canonical_root, &canonical_target)?;
    Ok(Some(canonical_target))
}

fn validate_removal_tree(root: &Path) -> MiniAppPlatformResult<()> {
    let metadata = fs::symlink_metadata(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot inspect MiniApp removal target {}: {error}",
            root.display()
        ))
    })?;
    if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
        return Err(MiniAppPlatformError::InvalidState(format!(
            "MiniApp removal target is not a regular directory: {}",
            root.display()
        )));
    }
    for entry in fs::read_dir(root).map_err(|error| {
        MiniAppPlatformError::Runtime(format!(
            "cannot read MiniApp removal target {}: {error}",
            root.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp removal entry under {}: {error}",
                root.display()
            ))
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect MiniApp removal entry {}: {error}",
                path.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp removal target contains a symlink or reparse point: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            validate_removal_tree(&path)?;
        } else if !metadata.is_file() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "MiniApp removal target contains a special file: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn ensure_managed_directory(
    root: &Path,
    components: &[&str],
) -> MiniAppPlatformResult<PathBuf> {
    let mut current = root.to_path_buf();
    for component in components {
        validate_path_component(component, "managed storage component")?;
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
                return Err(MiniAppPlatformError::InvalidState(format!(
                    "managed storage component is not a regular directory: {}",
                    current.display()
                )));
            }
        } else {
            fs::create_dir(&current).map_err(|error| {
                MiniAppPlatformError::Runtime(format!(
                    "cannot create managed storage directory {}: {error}",
                    current.display()
                ))
            })?;
        }
        let canonical = fs::canonicalize(&current).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot canonicalize managed storage directory {}: {error}",
                current.display()
            ))
        })?;
        ensure_within(root, &canonical)?;
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            MiniAppPlatformError::Runtime(format!(
                "cannot inspect managed storage directory {}: {error}",
                current.display()
            ))
        })?;
        if is_reparse_or_symlink(&metadata) || !metadata.is_dir() {
            return Err(MiniAppPlatformError::InvalidState(format!(
                "managed storage component changed during creation: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}

#[cfg(windows)]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_or_symlink(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
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
) -> MiniAppPlatformResult<(Connection, Arc<SqliteAuthorizationState>)> {
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
    let authorization = Arc::new(SqliteAuthorizationState {
        allow_schema: AtomicBool::new(false),
        allow_transactions: AtomicBool::new(false),
        allow_dml: AtomicBool::new(false),
    });
    install_authorizer(&connection, Arc::clone(&authorization));
    Ok((connection, authorization))
}

fn install_authorizer(
    connection: &Connection,
    authorization: Arc<SqliteAuthorizationState>,
) {
    connection.authorizer(Some(move |context: AuthContext<'_>| {
        let allow_schema = authorization.allow_schema.load(Ordering::Acquire);
        let allow_transactions = authorization
            .allow_transactions
            .load(Ordering::Acquire);
        let allow_dml = authorization.allow_dml.load(Ordering::Acquire);
        let main_database = context
            .database_name
            .is_none_or(|database| database == "main");
        if !main_database {
            return Authorization::Deny;
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
                if allow_transactions {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::CreateTable { table_name }
            | AuthAction::CreateIndex {
                table_name,
                ..
            }
            | AuthAction::AlterTable { table_name, .. } => {
                if allow_schema && is_user_table_name(table_name) {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Read { table_name, .. } => {
                if !allow_schema && !is_user_table_name(table_name) {
                    Authorization::Deny
                } else {
                    Authorization::Allow
                }
            }
            AuthAction::Insert { table_name }
            | AuthAction::Update { table_name, .. }
            | AuthAction::Delete { table_name } => {
                if !allow_dml || (!allow_schema && !is_user_table_name(table_name)) {
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

fn with_authorizer_mode<T>(
    authorization: &Arc<SqliteAuthorizationState>,
    allow_schema: bool,
    allow_transactions: bool,
    allow_dml: bool,
    operation: impl FnOnce() -> MiniAppPlatformResult<T>,
) -> MiniAppPlatformResult<T> {
    authorization
        .allow_schema
        .store(allow_schema, Ordering::Release);
    authorization
        .allow_transactions
        .store(allow_transactions, Ordering::Release);
    authorization.allow_dml.store(allow_dml, Ordering::Release);
    let result = operation();
    authorization.allow_schema.store(false, Ordering::Release);
    authorization
        .allow_transactions
        .store(false, Ordering::Release);
    authorization.allow_dml.store(false, Ordering::Release);
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
            let ordinal = row.get::<_, i64>(0)?;
            if ordinal <= 0 {
                return Err(rusqlite::Error::IntegralValueOutOfRange(0, ordinal));
            }
            let release_json: String = row.get(3)?;
            let release = serde_json::from_str(&release_json).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(MiniAppMigrationLedgerEntry {
                ordinal: u64::try_from(ordinal).map_err(|_| {
                    rusqlite::Error::IntegralValueOutOfRange(0, ordinal)
                })?,
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
    for entry in &entries {
        if entry.migration_id.as_ref().trim().is_empty()
            || !is_digest_value(entry.migration_digest.as_ref())
        {
            return Err(MiniAppPlatformError::Database(
                "private database migration ledger contains an invalid identity".into(),
            ));
        }
        entry
            .release
            .validate()
            .map_err(|error| MiniAppPlatformError::Database(error.to_string()))?;
    }
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

fn is_digest_value(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
        || value.len() > 128
        || !value
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(MiniAppPlatformError::InvalidDatabaseRequest(format!(
            "{field} is not a safe SQL identifier"
        )));
    }
    if is_reserved_host_name(value) {
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
        && !is_reserved_host_name(value)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_reserved_host_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("sqlite_") || lower.starts_with("__nomifun_")
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

fn install_cancellation_handler(
    connection: &Connection,
    cancellation: MiniAppCallCancellation,
) {
    connection.progress_handler(1_000, Some(move || cancellation.is_canceled()));
}

fn canceled_database_error(
    error: impl std::fmt::Display,
    cancellation: &MiniAppCallCancellation,
) -> MiniAppPlatformError {
    if cancellation.is_canceled() {
        MiniAppPlatformError::Canceled
    } else {
        MiniAppPlatformError::Database(error.to_string())
    }
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
