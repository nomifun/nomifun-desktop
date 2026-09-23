use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    PluginActionPublication, PluginArtifact, PluginDraftId, PluginId, PluginMigrationManifest,
    PluginMutationId,
};
use nomifun_plugin_platform::{
    DataGeneration, InstallArtifactRequest, InstallCommit, InstallTarget, NeverCancel,
    PluginArtifactStore, PluginBindingPort, PluginDataRootHandle, PluginDataRootManager,
    PluginDraftRecord, PluginDraftStatus, PluginInstallError,
    PluginInstallService, PluginMutationKind, PluginMutationPhase, PluginMutationRecord,
    PluginRecord, PluginRepository, PluginRuntimeContext,
    PluginRuntimePort, SqlitePluginRepository, StoredArtifactRecord,
};
use serde_json::json;
use uuid::Uuid;

#[derive(Default)]
struct ControlledRuntime {
    fail_activations: AtomicUsize,
    delay_quiesce: AtomicBool,
    quiesce_inflight: AtomicUsize,
    max_quiesce_inflight: AtomicUsize,
    active: Mutex<BTreeSet<PluginId>>,
}

impl ControlledRuntime {
    fn fail_next_activation(&self) {
        self.fail_activations.store(1, Ordering::Release);
    }

    fn delay_quiesce(&self) {
        self.delay_quiesce.store(true, Ordering::Release);
    }

    fn maximum_quiesce_concurrency(&self) -> usize {
        self.max_quiesce_inflight.load(Ordering::Acquire)
    }
}

#[async_trait]
impl PluginRuntimePort for ControlledRuntime {
    async fn migrate(
        &self,
        _artifact: &PluginArtifact,
        _data_root: &PluginDataRootHandle,
        _migrations: &[PluginMigrationManifest],
    ) -> Result<(), String> {
        Ok(())
    }

    async fn validate(&self, _context: &PluginRuntimeContext) -> Result<(), String> {
        Ok(())
    }

    async fn quiesce(&self, plugin_id: &PluginId) -> Result<(), String> {
        let inflight = self.quiesce_inflight.fetch_add(1, Ordering::AcqRel) + 1;
        self.max_quiesce_inflight
            .fetch_max(inflight, Ordering::AcqRel);
        if self.delay_quiesce.load(Ordering::Acquire) {
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        self.active.lock().unwrap().remove(plugin_id);
        self.quiesce_inflight.fetch_sub(1, Ordering::AcqRel);
        Ok(())
    }

    async fn activate(&self, context: PluginRuntimeContext) -> Result<(), String> {
        if self
            .fail_activations
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err("injected activation failure".into());
        }
        self.active
            .lock()
            .unwrap()
            .insert(context.plugin.plugin_id);
        Ok(())
    }

