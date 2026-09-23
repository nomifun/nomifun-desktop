use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use nomifun_agent_contracts::PluginId;
use nomifun_js_runtime::{
    CommittedRuntimeProvider, JavaScriptRuntimeError, JavaScriptWorkKind, ResolvedNodeRuntime,
    RuntimeAuthority, RuntimeUseLease,
};
use nomifun_plugin_platform::{
    ArtifactStoreLimits, DataGeneration, InstallArtifactRequest, InstallTarget,
    NeverCancel, NoopPluginBindings, PluginArtifactStore, PluginDataRootManager,
    PluginInstallError, PluginInstallService, PluginMutationPhase, PluginRecord,
    PluginRepository, PluginRuntimeContext, PluginRuntimePort, PluginServicePorts,
    PluginServiceRuntime, PluginSqlStatement, PluginSqlValue, SqlitePluginRepository,
};
use serde_json::json;
use tokio::sync::Notify;
use uuid::Uuid;

const PACKAGE_ID: &str = "e2e.migration";

struct RealFixture {
    _temp: tempfile::TempDir,
    _database: nomifun_db::Database,
    owner: String,
    repository: Arc<SqlitePluginRepository>,
    roots: Arc<PluginDataRootManager>,
    runtime: Arc<PluginServiceRuntime>,
    install: PluginInstallService,
}

impl RealFixture {
    async fn new() -> Option<Self> {
        let authority = match RuntimeAuthority::discover().await {
            Ok(authority) => authority,
            Err(error) => {
                eprintln!("Node Runtime is unavailable; skipping real migration coverage: {error}");
                return None;
            }
        };
        let temp = tempfile::tempdir().unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let repository = Arc::new(SqlitePluginRepository::new(database.pool().clone()));
        let artifacts = Arc::new(
            PluginArtifactStore::new(
                temp.path().join("artifacts space#hash"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        );
        let roots = Arc::new(
            PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap(),
        );
        let runtime = Arc::new(PluginServiceRuntime::new(
            authority,
            artifacts.clone(),
            PluginServicePorts::default(),
        ));
        let install = PluginInstallService::new(
            repository.clone(),
            artifacts.clone(),
            roots.clone(),
            runtime.clone(),
            Arc::new(NoopPluginBindings),
        );
        Some(Self {
            _temp: temp,
            _database: database,
            owner,
            repository,
            roots,
            runtime,
            install,
        })
    }

    async fn install_new(
        &self,
        plugin_id: PluginId,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> nomifun_plugin_platform::InstallArtifactOutcome {
        self.install
            .install_files(
                InstallArtifactRequest {
                    owner_user_id: self.owner.clone(),
                    target: InstallTarget::New { plugin_id },
                    local_package_id: None,
                    config: json!({}),
                    credential_bindings: BTreeMap::new(),
                    confirmed_permissions: BTreeSet::new(),
                    confirmed_secret_slots: BTreeSet::new(),
                    trusted_local_service_confirmed: false,
                },
                files,
                None,
            )
            .await
            .unwrap()
    }

    async fn update(
        &self,
        plugin_id: PluginId,
        expected_revision: u64,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<nomifun_plugin_platform::InstallArtifactOutcome, PluginInstallError> {
        self.install
            .install_files(
                InstallArtifactRequest {
                    owner_user_id: self.owner.clone(),
                    target: InstallTarget::Existing {
                        plugin_id,
                        expected_revision,
                    },
                    local_package_id: None,
                    config: json!({}),
                    credential_bindings: BTreeMap::new(),
                    confirmed_permissions: BTreeSet::new(),
                    confirmed_secret_slots: BTreeSet::new(),
                    trusted_local_service_confirmed: false,
                },
                files,
                None,
            )
            .await
    }
}

fn manifest(version: &str, data_version: u32, migrations: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "schema":"nomifun.plugin/v1",
        "id":PACKAGE_ID,
        "version":version,
        "name":"Migration fixture",
        "description":"Real Unified Plugin migration fixture.",
        "hostApi":">=1 <2",
        "entrypoints":{"ui":"ui/index.html"},
        "actions":{},
        "bindings":[],
        "dataVersion":data_version,
        "migrations":migrations,
        "configSchema":{"type":"object"},
        "secrets":[],
        "permissions":[]
    }))
    .unwrap()
}

fn v0_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("nomifun.plugin.json".into(), manifest("1.0.0", 0, json!([]))),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>migration-v0</main>".to_vec(),
        ),
    ])
}

