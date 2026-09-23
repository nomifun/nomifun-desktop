use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{PluginId, PluginManifest};
use nomifun_plugin_platform::{
    DataGeneration, InstallArtifactRequest, InstallTarget, NoopPluginBindings,
    ArtifactStoreLimits, NoopPluginRuntime, PluginArtifactStore, PluginBackupExport, PluginBackupFilesystem,
    PluginDataRootManager, PluginGrantMetadata, PluginInstallError, PluginInstallService, PluginRepository,
    PluginRepositoryError, SqlitePluginRepository,
};
use serde_json::json;

fn package(name: &str) -> BTreeMap<String, Vec<u8>> {
    let manifest = json!({
        "schema": "nomifun.plugin/v1",
        "id": name,
        "version": "1.0.0",
        "name": "Backup fixture",
        "description": "Unified Backup install fixture",
        "hostApi": "1.0.0",
        "entrypoints": {"ui": "ui/index.html"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type": "object"},
        "secrets": [],
        "permissions": []
    });
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        ("ui/index.html".into(), b"<!doctype html><main>backup</main>".to_vec()),
    ])
}

fn permission_package(
    version: &str,
    permissions: &[&str],
) -> BTreeMap<String, Vec<u8>> {
    let manifest = json!({
        "schema": "nomifun.plugin/v1",
        "id": "fixture.permissions",
        "version": version,
        "name": "Permission fixture",
        "description": "Permission update fixture",
        "hostApi": ">=1 <2",
        "entrypoints": {"ui": "ui/index.html"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type": "object"},
        "secrets": [],
        "permissions": permissions
    });
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        ("ui/index.html".into(), b"<!doctype html><main>permissions</main>".to_vec()),
    ])
}

fn secret_package(version: &str, secrets: &[&str]) -> BTreeMap<String, Vec<u8>> {
    let manifest = json!({
        "schema": "nomifun.plugin/v1",
        "id": "fixture.secrets",
        "version": version,
        "name": "Secret fixture",
        "description": "Credential binding update fixture",
        "hostApi": ">=1 <2",
        "entrypoints": {"ui": "ui/index.html"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type":"object"},
        "secrets": secrets,
        "permissions": []
    });
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        (
            "ui/index.html".into(),
            b"<!doctype html><main>secrets</main>".to_vec(),
        ),
    ])
}

fn service_package() -> BTreeMap<String, Vec<u8>> {
    let manifest = json!({
        "schema": "nomifun.plugin/v1",
        "id": "fixture.local-service",
        "version": "1.0.0",
        "name": "Local Service fixture",
        "description": "Trusted code confirmation fixture",
        "hostApi": ">=1 <2",
        "entrypoints": {"service": "service/main.mjs", "serviceMode":"onDemand"},
        "actions": {},
        "bindings": [],
        "dataVersion": 0,
        "migrations": [],
        "configSchema": {"type":"object"},
        "secrets": [],
        "permissions": []
    });
    BTreeMap::from([
        (
            "nomifun.plugin.json".into(),
            serde_json::to_vec(&manifest).unwrap(),
        ),
        (
            "service/main.mjs".into(),
            b"export async function activate(){return {async invoke(){return null}}}".to_vec(),
        ),
    ])
}