    async fn remove(&self, plugin_id: &PluginId) -> Result<(), String> {
        self.active.lock().unwrap().remove(plugin_id);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingBindings {
    plugins: Mutex<HashMap<PluginId, bool>>,
}

#[async_trait]
impl PluginBindingPort for RecordingBindings {
    async fn replace(
        &self,
        owner_user_id: &str,
        plugin: &PluginRecord,
        _actions: &[PluginActionPublication],
    ) -> Result<(), String> {
        if plugin.owner_user_id != owner_user_id {
            return Err("owner mismatch".into());
        }
        self.plugins
            .lock()
            .unwrap()
            .insert(plugin.plugin_id.clone(), plugin.is_available());
        Ok(())
    }

    async fn remove(
        &self,
        _owner_user_id: &str,
        plugin_id: &PluginId,
    ) -> Result<(), String> {
        self.plugins.lock().unwrap().remove(plugin_id);
        Ok(())
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    _database: nomifun_db::Database,
    owner: String,
    repository: Arc<SqlitePluginRepository>,
    artifacts: Arc<PluginArtifactStore>,
    roots: Arc<PluginDataRootManager>,
    runtime: Arc<ControlledRuntime>,
    bindings: Arc<RecordingBindings>,
    service: Arc<PluginInstallService>,
}

impl Fixture {
    async fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let database = nomifun_db::init_database_memory().await.unwrap();
        let owner = nomifun_db::installation_owner_id(database.pool())
            .await
            .unwrap();
        let repository = Arc::new(SqlitePluginRepository::new(database.pool().clone()));
        let artifacts = Arc::new(
            PluginArtifactStore::new(
                temp.path().join("artifacts"),
                nomifun_plugin_platform::ArtifactStoreLimits::default(),
            )
            .unwrap(),
        );
        let roots = Arc::new(
            PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap(),
        );
        let runtime = Arc::new(ControlledRuntime::default());
        let bindings = Arc::new(RecordingBindings::default());
        let service = Arc::new(PluginInstallService::new(
            repository.clone(),
            artifacts.clone(),
            roots.clone(),
            runtime.clone(),
            bindings.clone(),
        ));
        Self {
            _temp: temp,
            _database: database,
            owner,
            repository,
            artifacts,
            roots,
            runtime,
            bindings,
            service,
        }
    }

    async fn install(&self, version: &str, target: InstallTarget) -> PluginRecord {
        self.service
            .install_files(
                InstallArtifactRequest {
                    owner_user_id: self.owner.clone(),
                    target,
                    local_package_id: None,
                    config: json!({"theme":"old"}),
                    credential_bindings: BTreeMap::from([(
                        "api_token".into(),
                        "credential-initial".into(),
                    )]),
                    confirmed_permissions: BTreeSet::from(["network".into()]),
                    confirmed_secret_slots: BTreeSet::from(["api_token".into()]),
                    trusted_local_service_confirmed: false,
                },
                &package(version),
                None,
            )
            .await
            .unwrap()
            .plugin
    }

    async fn commit_interrupted_update(
        &self,
        plugin_id: &PluginId,
        current: &PluginRecord,
        version: &str,
        config: serde_json::Value,
        credential_id: &str,
        network_granted: bool,
    ) -> PluginRecord {
        let imported = self
            .artifacts
            .import_files(&package(version), &NeverCancel)
            .unwrap();
        let now = nomifun_common::now_ms().max(1);
        let mutation = PluginMutationRecord {
            mutation_id: PluginMutationId::from(Uuid::now_v7().to_string()),
            owner_user_id: self.owner.clone(),
            plugin_id: plugin_id.clone(),
            kind: PluginMutationKind::Update,
            phase: PluginMutationPhase::Prepared,
            old_artifact_digest: Some(current.active_artifact_digest.clone()),
            new_artifact_digest: Some(imported.stored.artifact.artifact_digest.clone()),
            old_data_generation: Some(current.data_generation.clone()),
            new_data_generation: Some(current.data_generation.clone()),
            expected_revision: Some(current.revision),
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        self.repository.begin_mutation(&mutation).await.unwrap();
        self.repository
            .commit_install(&InstallCommit {
                mutation_id: mutation.mutation_id,
                owner_user_id: self.owner.clone(),
                plugin_id: plugin_id.clone(),
                package_id: imported.stored.artifact.manifest.id.clone(),
                expected_revision: Some(current.revision),
                artifact: StoredArtifactRecord {
                    artifact: imported.stored.artifact,
                    artifact_root: imported
                        .stored
                        .artifact_root
                        .to_string_lossy()
                        .into_owned(),
                    created_at_ms: now,
                },
                data_generation: current.data_generation.clone(),
                config,
                credential_bindings: BTreeMap::from([(
                    "api_token".into(),
                    credential_id.into(),
                )]),
                grants: BTreeMap::from([("network".into(), network_granted)]),
                now_ms: now,
            })
            .await
            .unwrap()
    }
}

fn package(version: &str) -> BTreeMap<String, Vec<u8>> {
    let manifest = json!({
        "schema": "nomifun.plugin/v1",
        "id": "fixture.lifecycle",
        "version": version,
        "name": format!("Lifecycle fixture {version}"),
        "description": format!("Atomic lifecycle fixture {version}"),
        "hostApi": ">=1 <2",
        "entrypoints": {"ui": "ui/index.html"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type":"object"},
        "secrets": ["api_token"],
        "permissions": ["network"]
    });
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        (
            "ui/index.html".into(),
            format!("<!doctype html><main>{version}</main>").into_bytes(),
        ),
    ])
}

#[tokio::test]
async fn committed_update_recovery_converges_to_complete_new_or_complete_old_state() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let original = fixture
        .install(
            "1.0.0",
            InstallTarget::New {
                plugin_id: plugin_id.clone(),
            },
        )
        .await;
    let committed = fixture
        .commit_interrupted_update(
            &plugin_id,
            &original,
            "2.0.0",
            json!({"theme":"stable-v2"}),
            "credential-v2",
            false,
        )
        .await;
    fixture.runtime.active.lock().unwrap().clear();
    fixture.bindings.plugins.lock().unwrap().clear();

    fixture.service.recover().await.unwrap();
    let recovered = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recovered, committed, "successful restart must retain the complete new pointers");
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
    assert!(fixture.runtime.active.lock().unwrap().contains(&plugin_id));
    assert_eq!(
        fixture.bindings.plugins.lock().unwrap().get(&plugin_id),
        Some(&true)
    );
    let recovered_inventory = fixture
        .repository
        .inventory(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();

    let committed_again = fixture
        .commit_interrupted_update(
            &plugin_id,
            &recovered,
            "3.0.0",
            json!({"theme":"candidate-v3"}),
            "credential-v3",
            true,
        )
        .await;
    fixture.runtime.active.lock().unwrap().clear();
    fixture.bindings.plugins.lock().unwrap().clear();
    fixture.runtime.fail_next_activation();

    fixture.service.recover().await.unwrap();
    let rolled_back = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rolled_back.active_artifact_digest,
        recovered.active_artifact_digest,
        "failed restart must restore the complete old Artifact pointer"
    );
    assert_eq!(rolled_back.data_generation, recovered.data_generation);
    assert_eq!(rolled_back.name, recovered.name);
    assert_eq!(rolled_back.description, recovered.description);
    assert_eq!(rolled_back.config, recovered.config);
    assert_eq!(
        rolled_back.previous_artifact_digest,
        recovered.previous_artifact_digest
    );
    assert_eq!(
        rolled_back.previous_data_generation,
        recovered.previous_data_generation
    );
    assert_ne!(rolled_back.active_artifact_digest, committed_again.active_artifact_digest);
    let rolled_back_inventory = fixture
        .repository
        .inventory(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        rolled_back_inventory.credential_bindings,
        recovered_inventory.credential_bindings
    );
    assert_eq!(rolled_back_inventory.grants, recovered_inventory.grants);
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
    assert!(fixture.runtime.active.lock().unwrap().contains(&plugin_id));
    assert_eq!(
        fixture.bindings.plugins.lock().unwrap().get(&plugin_id),
        Some(&true)
    );
}