fn v1_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            manifest(
                "1.1.0",
                1,
                json!([{"id":"v0_to_v1","from":0,"to":1,"path":"migrations/0001-v1.mjs"}]),
            ),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>migration-v1</main>".to_vec(),
        ),
        (
            "migrations/0001-v1.mjs".into(),
            br#"export async function migrate(ctx) {
  await ctx.storage.db.execute("ALTER TABLE items ADD COLUMN migrated INTEGER NOT NULL DEFAULT 0");
  await ctx.storage.db.execute("UPDATE items SET body = body || '-v1', migrated = 1");
  const previous = (await ctx.storage.files.read("state.txt")).toString("utf8");
  await ctx.storage.files.write("state.txt", `${previous}-v1`);
  await ctx.storage.files.write("migration/v1.txt", "created-by-v1");
  await ctx.storage.kv.set("schema-version", 1);
}
"#
            .to_vec(),
        ),
    ])
}

fn v0_code_update_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            manifest("1.0.1", 0, json!([])),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>migration-v0-code-update</main>".to_vec(),
        ),
    ])
}

fn failing_v2_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            manifest(
                "2.0.0",
                2,
                json!([{"id":"v1_to_v2","from":1,"to":2,"path":"migrations/0002-v2.mjs"}]),
            ),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>migration-v2-fails</main>".to_vec(),
        ),
        (
            "migrations/0002-v2.mjs".into(),
            br#"export async function migrate(ctx) {
  await ctx.storage.db.execute("UPDATE items SET body = 'corrupted-v2', migrated = 2");
  await ctx.storage.files.write("state.txt", "corrupted-v2");
  await ctx.storage.files.write("migration/v2.txt", "must-be-discarded");
  throw new Error("injected v2 migration failure");
}
"#
            .to_vec(),
        ),
    ])
}

fn gap_v2_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            manifest(
                "2.0.0",
                2,
                json!([{"id":"v1_to_v2","from":1,"to":2,"path":"migrations/0002-only.mjs"}]),
            ),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>migration-gap</main>".to_vec(),
        ),
        (
            "migrations/0002-only.mjs".into(),
            b"export async function migrate() {}\n".to_vec(),
        ),
    ])
}

fn create_v0_data(root: &nomifun_plugin_platform::PluginDataRootHandle) {
    let storage = root.storage();
    storage
        .db_execute(&PluginSqlStatement::new(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
            vec![],
        ))
        .unwrap();
    storage
        .db_execute(&PluginSqlStatement::new(
            "INSERT INTO items (id, body) VALUES (?1, ?2)",
            vec![
                PluginSqlValue::Integer(1),
                PluginSqlValue::Text("original".into()),
            ],
        ))
        .unwrap();
    storage.file_write("state.txt", b"original-file").unwrap();
    storage.kv_set("schema-version", &json!(0)).unwrap();
}

fn query_item(
    root: &nomifun_plugin_platform::PluginDataRootHandle,
) -> Vec<Vec<PluginSqlValue>> {
    root.storage()
        .db_query(&PluginSqlStatement::new(
            "SELECT body, migrated FROM items ORDER BY id",
            vec![],
        ))
        .unwrap()
        .rows
}

fn staging_is_empty(roots: &PluginDataRootManager, plugin_id: &PluginId) -> bool {
    std::fs::read_dir(roots.root().join(plugin_id.as_ref()).join("staging"))
        .unwrap()
        .next()
        .is_none()
}

#[tokio::test]
async fn same_data_version_update_switches_only_code_and_restore_reuses_the_live_data() {
    let Some(fixture) = RealFixture::new().await else {
        return;
    };
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let installed = fixture
        .install_new(plugin_id.clone(), &v0_package())
        .await;
    let root = fixture
        .roots
        .open_generation(
            plugin_id.clone(),
            DataGeneration::new(installed.plugin.data_generation.clone()).unwrap(),
        )
        .unwrap();
    create_v0_data(&root);

    let updated = fixture
        .update(
            plugin_id.clone(),
            installed.plugin.revision,
            &v0_code_update_package(),
        )
        .await
        .unwrap();
    assert_ne!(
        updated.plugin.active_artifact_digest,
        installed.plugin.active_artifact_digest
    );
    assert_eq!(
        updated.plugin.previous_artifact_digest.as_ref(),
        Some(&installed.plugin.active_artifact_digest)
    );
    assert_eq!(
        updated.plugin.data_generation,
        installed.plugin.data_generation,
        "same-dataVersion updates must reuse the live generation"
    );
    assert!(updated.plugin.previous_data_generation.is_none());
    assert_eq!(root.storage().file_read("state.txt").unwrap(), b"original-file");
    assert_eq!(
        root.storage()
            .db_query(&PluginSqlStatement::new(
                "SELECT body FROM items ORDER BY id",
                vec![],
            ))
            .unwrap()
            .rows,
        [vec![PluginSqlValue::Text("original".into())]]
    );

    let restored = fixture
        .install
        .restore_previous(
            &fixture.owner,
            &plugin_id,
            updated.plugin.revision,
            false,
        )
        .await
        .unwrap();
    assert_eq!(
        restored.active_artifact_digest,
        installed.plugin.active_artifact_digest
    );
    assert_eq!(restored.data_generation, installed.plugin.data_generation);
    assert!(restored.previous_data_generation.is_none());
    assert_eq!(root.storage().file_read("state.txt").unwrap(), b"original-file");
    fixture.runtime.shutdown().await.unwrap();
}