#[tokio::test]
async fn local_service_trust_is_enforced_by_the_core_before_runtime_validation() {
    let temp = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let install = PluginInstallService::new(
        Arc::new(SqlitePluginRepository::new(database.pool().clone())),
        Arc::new(
            PluginArtifactStore::new(
                temp.path().join("artifacts"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        ),
        Arc::new(
            PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap(),
        ),
        Arc::new(NoopPluginRuntime),
        Arc::new(NoopPluginBindings),
    );
    let error = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner,
                target: InstallTarget::New {
                    plugin_id: PluginId::from(uuid::Uuid::now_v7().to_string()),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &service_package(),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        PluginInstallError::LocalServiceConfirmationRequired
    ));
}

#[tokio::test]
async fn backup_restore_enters_the_same_artifact_journal_and_generation_pipeline() {
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
    let install = PluginInstallService::new(
        repository.clone(),
        artifacts.clone(),
        roots.clone(),
        Arc::new(NoopPluginRuntime),
        Arc::new(NoopPluginBindings),
    );

    let first_id = PluginId::from(uuid::Uuid::now_v7().to_string());
    let first = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: first_id.clone(),
                },
                local_package_id: None,
                config: json!({"theme": "dark"}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &package("fixture.backup"),
            None,
        )
        .await
        .unwrap();
    let replay = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::Existing {
                    plugin_id: first_id.clone(),
                    expected_revision: first.plugin.revision,
                },
                local_package_id: None,
                config: first.plugin.config.clone(),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &package("fixture.backup"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(replay.plugin, first.plugin);
    assert!(repository.list_mutations().await.unwrap().is_empty());
    let first_root = roots
        .open_generation(
            first_id.clone(),
            DataGeneration::new(first.plugin.data_generation.clone()).unwrap(),
        )
        .unwrap();
    first_root
        .storage()
        .kv_set("counter", &json!(7))
        .unwrap();
    first_root
        .storage()
        .file_write("notes/state.txt", b"persisted")
        .unwrap();

    let stored = artifacts.load(&first.artifact.artifact_digest).unwrap();
    let transfer = PluginBackupFilesystem::new(temp.path().join("transfer")).unwrap();
    let backup_path = temp.path().join("plugin-backup.zip");
    transfer
        .export_backup_zip(
            PluginBackupExport {
                plugin_id: &first_id,
                artifact_digest: &first.artifact.artifact_digest,
                generation: &first.plugin.data_generation,
                package_root: &stored.package_root,
                generation_root: first_root.path(),
                config: &json!({"theme": "dark"}),
                grants: &Vec::<PluginGrantMetadata>::new(),
                credential_slots: &BTreeSet::new(),
            },
            &backup_path,
        )
        .unwrap();
    let imported = transfer.import_backup_zip(&backup_path).unwrap();
    assert_eq!(
        PluginManifest::parse(&imported.package.files["nomifun.plugin.json"])
            .unwrap()
            .id,
        "fixture.backup"
    );

    let restored_id = PluginId::from(uuid::Uuid::now_v7().to_string());
    let restored = install
        .install_backup(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: restored_id.clone(),
                },
                local_package_id: Some(format!("fixture.backup.copy.{}", uuid::Uuid::now_v7())),
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            imported,
            None,
        )
        .await
        .unwrap();
    assert_eq!(restored.artifact.artifact_digest, first.artifact.artifact_digest);
    let inventory = repository
        .inventory(&owner, &restored_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(inventory.plugin.config, json!({"theme": "dark"}));
    let restored_root = roots
        .open_generation(
            restored_id,
            DataGeneration::new(restored.plugin.data_generation).unwrap(),
        )
        .unwrap();
    assert_eq!(
        restored_root.storage().kv_get("counter").unwrap().value,
        Some(json!(7))
    );
    assert_eq!(
        restored_root
            .storage()
            .file_read("notes/state.txt")
            .unwrap(),
        b"persisted"
    );
    assert!(repository.list_mutations().await.unwrap().is_empty());

    let mut library = repository.list_library_states(&owner).await.unwrap();
    assert_eq!(library.len(), 2);
    assert!(library.iter().all(|item| item.revision == 1));
    library[0].pinned = true;
    library[0].custom_name = Some("Pinned backup fixture".into());
    assert_eq!(
        repository
            .replace_library_states(&owner, 1, &library)
            .await
            .unwrap(),
        2
    );
    assert!(matches!(
        repository.replace_library_states(&owner, 1, &library).await,
        Err(PluginRepositoryError::Conflict)
    ));
    let saved = repository.list_library_states(&owner).await.unwrap();
    assert!(saved.iter().all(|item| item.revision == 2));
    assert_eq!(saved.iter().filter(|item| item.pinned).count(), 1);
}

#[tokio::test]
async fn unchanged_permissions_keep_revocation_while_expansion_requires_confirmation() {
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
    let install = PluginInstallService::new(
        repository.clone(),
        artifacts,
        roots,
        Arc::new(NoopPluginRuntime),
        Arc::new(NoopPluginBindings),
    );
    let plugin_id = PluginId::from(uuid::Uuid::now_v7().to_string());
    let first = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: plugin_id.clone(),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::from(["desktop.files.open".into()]),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &permission_package("1.0.0", &["desktop.files.open"]),
            None,
        )
        .await
        .unwrap();
    install
        .configure(
            &owner,
            &plugin_id,
            first.plugin.revision,
            json!({}),
            BTreeMap::new(),
            BTreeMap::from([("desktop.files.open".into(), false)]),
        )
        .await
        .unwrap();
    let updated = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::Existing {
                    plugin_id: plugin_id.clone(),
                    expected_revision: 2,
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &permission_package("1.0.1", &["desktop.files.open"]),
            None,
        )
        .await
        .unwrap();
    assert_eq!(updated.plugin.revision, 3);
    assert!(!repository
        .inventory(&owner, &plugin_id)
        .await
        .unwrap()
        .unwrap()
        .grants["desktop.files.open"]
        .granted);

    let expansion = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner,
                target: InstallTarget::Existing {
                    plugin_id,
                    expected_revision: 3,
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &permission_package(
                "1.0.2",
                &["desktop.files.open", "desktop.window.notify"],
            ),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        expansion,
        PluginInstallError::PermissionConfirmationRequired(missing)
            if missing == BTreeSet::from(["desktop.window.notify".into()])
    ));
}