#[tokio::test]
async fn configure_activation_failure_rolls_back_durable_state_and_readmits_previous() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    fixture
        .install(
            "1.0.0",
            InstallTarget::New {
                plugin_id: plugin_id.clone(),
            },
        )
        .await;
    fixture.runtime.fail_next_activation();

    let error = fixture
        .service
        .configure(
            &fixture.owner,
            &plugin_id,
            1,
            json!({"theme":"new"}),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, PluginInstallError::Activation(_)));
    let inventory = fixture
        .repository
        .inventory(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inventory.plugin.config, json!({"theme":"old"}));
    assert_eq!(inventory.plugin.revision, 3, "commit and rollback are both evidenced");
    assert!(fixture.runtime.active.lock().unwrap().contains(&plugin_id));
}

#[tokio::test]
async fn restore_activation_failure_rolls_back_the_journaled_pointer() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    fixture
        .install(
            "1.0.0",
            InstallTarget::New {
                plugin_id: plugin_id.clone(),
            },
        )
        .await;
    let updated = fixture
        .install(
            "2.0.0",
            InstallTarget::Existing {
                plugin_id: plugin_id.clone(),
                expected_revision: 1,
            },
        )
        .await;
    let active_before_restore = updated.active_artifact_digest.clone();
    fixture.runtime.fail_next_activation();

    let error = fixture
        .service
        .restore_previous(&fixture.owner, &plugin_id, 2, false)
        .await
        .unwrap_err();
    assert!(matches!(error, PluginInstallError::Activation(_)));
    let restored = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.active_artifact_digest, active_before_restore);
    assert_eq!(restored.revision, 4);
    assert!(restored.last_error.is_some());
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
    assert!(fixture.runtime.active.lock().unwrap().contains(&plugin_id));
}