#[tokio::test]
async fn real_node_migration_switches_atomically_and_failed_next_version_is_fully_discarded() {
    let Some(fixture) = RealFixture::new().await else {
        return;
    };
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let installed = fixture
        .install_new(plugin_id.clone(), &v0_package())
        .await;
    let v0_root = fixture
        .roots
        .open_generation(
            plugin_id.clone(),
            DataGeneration::new(installed.plugin.data_generation.clone()).unwrap(),
        )
        .unwrap();
    create_v0_data(&v0_root);

    let migrated = fixture
        .update(plugin_id.clone(), installed.plugin.revision, &v1_package())
        .await
        .unwrap();
    assert_ne!(migrated.plugin.data_generation, installed.plugin.data_generation);
    assert_eq!(
        migrated.plugin.previous_data_generation.as_deref(),
        Some(installed.plugin.data_generation.as_str())
    );
    assert_eq!(
        migrated.plugin.previous_artifact_digest.as_ref(),
        Some(&installed.plugin.active_artifact_digest)
    );
    let v1_root = fixture
        .roots
        .open_generation(
            plugin_id.clone(),
            DataGeneration::new(migrated.plugin.data_generation.clone()).unwrap(),
        )
        .unwrap();
    assert_eq!(
        query_item(&v1_root),
        [vec![
            PluginSqlValue::Text("original-v1".into()),
            PluginSqlValue::Integer(1),
        ]]
    );
    assert_eq!(v1_root.storage().file_read("state.txt").unwrap(), b"original-file-v1");
    assert_eq!(
        v1_root.storage().file_read("migration/v1.txt").unwrap(),
        b"created-by-v1"
    );
    assert_eq!(
        v1_root.storage().kv_get("schema-version").unwrap().value,
        Some(json!(1))
    );
    assert_eq!(v1_root.storage().migrations().unwrap().len(), 1);
    assert_eq!(
        v0_root
            .storage()
            .db_query(&PluginSqlStatement::new(
                "SELECT body FROM items ORDER BY id",
                vec![],
            ))
            .unwrap()
            .rows,
        [vec![PluginSqlValue::Text("original".into())]]
    );
    assert_eq!(v0_root.storage().file_read("state.txt").unwrap(), b"original-file");

    let before_failure = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    let failure = fixture
        .update(
            plugin_id.clone(),
            before_failure.revision,
            &failing_v2_package(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&failure, PluginInstallError::Validation(message) if message.contains("migration failed")),
        "unexpected failure: {failure}"
    );
    let after_failure = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_failure, before_failure, "failed migration changed an authority pointer");
    assert_eq!(
        query_item(&v1_root),
        [vec![
            PluginSqlValue::Text("original-v1".into()),
            PluginSqlValue::Integer(1),
        ]]
    );
    assert_eq!(v1_root.storage().file_read("state.txt").unwrap(), b"original-file-v1");
    assert!(v1_root.storage().file_read("migration/v2.txt").is_err());
    assert!(staging_is_empty(&fixture.roots, &plugin_id));
    let mutations = fixture.repository.list_mutations().await.unwrap();
    assert_eq!(mutations.len(), 1);
    assert_eq!(mutations[0].phase, PluginMutationPhase::Failed);

    fixture.install.recover().await.unwrap();
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
    assert!(staging_is_empty(&fixture.roots, &plugin_id));
    assert_eq!(
        fixture
            .repository
            .get_plugin(&fixture.owner, &plugin_id)
            .await
            .unwrap()
            .unwrap(),
        before_failure
    );
    let restored = fixture
        .install
        .restore_previous(
            &fixture.owner,
            &plugin_id,
            before_failure.revision,
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        restored.active_artifact_digest,
        installed.plugin.active_artifact_digest
    );
    assert_eq!(restored.data_generation, installed.plugin.data_generation);
    assert_eq!(
        restored.previous_data_generation.as_deref(),
        Some(migrated.plugin.data_generation.as_str())
    );
    assert_eq!(
        v0_root
            .storage()
            .db_query(&PluginSqlStatement::new(
                "SELECT body FROM items ORDER BY id",
                vec![],
            ))
            .unwrap()
            .rows,
        [vec![PluginSqlValue::Text("original".into())]],
        "full Previous restore must select the exact pre-migration DataRoot"
    );
    fixture.runtime.shutdown().await.unwrap();
    assert!(matches!(
        fixture
            .runtime
            .invoke(
                &plugin_id,
                "after-shutdown",
                json!({}),
                Vec::new(),
                Default::default(),
            )
            .await,
        Err(nomifun_plugin_platform::PluginServiceError::InvalidConfiguration(message))
            if message.contains("closed")
    ));
}

