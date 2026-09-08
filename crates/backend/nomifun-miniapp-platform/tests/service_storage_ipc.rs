use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ArtifactId, DigestHex, MiniAppAdditiveMigrationAction, MiniAppBridgeCallId,
    MiniAppBridgeKvRequest, MiniAppId, MiniAppMigration, MiniAppMigrationColumn, MiniAppMigrationId,
    MiniAppReleaseId,
    MiniAppReleaseRef, MiniAppServiceLifecycle, MiniAppServiceRuntimeFingerprint,
    MiniAppServiceStorageDescriptor, ResolvedMiniAppServiceSpec, ResolvedMiniAppServiceSpecInputs,
    RuntimeInstallationId, RuntimeTarget, StrictJsonValue, VersionString, digest_bytes,
};
use nomifun_db::{
    CreateMiniAppM1Params, IMiniAppM1Repository, MiniAppM1Kind, SqliteMiniAppM1Repository,
    init_database_memory, installation_owner_id,
};
use nomifun_miniapp_platform::{
    FixedMiniAppServiceModuleResolver, InMemoryMiniAppManagedStorage,
    MiniAppCallCancellation, MiniAppServiceInvocation, MiniAppServiceProcessFactory,
    MiniAppServiceProcessLimits, MiniAppServiceStoragePort, NodeMiniAppServiceProcessFactory,
    MiniAppServiceStorageRequest, MiniAppServiceStorageResolution, MiniAppPlatformError,
    SqliteMiniAppManagedStorage,
};
use serde_json::json;
use tempfile::TempDir;
use uuid::Uuid;

const STORAGE_SERVICE: &str = r#"
export async function start(context) {
  const boot = await context.storage.kv.set("boot", { ready: true });
  return {
    async invoke({ method }) {
      if (method !== "storage") throw new Error("unexpected method");
      const written = await context.storage.kv.set("state", { value: 7 });
      const value = await context.storage.kv.get("state");
      const bootValue = await context.storage.kv.get("boot");
      const query = context.storage.database
        ? await context.storage.database.query(
            "SELECT id FROM state WHERE id = ?",
            ["one"],
          )
        : null;
      return {
        filesDir: context.storage.filesDir,
        databaseAvailable: Boolean(context.storage.database),
        written,
        value,
        bootValue,
        query,
      };
    },
    async dispose() {},
  };
}
"#;

const SQLITE_STORAGE_SERVICE: &str = r#"
export async function start(context) {
  return {
    async invoke({ method }) {
      if (method !== "storage") throw new Error("unexpected method");
      const inserted = await context.storage.database.execute(
        "INSERT INTO state (id, value) VALUES (?, ?)",
        ["node", 11],
      );
      const selected = await context.storage.database.query(
        "SELECT id, value FROM state WHERE id = ?",
        ["node"],
      );
      return { inserted, selected };
    },
    async dispose() {},
  };
}
"#;

const CONCURRENT_STORAGE_SERVICE: &str = r#"
export async function start(context) {
  return {
    async invoke({ method }) {
      if (method === "slow") {
        return await context.storage.kv.get("slow");
      }
      if (method === "fast") return { fast: true };
      throw new Error("unexpected method");
    },
    async dispose() {},
  };
}
"#;

fn digest(seed: &str) -> DigestHex {
    digest_bytes(seed.as_bytes())
}

fn node_executable() -> Option<PathBuf> {
    std::fs::canonicalize(which::which("node").ok()?).ok()
}

fn spec(
    node: &std::path::Path,
    miniapp_id: MiniAppId,
    storage: MiniAppServiceStorageDescriptor,
    module: &[u8],
) -> ResolvedMiniAppServiceSpec {
    let runtime_bytes = std::fs::read(node).unwrap();
    let version = std::process::Command::new(node)
        .arg("--version")
        .output()
        .unwrap();
    let node_version = String::from_utf8(version.stdout)
        .unwrap()
        .trim()
        .trim_start_matches('v')
        .to_owned();
    ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
        miniapp_id: miniapp_id.clone(),
        release: MiniAppReleaseRef {
            release_id: MiniAppReleaseId::from(Uuid::now_v7().to_string()),
            artifact_id: ArtifactId::from(Uuid::now_v7().to_string()),
            release_digest: digest("release"),
            manifest_digest: digest("manifest"),
        },
        active_release_epoch: 1,
        service_module_digest: digest_bytes(module),
        lifecycle: MiniAppServiceLifecycle::OnDemand,
        host_protocol_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
        sdk_contract_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
        runtime: MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("storage-ipc-node"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest_bytes(&runtime_bytes),
            node_version: VersionString::from(node_version),
        },
        config_schema_digest: digest("config"),
        config_snapshot_digest: digest("config-snapshot"),
        credential_slots_digest: digest("credentials"),
        resource_contract_digest: digest("resources"),
        resource_bindings_digest: digest("bindings"),
        runtime_requirements_digest: digest("runtime-requirements"),
        bridge_contract_digest: digest("bridge"),
        contribution_set_digest: digest("contributions"),
        storage,
    })
    .unwrap()
}