#[tokio::test]
async fn removed_credential_slots_are_pruned_and_never_resurrect_on_readdition() {
    let temp = tempfile::tempdir().unwrap();
    let database = nomifun_db::init_database_memory().await.unwrap();
    let owner = nomifun_db::installation_owner_id(database.pool())
        .await
        .unwrap();
    let repository = Arc::new(SqlitePluginRepository::new(database.pool().clone()));
    let install = PluginInstallService::new(
        repository.clone(),
        Arc::new(
            PluginArtifactStore::new(
                temp.path().join("artifacts"),
                ArtifactStoreLimits::default(),
            )
            .unwrap(),
        ),
        Arc::new(
            PluginDataRootManager::new(temp.path().join("plugin-data")).unwrap(),
        ),
        Arc::new(NoopPluginRuntime),
        Arc::new(NoopPluginBindings),
    );
    let plugin_id = PluginId::from(uuid::Uuid::now_v7().to_string());
    let missing_confirmation = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: plugin_id.clone(),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &secret_package("1.0.0", &["api_key"]),
            None,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        missing_confirmation,
        PluginInstallError::SecretConfirmationRequired(missing)
            if missing == BTreeSet::from(["api_key".into()])
    ));
    let installed = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::New {
                    plugin_id: plugin_id.clone(),
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::from(["api_key".into()]),
                trusted_local_service_confirmed: false,
            },
            &secret_package("1.0.0", &["api_key"]),
            None,
        )
        .await
        .unwrap();
    let configured = install
        .configure(
            &owner,
            &plugin_id,
            installed.plugin.revision,
            json!({}),
            BTreeMap::from([("api_key".into(), "provider:test".into())]),
            BTreeMap::new(),
        )
        .await
        .unwrap();
    assert_eq!(
        repository
            .inventory(&owner, &plugin_id)
            .await
            .unwrap()
            .unwrap()
            .credential_bindings
            .len(),
        1
    );

    let removed = install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::Existing {
                    plugin_id: plugin_id.clone(),
                    expected_revision: configured.revision,
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::new(),
                trusted_local_service_confirmed: false,
            },
            &secret_package("1.1.0", &[]),
            None,
        )
        .await
        .unwrap();
    assert!(
        repository
            .inventory(&owner, &plugin_id)
            .await
            .unwrap()
            .unwrap()
            .credential_bindings
            .is_empty()
    );

    install
        .install_files(
            InstallArtifactRequest {
                owner_user_id: owner.clone(),
                target: InstallTarget::Existing {
                    plugin_id: plugin_id.clone(),
                    expected_revision: removed.plugin.revision,
                },
                local_package_id: None,
                config: json!({}),
                credential_bindings: BTreeMap::new(),
                confirmed_permissions: BTreeSet::new(),
                confirmed_secret_slots: BTreeSet::from(["api_key".into()]),
                trusted_local_service_confirmed: false,
            },
            &secret_package("1.2.0", &["api_key"]),
            None,
        )
        .await
        .unwrap();
    assert!(
        repository
            .inventory(&owner, &plugin_id)
            .await
            .unwrap()
            .unwrap()
            .credential_bindings
            .is_empty(),
        "re-adding a slot must require an explicit new Credential binding"
    );
}