#[tokio::test]
async fn migration_chain_gap_is_rejected_before_node_and_leaves_no_staging_or_journal() {
    let Some(fixture) = RealFixture::new().await else {
        return;
    };
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let installed = fixture
        .install_new(plugin_id.clone(), &v0_package())
        .await;
    let root = fixture
        .roots
        .open_generation(
            plugin_id.clone(),
            DataGeneration::new(installed.plugin.data_generation.clone()).unwrap(),
        )
        .unwrap();
    create_v0_data(&root);

    let error = fixture
        .update(plugin_id.clone(), installed.plugin.revision, &gap_v2_package())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, PluginInstallError::InvalidUpdate(message) if message.contains("migration chain does not cover dataVersion 0")),
        "unexpected error: {error}"
    );
    assert!(staging_is_empty(&fixture.roots, &plugin_id));
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
    let current = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.active_artifact_digest, installed.plugin.active_artifact_digest);
    assert_eq!(current.data_generation, installed.plugin.data_generation);
    assert_eq!(current.revision, installed.plugin.revision);
}

struct BlockingRuntimeProvider {
    inner: Arc<RuntimeAuthority>,
    entered: Notify,
    release: Notify,
    acquisitions: AtomicUsize,
}

impl BlockingRuntimeProvider {
    fn new(inner: Arc<RuntimeAuthority>) -> Self {
        Self {
            inner,
            entered: Notify::new(),
            release: Notify::new(),
            acquisitions: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl CommittedRuntimeProvider for BlockingRuntimeProvider {
    async fn acquire_use(
        &self,
        kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        self.acquisitions.fetch_add(1, Ordering::AcqRel);
        self.entered.notify_one();
        self.release.notified().await;
        self.inner.acquire_use(kind).await
    }

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        self.inner.committed_runtime().await
    }
}

fn continuous_service_package() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec_pretty(&json!({
                "schema":"nomifun.plugin/v1",
                "id":"e2e.shutdown-race",
                "version":"1.0.0",
                "name":"Shutdown race fixture",
                "description":"Proves Plugin Service shutdown admission.",
                "hostApi":">=1 <2",
                "entrypoints":{"service":"service/main.mjs","serviceMode":"continuous"},
                "actions":{},
                "bindings":[],
                "dataVersion":0,
                "migrations":[],
                "configSchema":{"type":"object"},
                "secrets":[],
                "permissions":[]
            }))
            .unwrap(),
        ),
        (
            "service/main.mjs".into(),
            br#"export async function activate(ctx) {
  await ctx.storage.kv.set("activated", true);
  return { async invoke() { return null; } };
}
"#
            .to_vec(),
        ),
    ])
}