#[tokio::test]
async fn real_node_service_round_trips_host_storage_without_exposing_db_path() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping Service Storage IPC test");
        return;
    };
    let temp = TempDir::new().unwrap();
    let module = temp.path().join("main.mjs");
    tokio::fs::write(&module, STORAGE_SERVICE).await.unwrap();
    let storage = Arc::new(InMemoryMiniAppManagedStorage::new());
    let miniapp_id = MiniAppId::from("miniapp-storage-ipc");
    let resolved = storage
        .resolve_service_storage("owner", &miniapp_id, true, true)
        .await
        .unwrap();
    let service_spec = spec(
        &node,
        miniapp_id.clone(),
        resolved.descriptor.clone(),
        STORAGE_SERVICE.as_bytes(),
    );
    let process = NodeMiniAppServiceProcessFactory::new(
        node.clone(),
        Arc::new(FixedMiniAppServiceModuleResolver::new(module)),
    )
    .unwrap()
    .with_storage(storage.clone())
    .with_limits(MiniAppServiceProcessLimits {
        hello_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_secs(3),
        max_frame_bytes: 1024 * 1024,
        command_queue_capacity: 32,
        cancellation_poll_interval: Duration::from_millis(5),
    })
    .unwrap()
    .start(nomifun_miniapp_platform::MiniAppServiceLaunch {
        spec: service_spec.clone(),
        host_generation: 1,
    })
    .await
    .unwrap();
    let result = process
        .invoke(
            MiniAppServiceInvocation {
                fence: nomifun_miniapp_platform::MiniAppServiceGenerationFence {
                    miniapp_id: service_spec.miniapp_id.clone(),
                    release: service_spec.release.clone(),
                    active_release_epoch: service_spec.active_release_epoch,
                    service_run_key: service_spec.service_run_key.clone(),
                    host_generation: 1,
                },
                call_id: MiniAppBridgeCallId::from("storage-call"),
                method: "storage".into(),
                payload: StrictJsonValue(json!({})),
            },
            MiniAppCallCancellation::default(),
        )
        .await;
    process.stop().await;
    let result = result.expect("storage IPC invocation failed");
    assert_eq!(result.0["value"]["value"]["value"], json!(7));
    assert_eq!(result.0["bootValue"]["value"]["ready"], json!(true));
    assert!(result.0["filesDir"].is_string());
    assert_eq!(result.0["databaseAvailable"], json!(true));
}