#[tokio::test]
async fn concurrent_lifecycle_writers_share_one_plugin_mutex() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    fixture
        .install(
            "1.0.0",
            InstallTarget::New {
                plugin_id: plugin_id.clone(),
            },
        )
        .await;
    fixture.runtime.delay_quiesce();
    let first = {
        let service = fixture.service.clone();
        let owner = fixture.owner.clone();
        let plugin_id = plugin_id.clone();
        tokio::spawn(async move { service.set_enabled(&owner, &plugin_id, 1, false).await })
    };
    let second = {
        let service = fixture.service.clone();
        let owner = fixture.owner.clone();
        let plugin_id = plugin_id.clone();
        tokio::spawn(async move { service.set_enabled(&owner, &plugin_id, 1, false).await })
    };
    let results = [first.await.unwrap(), second.await.unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert_eq!(fixture.runtime.maximum_quiesce_concurrency(), 1);
    let plugin = fixture
        .repository
        .get_plugin(&fixture.owner, &plugin_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!plugin.enabled);
    assert_eq!(plugin.revision, 2);
}

#[tokio::test]
async fn committed_permanent_delete_recovery_cleans_every_owned_root_and_cache() {
    let fixture = Fixture::new().await;
    let plugin_id = PluginId::from(Uuid::now_v7().to_string());
    let installed = fixture
        .install(
            "1.0.0",
            InstallTarget::New {
                plugin_id: plugin_id.clone(),
            },
        )
        .await;
    let live = fixture
        .roots
        .open_generation(
            plugin_id.clone(),
            DataGeneration::new(installed.data_generation.clone()).unwrap(),
        )
        .unwrap();
    let staged = fixture
        .roots
        .stage_clone(
            &live,
            DataGeneration::new(Uuid::now_v7().to_string()).unwrap(),
        )
        .unwrap();
    live.cache().set("live", json!(true), None).unwrap();
    staged.cache().set("staged", json!(true), None).unwrap();
    let draft_id = PluginDraftId::from(Uuid::now_v7().to_string());
    fixture
        .repository
        .create_draft(&PluginDraftRecord {
            owner_user_id: fixture.owner.clone(),
            draft_id: draft_id.clone(),
            revision: 1,
            plugin_id: Some(plugin_id.clone()),
            base_revision: Some(installed.revision),
            name: "Detached after delete".into(),
            workspace_path: "C:/managed/plugin-draft".into(),
            messages: Vec::new(),
            status: PluginDraftStatus::Ready,
            last_error: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .await
        .unwrap();
    let trashed = fixture
        .service
        .trash(&fixture.owner, &plugin_id, 1)
        .await
        .unwrap();
    let now = nomifun_common::now_ms().max(1);
    let mutation = PluginMutationRecord {
        mutation_id: PluginMutationId::from(Uuid::now_v7().to_string()),
        owner_user_id: fixture.owner.clone(),
        plugin_id: plugin_id.clone(),
        kind: PluginMutationKind::PermanentDelete,
        phase: PluginMutationPhase::Prepared,
        old_artifact_digest: Some(trashed.active_artifact_digest.clone()),
        new_artifact_digest: None,
        old_data_generation: Some(trashed.data_generation.clone()),
        new_data_generation: None,
        expected_revision: Some(trashed.revision),
        error: None,
        created_at_ms: now,
        updated_at_ms: now,
    };
    fixture.repository.begin_mutation(&mutation).await.unwrap();
    fixture
        .repository
        .delete_plugin_rows(
            &mutation.mutation_id,
            &fixture.owner,
            &plugin_id,
            trashed.revision,
        )
        .await
        .unwrap();
    let detached = fixture
        .repository
        .get_draft(&fixture.owner, &draft_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detached.plugin_id, None);
    assert_eq!(detached.base_revision, None);
    assert_eq!(detached.revision, 2);
    assert!(fixture.roots.root().join(plugin_id.as_ref()).exists());

    fixture.service.recover().await.unwrap();

    assert!(!fixture.roots.root().join(plugin_id.as_ref()).exists());
    assert_eq!(live.cache().get("live").unwrap(), None);
    assert_eq!(staged.cache().get("staged").unwrap(), None);
    assert!(fixture.repository.list_mutations().await.unwrap().is_empty());
}