#[tokio::test]
async fn shutdown_waits_for_admitted_spawn_then_stops_it_and_permanently_fences_restart() {
    let authority = match RuntimeAuthority::discover().await {
        Ok(authority) => authority,
        Err(error) => {
            eprintln!("Node Runtime is unavailable; skipping shutdown race coverage: {error}");
            return;
        }
    };
    let temp = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(
        PluginArtifactStore::new(
            temp.path().join("artifacts"),
            ArtifactStoreLimits::default(),
        )
        .unwrap(),
    );
    let imported = artifacts
        .import_files(&continuous_service_package(), &NeverCancel)
        .unwrap();
    let artifact = imported.stored.artifact;
    let roots = PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap();
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let generation = DataGeneration::new(Uuid::now_v7().to_string()).unwrap();
    let root = roots
        .create_empty_generation(plugin_id.clone(), generation.clone())
        .unwrap();
    let plugin = PluginRecord {
        owner_user_id: Uuid::now_v7().to_string(),
        plugin_id: plugin_id.clone(),
        package_id: artifact.manifest.id.clone(),
        name: artifact.manifest.name.clone(),
        description: artifact.manifest.description.clone(),
        enabled: true,
        trashed_at_ms: None,
        active_artifact_digest: artifact.artifact_digest.clone(),
        previous_artifact_digest: None,
        data_generation: generation.as_str().to_owned(),
        previous_data_generation: None,
        revision: 1,
        config: json!({}),
        last_error: None,
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    let context = PluginRuntimeContext {
        owner_user_id: plugin.owner_user_id.clone(),
        plugin,
        artifact,
        data_root: root.clone(),
        credential_bindings: BTreeMap::new(),
        granted_permissions: BTreeSet::new(),
    };
    let provider = Arc::new(BlockingRuntimeProvider::new(authority));
    let runtime = Arc::new(PluginServiceRuntime::new(
        provider.clone(),
        artifacts,
        PluginServicePorts::default(),
    ));

    let activation = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.activate(context).await })
    };
    provider.entered.notified().await;
    let mut shutdown = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.shutdown().await })
    };
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(75), &mut shutdown)
            .await
            .is_err(),
        "shutdown crossed an admitted spawn instead of waiting for its read guard"
    );

    provider.release.notify_waiters();
    activation.await.unwrap().unwrap();
    shutdown.await.unwrap().unwrap();
    assert_eq!(provider.acquisitions.load(Ordering::Acquire), 1);
    assert_eq!(
        root.storage().kv_get("activated").unwrap().value,
        Some(json!(true)),
        "the pre-shutdown call must have reached the real Node activate hook"
    );
    assert_eq!(
        runtime.observation(&plugin_id).await,
        nomifun_plugin_platform::PluginServiceObservation::Stopped
    );
    assert!(matches!(
        runtime
            .invoke(
                &plugin_id,
                "after-shutdown",
                json!({}),
                Vec::new(),
                Default::default(),
            )
            .await,
        Err(nomifun_plugin_platform::PluginServiceError::InvalidConfiguration(message))
            if message.contains("closed")
    ));
}

#[derive(Default)]
struct CountingUnavailableRuntime {
    acquisitions: AtomicUsize,
}

#[async_trait]
impl CommittedRuntimeProvider for CountingUnavailableRuntime {
    async fn acquire_use(
        &self,
        _kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        self.acquisitions.fetch_add(1, Ordering::AcqRel);
        Err(JavaScriptRuntimeError::Unavailable)
    }

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        Ok(None)
    }
}

#[tokio::test]
async fn ui_only_install_and_activation_never_acquire_a_node_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let repository = Arc::new(SqlitePluginRepository::new(database.pool().clone()));
    let artifacts = Arc::new(
        PluginArtifactStore::new(
            temp.path().join("artifacts"),
            ArtifactStoreLimits::default(),
        )
        .unwrap(),
    );
    let roots = Arc::new(
        PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap(),
    );
    let authority = Arc::new(CountingUnavailableRuntime::default());
    let runtime = Arc::new(PluginServiceRuntime::new(
        authority.clone(),
        artifacts.clone(),
        PluginServicePorts::default(),
    ));
    let install = PluginInstallService::new(
        repository.clone(),
        artifacts,
        roots,
        runtime,
        Arc::new(NoopPluginBindings),
    );

    let invalid_plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let mut invalid_ui = v0_package();
    invalid_ui.insert("ui/index.html".into(), vec![0xff, 0xfe]);
    let invalid = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: invalid_plugin_id.clone(),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &invalid_ui,
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(invalid, PluginInstallError::Validation(message) if message.contains("UTF-8 HTML"))
    );
    assert!(
        repository
            .get_plugin(&owner, &invalid_plugin_id)
            .await
            .unwrap()
            .is_none(),
        "a failed static UI load check must not publish a Plugin"
    );

    install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner,
                target: InstallTarget::New {
                    plugin_id: PluginId::from(Uuid::now_v7().to_string()),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &v0_package(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(authority.acquisitions.load(Ordering::Acquire), 0);
}