#[tokio::test]
async fn real_node_service_round_trips_production_sqlite_storage() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping production SQLite Storage IPC test");
        return;
    };
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let miniapp_id = Uuid::now_v7().to_string();
    let project_id = Uuid::now_v7().to_string();
    SqliteMiniAppM1Repository::new(database.pool().clone())
        .create(&CreateMiniAppM1Params {
            owner_user_id: owner.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id,
            expected_library_revision: 0,
            display_name: "Node SQLite fixture".into(),
            description: None,
            icon_asset_id: None,
            kind: MiniAppM1Kind::Service,
            materialized_catalog_digest: digest("catalog").as_ref().to_owned(),
            config_schema_json: r#"{"type":"object"}"#.into(),
            config_json: "{}".into(),
            created_at: 1,
        })
        .await
        .unwrap();

    let temp = TempDir::new().unwrap();
    let module = temp.path().join("main.mjs");
    tokio::fs::write(&module, SQLITE_STORAGE_SERVICE)
        .await
        .unwrap();
    let storage = Arc::new(
        SqliteMiniAppManagedStorage::new(temp.path().join("managed"), database.pool().clone())
            .unwrap(),
    );
    let miniapp = MiniAppId::from(miniapp_id);
    let resolved = storage
        .resolve_service_storage(&owner, &miniapp, false, true)
        .await
        .unwrap();
    let migration = MiniAppMigration::new(
        MiniAppMigrationId::from("001_create_state"),
        vec![MiniAppAdditiveMigrationAction::CreateTable {
            table_name: "state".into(),
            columns: vec![
                MiniAppMigrationColumn {
                    name: "id".into(),
                    declared_type: "TEXT".into(),
                    nullable: false,
                    default_literal: None,
                },
                MiniAppMigrationColumn {
                    name: "value".into(),
                    declared_type: "INTEGER".into(),
                    nullable: false,
                    default_literal: Some("0".into()),
                },
            ],
            primary_key_columns: vec!["id".into()],
        }],
    )
    .unwrap();
    let db_descriptor = resolved.descriptor.private_database.as_ref().unwrap();
    let release = MiniAppReleaseRef {
        release_id: "release-node-sqlite".into(),
        artifact_id: "artifact-node-sqlite".into(),
        release_digest: digest("release-node-sqlite"),
        manifest_digest: digest("manifest-node-sqlite"),
    };
    storage
        .apply_additive_migrations(
            &owner,
            &miniapp,
            &resolved.descriptor,
            &db_descriptor.migration_ledger_digest,
            &release,
            std::slice::from_ref(&migration),
            2,
        )
        .await
        .unwrap();
    let resolved = storage
        .resolve_service_storage(&owner, &miniapp, false, true)
        .await
        .unwrap();
    let service_spec = spec(
        &node,
        miniapp.clone(),
        resolved.descriptor,
        SQLITE_STORAGE_SERVICE.as_bytes(),
    );
    let process = NodeMiniAppServiceProcessFactory::new(
        node.clone(),
        Arc::new(FixedMiniAppServiceModuleResolver::new(module)),
    )
    .unwrap()
    .with_storage(storage)
    .with_limits(MiniAppServiceProcessLimits {
        hello_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(5),
        shutdown_timeout: Duration::from_secs(3),
        max_frame_bytes: 1024 * 1024,
        command_queue_capacity: 32,
        cancellation_poll_interval: Duration::from_millis(5),
    })
    .unwrap()
    .start(nomifun_miniapp_platform::MiniAppServiceLaunch {
        spec: service_spec.clone(),
        host_generation: 1,
    })
    .await
    .unwrap();
    let result = process
        .invoke(
            MiniAppServiceInvocation {
                fence: nomifun_miniapp_platform::MiniAppServiceGenerationFence {
                    miniapp_id: service_spec.miniapp_id.clone(),
                    release: service_spec.release.clone(),
                    active_release_epoch: service_spec.active_release_epoch,
                    service_run_key: service_spec.service_run_key.clone(),
                    host_generation: 1,
                },
                call_id: MiniAppBridgeCallId::from("sqlite-storage-call"),
                method: "storage".into(),
                payload: StrictJsonValue(json!({})),
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .expect("production SQLite Storage IPC invocation must succeed");
    process.stop().await;
    assert_eq!(result.0["inserted"]["affected_rows"], json!(1));
    assert_eq!(
        result.0["selected"]["rows"][0],
        json!({"id": "node", "value": 11})
    );
}

#[derive(Clone)]
struct SlowStorage {
    inner: Arc<InMemoryMiniAppManagedStorage>,
    started: Arc<tokio::sync::Notify>,
    active: Arc<AtomicBool>,
}

#[async_trait]
impl MiniAppServiceStoragePort for SlowStorage {
    async fn resolve_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> nomifun_miniapp_platform::MiniAppPlatformResult<MiniAppServiceStorageResolution> {
        self.inner
            .resolve_service_storage(
                owner_user_id,
                miniapp_id,
                uses_files,
                uses_private_database,
            )
            .await
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
    ) -> nomifun_miniapp_platform::MiniAppPlatformResult<
        nomifun_miniapp_platform::MiniAppMigrationLedger,
    > {
        self.inner
            .apply_additive_migrations(
                owner_user_id,
                miniapp_id,
                storage,
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
        request: MiniAppServiceStorageRequest,
        cancellation: MiniAppCallCancellation,
    ) -> nomifun_miniapp_platform::MiniAppPlatformResult<StrictJsonValue> {
        let slow = matches!(
            &request,
            MiniAppServiceStorageRequest::Kv {
                request: MiniAppBridgeKvRequest::Get { key }
            } if key == "slow"
        );
        if slow {
            self.active.store(true, Ordering::Release);
            self.started.notify_waiters();
            while !cancellation.is_canceled() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            self.active.store(false, Ordering::Release);
            return Err(MiniAppPlatformError::Canceled);
        }
        self.inner
            .handle_service_request(miniapp_id, storage, request, cancellation)
            .await
    }

    async fn purge_service_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> nomifun_miniapp_platform::MiniAppPlatformResult<()> {
        self.inner
            .purge_service_storage(owner_user_id, miniapp_id)
            .await
    }
}

#[tokio::test]
async fn slow_storage_request_does_not_block_other_calls_and_cancel_reaches_host() {
    let Some(node) = node_executable() else {
        eprintln!("Node is unavailable; skipping concurrent Storage IPC test");
        return;
    };
    let temp = TempDir::new().unwrap();
    let module = temp.path().join("main.mjs");
    tokio::fs::write(&module, CONCURRENT_STORAGE_SERVICE)
        .await
        .unwrap();
    let inner = Arc::new(InMemoryMiniAppManagedStorage::new());
    let storage = Arc::new(SlowStorage {
        inner: inner.clone(),
        started: Arc::new(tokio::sync::Notify::new()),
        active: Arc::new(AtomicBool::new(false)),
    });
    let miniapp = MiniAppId::from("miniapp-concurrent-storage");
    let resolved = storage
        .resolve_service_storage("owner", &miniapp, false, false)
        .await
        .unwrap();
    let service_spec = spec(
        &node,
        miniapp,
        resolved.descriptor,
        CONCURRENT_STORAGE_SERVICE.as_bytes(),
    );
    let process = NodeMiniAppServiceProcessFactory::new(
        node,
        Arc::new(FixedMiniAppServiceModuleResolver::new(module)),
    )
    .unwrap()
    .with_storage(storage.clone())
    .with_limits(MiniAppServiceProcessLimits {
        hello_timeout: Duration::from_secs(5),
        request_timeout: Duration::from_secs(3),
        shutdown_timeout: Duration::from_secs(3),
        max_frame_bytes: 1024 * 1024,
        command_queue_capacity: 32,
        cancellation_poll_interval: Duration::from_millis(5),
    })
    .unwrap()
    .start(nomifun_miniapp_platform::MiniAppServiceLaunch {
        spec: service_spec.clone(),
        host_generation: 1,
    })
    .await
    .unwrap();

    let slow_cancellation = MiniAppCallCancellation::default();
    let slow_process = process.clone();
    let slow_spec = service_spec.clone();
    let slow_cancellation_for_task = slow_cancellation.clone();
    let slow = tokio::spawn(async move {
        slow_process
            .invoke(
                MiniAppServiceInvocation {
                    fence: nomifun_miniapp_platform::MiniAppServiceGenerationFence {
                        miniapp_id: slow_spec.miniapp_id.clone(),
                        release: slow_spec.release.clone(),
                        active_release_epoch: slow_spec.active_release_epoch,
                        service_run_key: slow_spec.service_run_key.clone(),
                        host_generation: 1,
                    },
                    call_id: MiniAppBridgeCallId::from("slow-call"),
                    method: "slow".into(),
                    payload: StrictJsonValue(json!({})),
                },
                slow_cancellation_for_task,
            )
            .await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        while !storage.active.load(Ordering::Acquire) {
            storage.started.notified().await;
        }
    })
    .await
    .expect("slow Storage request must reach Host");

    let fast = tokio::time::timeout(
        Duration::from_secs(1),
        process.invoke(
            MiniAppServiceInvocation {
                fence: nomifun_miniapp_platform::MiniAppServiceGenerationFence {
                    miniapp_id: service_spec.miniapp_id.clone(),
                    release: service_spec.release.clone(),
                    active_release_epoch: service_spec.active_release_epoch,
                    service_run_key: service_spec.service_run_key.clone(),
                    host_generation: 1,
                },
                call_id: MiniAppBridgeCallId::from("fast-call"),
                method: "fast".into(),
                payload: StrictJsonValue(json!({})),
            },
            MiniAppCallCancellation::default(),
        ),
    )
    .await
    .expect("fast call must not wait for slow Storage");
    assert_eq!(fast.unwrap().0, json!({"fast": true}));

    slow_cancellation.cancel();
    let slow_result = tokio::time::timeout(Duration::from_secs(1), slow)
        .await
        .expect("canceled slow call must settle")
        .unwrap();
    assert!(slow_result.is_err());
    process.stop().await;
}
