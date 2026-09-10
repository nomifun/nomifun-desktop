use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ArtifactId, DigestHex, MiniAppAdditiveMigrationAction, MiniAppBridgeCallId,
    MiniAppBridgeKvRequest, MiniAppBridgeRequest, MiniAppBridgeTarget, MiniAppDatabaseHandleId,
    MiniAppFilesDirDescriptor, MiniAppFilesHandleId, MiniAppId, MiniAppKvHandleDescriptor,
    MiniAppKvHandleId, MiniAppMigration, MiniAppMigrationColumn, MiniAppMigrationId,
    MiniAppPrivateDatabaseDescriptor, MiniAppReleaseId, MiniAppReleasePointerState,
    MiniAppReleaseRef, MiniAppServiceLifecycle, MiniAppServiceRuntimeFingerprint,
    MiniAppServiceStorageDescriptor, MiniAppSurfaceSessionId, ResolvedMiniAppServiceSpec,
    ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId, RuntimeTarget, StrictJsonValue,
    VersionString, digest_payload,
};
use tokio::sync::Mutex;

use crate::{
    InMemoryMiniAppBridgeHost, InMemoryMiniAppManagedStorage, InMemoryMiniAppServiceHost,
    MiniAppBridgeBinding, MiniAppBridgeHost, MiniAppCallCancellation, MiniAppDatabaseStatement,
    MiniAppFilesPort, MiniAppMigrationLedger, MiniAppPlatformError, MiniAppPlatformResult,
    MiniAppPrivateDatabasePort, MiniAppServiceGenerationFence, MiniAppServiceHostPort,
    MiniAppServiceHostState, MiniAppServiceInvocation, MiniAppServiceLaunch,
    MiniAppServiceProcess, MiniAppServiceProcessError, MiniAppServiceProcessFactory,
    MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS, MINIAPP_CONTINUOUS_CRASH_FAILURE_THRESHOLD,
};
use crate::storage::next_kv_revision;

#[derive(Default)]
struct TestProcess {
    block: AtomicBool,
    entered: AtomicBool,
    released: AtomicBool,
    stopped: AtomicBool,
    crashed: AtomicBool,
}

impl TestProcess {
    fn blocking() -> Self {
        Self {
            block: AtomicBool::new(true),
            ..Self::default()
        }
    }

    async fn wait_until_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
    }

    fn crash(&self) {
        self.crashed.store(true, Ordering::Release);
    }
}

#[async_trait]
impl MiniAppServiceProcess for TestProcess {
    async fn invoke(
        &self,
        invocation: MiniAppServiceInvocation,
        _cancellation: MiniAppCallCancellation,
    ) -> Result<StrictJsonValue, MiniAppServiceProcessError> {
        self.entered.store(true, Ordering::Release);
        while self.block.load(Ordering::Acquire) && !self.released.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
        Ok(invocation.payload)
    }

    async fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn terminal_result(&self) -> Option<Result<(), String>> {
        self.crashed
            .load(Ordering::Acquire)
            .then(|| Err("simulated passive process exit".into()))
    }
}

#[derive(Default)]
struct TestProcessFactory {
    blocking_next: AtomicBool,
    fail_next_start: AtomicBool,
    block_next_start: AtomicBool,
    start_entered: AtomicBool,
    release_start: AtomicBool,
    processes: Mutex<BTreeMap<MiniAppId, Vec<Arc<TestProcess>>>>,
}

impl TestProcessFactory {
    fn block_next(&self) {
        self.blocking_next.store(true, Ordering::Release);
    }

    fn fail_next_start(&self) {
        self.fail_next_start.store(true, Ordering::Release);
    }

    fn block_next_start(&self) {
        self.start_entered.store(false, Ordering::Release);
        self.release_start.store(false, Ordering::Release);
        self.block_next_start.store(true, Ordering::Release);
    }

    async fn wait_until_starting(&self) {
        while !self.start_entered.load(Ordering::Acquire) {
            tokio::task::yield_now().await;
        }
    }

    fn release_start(&self) {
        self.release_start.store(true, Ordering::Release);
    }

    async fn latest(&self, miniapp_id: &MiniAppId) -> Arc<TestProcess> {
        self.processes
            .lock()
            .await
            .get(miniapp_id)
            .and_then(|processes| processes.last())
            .cloned()
            .expect("process was started")
    }

    async fn start_count(&self, miniapp_id: &MiniAppId) -> usize {
        self.processes
            .lock()
            .await
            .get(miniapp_id)
            .map_or(0, Vec::len)
    }
}

#[async_trait]
impl MiniAppServiceProcessFactory for TestProcessFactory {
    async fn start(
        &self,
        launch: MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<Arc<dyn MiniAppServiceProcess>> {
        if self.block_next_start.swap(false, Ordering::AcqRel) {
            self.start_entered.store(true, Ordering::Release);
            while !self.release_start.load(Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        }
        if self.fail_next_start.swap(false, Ordering::AcqRel) {
            return Err(MiniAppPlatformError::Runtime(
                "simulated Service start failure".into(),
            ));
        }
        let process = Arc::new(if self.blocking_next.swap(false, Ordering::AcqRel) {
            TestProcess::blocking()
        } else {
            TestProcess::default()
        });
        self.processes
            .lock()
            .await
            .entry(launch.spec.miniapp_id)
            .or_default()
            .push(process.clone());
        Ok(process)
    }
}

#[tokio::test]
async fn service_host_enforces_lifecycle_idle_reap_and_crash_isolation() {
    let factory = Arc::new(TestProcessFactory::default());
    let host = InMemoryMiniAppServiceHost::new(factory.clone());
    let continuous = service_spec("continuous", 1, MiniAppServiceLifecycle::Continuous);
    let on_demand = service_spec("demand", 1, MiniAppServiceLifecycle::OnDemand);

    host.bind_active(continuous.clone(), true).await.unwrap();
    host.bind_active(on_demand.clone(), true).await.unwrap();
    assert_eq!(factory.start_count(&continuous.miniapp_id).await, 1);
    assert_eq!(factory.start_count(&on_demand.miniapp_id).await, 0);

    let payload = json_object("value", 1);
    assert_eq!(
        host.invoke(
            &on_demand,
            MiniAppBridgeCallId::from("call-demand"),
            "read".into(),
            payload.clone(),
            MiniAppCallCancellation::default(),
            100,
        )
        .await
        .unwrap(),
        payload
    );
    assert_eq!(factory.start_count(&on_demand.miniapp_id).await, 1);
    assert_eq!(
        host.reap_idle(149, 50).await.unwrap(),
        Vec::<MiniAppId>::new()
    );
    assert_eq!(
        host.reap_idle(150, 50).await.unwrap(),
        vec![on_demand.miniapp_id.clone()]
    );
    assert_eq!(
        host.state(&on_demand.miniapp_id).await,
        Some(MiniAppServiceHostState::Stopped)
    );

    let continuous_fence = running_fence(&host, &continuous.miniapp_id).await;
    assert!(
        host.report_crash(&continuous_fence, "process exited".into(), 200)
            .await
            .unwrap()
    );
    assert!(matches!(
        host.state(&continuous.miniapp_id).await,
        Some(MiniAppServiceHostState::Backoff { .. })
    ));
    assert_eq!(
        host.state(&on_demand.miniapp_id).await,
        Some(MiniAppServiceHostState::Stopped),
        "one MiniApp crash must not mutate another dedicated Host"
    );
}

#[tokio::test]
async fn maintenance_observes_a_passive_on_demand_process_exit() {
    let factory = Arc::new(TestProcessFactory::default());
    let host = InMemoryMiniAppServiceHost::new(factory.clone());
    let service = service_spec("passive-exit", 1, MiniAppServiceLifecycle::OnDemand);
    host.bind_active(service.clone(), true).await.unwrap();
    host.invoke(
        &service,
        MiniAppBridgeCallId::from("start-before-passive-exit"),
        "start".into(),
        json_object("ok", 1),
        MiniAppCallCancellation::default(),
        100,
    )
    .await
    .unwrap();
    factory.latest(&service.miniapp_id).await.crash();

    assert_eq!(
        host.observe_process_exits(101).await,
        vec![service.miniapp_id.clone()]
    );
    assert!(matches!(
        host.state(&service.miniapp_id).await,
        Some(MiniAppServiceHostState::Error { error, .. })
            if error == "simulated passive process exit"
    ));
    assert!(host.capacity_snapshot().await.active_miniapps.is_empty());
}

#[tokio::test]
async fn service_capacity_counts_starting_and_running_hosts_without_eviction() {
    assert!(matches!(
        InMemoryMiniAppServiceHost::with_capacity(
            Arc::new(TestProcessFactory::default()),
            0
        ),
        Err(MiniAppPlatformError::InvalidState(_))
    ));

    let factory = Arc::new(TestProcessFactory::default());
    factory.block_next_start();
    let host = Arc::new(InMemoryMiniAppServiceHost::with_capacity(factory.clone(), 2).unwrap());
    let first = service_spec("capacity-a", 1, MiniAppServiceLifecycle::Continuous);
    let second = service_spec("capacity-b", 1, MiniAppServiceLifecycle::Continuous);
    let third = service_spec("capacity-c", 1, MiniAppServiceLifecycle::Continuous);

    let starting_host = host.clone();
    let starting_spec = first.clone();
    let starting = tokio::spawn(async move {
        starting_host.bind_active(starting_spec, true).await
    });
    factory.wait_until_starting().await;
    assert_eq!(
        host.capacity_snapshot().await.active_miniapps,
        vec![first.miniapp_id.clone()],
        "starting Hosts consume capacity"
    );
    factory.release_start();
    starting.await.unwrap().unwrap();
    host.bind_active(second.clone(), true).await.unwrap();

    let lowered = host.update_capacity(1).await.unwrap();
    assert_eq!(lowered.max_active_service_hosts, 1);
    assert_eq!(
        lowered.active_miniapps,
        vec![first.miniapp_id.clone(), second.miniapp_id.clone()],
        "lowering the limit must not kill existing Hosts"
    );
    let exhausted = host.bind_active(third.clone(), true).await;
    assert!(matches!(
        exhausted,
        Err(MiniAppPlatformError::ServiceCapacityExhausted {
            max_active: 1,
            active_miniapps,
        }) if active_miniapps
            == vec![
                first.miniapp_id.as_ref().to_owned(),
                second.miniapp_id.as_ref().to_owned(),
            ]
    ));

    host.stop(&first.miniapp_id).await.unwrap();
    assert!(matches!(
        host.bind_active(third.clone(), true).await,
        Err(MiniAppPlatformError::ServiceCapacityExhausted { .. })
    ));
    host.stop(&second.miniapp_id).await.unwrap();

    factory.fail_next_start();
    assert!(matches!(
        host.bind_active(third.clone(), true).await,
        Err(MiniAppPlatformError::Runtime(_))
    ));
    assert!(
        host.capacity_snapshot().await.active_miniapps.is_empty(),
        "start failure releases its exact reservation"
    );
    host.retry(&third.miniapp_id).await.unwrap();
    let third_fence = running_fence(&host, &third.miniapp_id).await;
    host.report_crash(&third_fence, "capacity crash".into(), 100)
        .await
        .unwrap();
    assert!(
        host.capacity_snapshot().await.active_miniapps.is_empty(),
        "crash releases its exact running reservation"
    );
}

#[tokio::test]
async fn continuous_service_uses_finite_backoff_threshold_and_manual_retry_reset() {
    let factory = Arc::new(TestProcessFactory::default());
    let host = InMemoryMiniAppServiceHost::new(factory.clone());
    let continuous = service_spec("backoff", 1, MiniAppServiceLifecycle::Continuous);
    host.bind_active(continuous.clone(), true).await.unwrap();

    let first_fence = running_fence(&host, &continuous.miniapp_id).await;
    host.report_crash(&first_fence, "crash-1".into(), 100)
        .await
        .unwrap();
    assert_eq!(
        host.state(&continuous.miniapp_id).await,
        Some(MiniAppServiceHostState::Backoff {
            host_generation: first_fence.host_generation,
            consecutive_failures: 1,
            retry_at_ms: 100 + MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[0],
            error: "crash-1".into(),
        })
    );
    assert!(
        host.reconcile_continuous(
            100 + MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[0] - 1
        )
        .await
        .unwrap()
        .restarted
        .is_empty()
    );
    assert_eq!(
        host.reconcile_continuous(100 + MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[0])
            .await
            .unwrap()
            .restarted,
        vec![continuous.miniapp_id.clone()]
    );

    let second_fence = running_fence(&host, &continuous.miniapp_id).await;
    host.report_crash(&second_fence, "crash-2".into(), 2_000)
        .await
        .unwrap();
    let second_retry_at = 2_000 + MINIAPP_CONTINUOUS_CRASH_BACKOFF_MS[1];
    assert_eq!(
        host.reconcile_continuous(second_retry_at)
            .await
            .unwrap()
            .restarted,
        vec![continuous.miniapp_id.clone()]
    );

    let third_fence = running_fence(&host, &continuous.miniapp_id).await;
    host.report_crash(&third_fence, "crash-3".into(), 8_000)
        .await
        .unwrap();
    assert!(matches!(
        host.state(&continuous.miniapp_id).await,
        Some(MiniAppServiceHostState::Error {
            consecutive_failures: MINIAPP_CONTINUOUS_CRASH_FAILURE_THRESHOLD,
            ..
        })
    ));
    assert!(
        host.reconcile_continuous(i64::MAX)
            .await
            .unwrap()
            .restarted
            .is_empty(),
        "threshold Error waits for explicit Retry"
    );

    host.retry(&continuous.miniapp_id).await.unwrap();
    let retried_fence = running_fence(&host, &continuous.miniapp_id).await;
    host.report_crash(&retried_fence, "after-manual-retry".into(), 9_000)
        .await
        .unwrap();
    assert!(matches!(
        host.state(&continuous.miniapp_id).await,
        Some(MiniAppServiceHostState::Backoff {
            consecutive_failures: 1,
            ..
        })
    ));

    let on_demand = service_spec("no-auto-restart", 1, MiniAppServiceLifecycle::OnDemand);
    host.bind_active(on_demand.clone(), true).await.unwrap();
    host.invoke(
        &on_demand,
        MiniAppBridgeCallId::from("start-on-demand"),
        "start".into(),
        json_object("ok", 1),
        MiniAppCallCancellation::default(),
        10_000,
    )
    .await
    .unwrap();
    let on_demand_fence = running_fence(&host, &on_demand.miniapp_id).await;
    host.report_crash(&on_demand_fence, "on-demand-crash".into(), 10_001)
        .await
        .unwrap();
    assert!(matches!(
        host.state(&on_demand.miniapp_id).await,
        Some(MiniAppServiceHostState::Error { .. })
    ));
    assert!(
        !host
            .reconcile_continuous(i64::MAX)
            .await
            .unwrap()
            .restarted
            .contains(&on_demand.miniapp_id)
    );
}

#[tokio::test]
async fn service_host_rejects_late_callback_after_release_generation_changes() {
    let factory = Arc::new(TestProcessFactory::default());
    factory.block_next();
    let host = Arc::new(InMemoryMiniAppServiceHost::new(factory.clone()));
    let first = service_spec("late", 1, MiniAppServiceLifecycle::OnDemand);
    let second = service_spec("late", 2, MiniAppServiceLifecycle::OnDemand);
    host.bind_active(first.clone(), true).await.unwrap();

    let invoking_host = host.clone();
    let invoking_spec = first.clone();
    let task = tokio::spawn(async move {
        invoking_host
            .invoke(
                &invoking_spec,
                MiniAppBridgeCallId::from("call-late"),
                "slow".into(),
                json_object("generation", 1),
                MiniAppCallCancellation::default(),
                100,
            )
            .await
    });
    let old_process = loop {
        if factory.start_count(&first.miniapp_id).await > 0 {
            break factory.latest(&first.miniapp_id).await;
        }
        tokio::task::yield_now().await;
    };
    old_process.wait_until_entered().await;
    host.bind_active(second, true).await.unwrap();
    old_process.release();

    assert!(matches!(
        task.await.unwrap(),
        Err(MiniAppPlatformError::StaleServiceGeneration)
    ));
    assert!(old_process.stopped.load(Ordering::Acquire));
}

#[tokio::test]
async fn bridge_binds_owner_release_epoch_surface_and_rejects_old_ports() {
    let storage = Arc::new(InMemoryMiniAppManagedStorage::new());
    let factory = Arc::new(TestProcessFactory::default());
    let service = Arc::new(InMemoryMiniAppServiceHost::new(factory));
    let bridge = InMemoryMiniAppBridgeHost::new(storage.clone(), service);
    let owner = MiniAppId::from("bridge-owner");
    let first_storage = ui_storage(&owner, "bridge-owner");
    storage.register(first_storage.clone(), None).await.unwrap();

    let mut exact_pointer = pointer(&owner, 1);
    exact_pointer.pointer_revision = 41;
    exact_pointer.materialized_catalog_digest = digest("exact-bridge-catalog");
    let first = bridge
        .open(MiniAppBridgeBinding {
            miniapp_id: owner.clone(),
            surface_session_id: MiniAppSurfaceSessionId::from("surface-1"),
            active: exact_pointer.clone(),
            storage: first_storage.clone(),
            service_spec: None,
        })
        .await
        .unwrap();
    assert_eq!(
        bridge.pointer_snapshot(&first).await.unwrap(),
        exact_pointer,
        "Bridge must retain the exact committed pointer, not synthesize one"
    );
    bridge
        .request(
            &first,
            MiniAppBridgeRequest {
                call_id: MiniAppBridgeCallId::from("kv-set"),
                target: MiniAppBridgeTarget::HostKv {
                    request: MiniAppBridgeKvRequest::Set {
                        key: "theme".into(),
                        value: StrictJsonValue(serde_json::json!("dark")),
                    },
                },
            },
            MiniAppCallCancellation::default(),
            10,
        )
        .await
        .unwrap();

    let second = bridge
        .open(MiniAppBridgeBinding {
            miniapp_id: owner.clone(),
            surface_session_id: MiniAppSurfaceSessionId::from("surface-2"),
            active: pointer(&owner, 2),
            storage: first_storage,
            service_spec: None,
        })
        .await
        .unwrap();
    assert!(first.is_closed(), "new Active epoch closes the old port");
    assert!(!second.is_closed());
    assert!(matches!(
        bridge
            .request(
                &first,
                MiniAppBridgeRequest {
                    call_id: MiniAppBridgeCallId::from("late-kv"),
                    target: MiniAppBridgeTarget::HostKv {
                        request: MiniAppBridgeKvRequest::Get {
                            key: "theme".into(),
                        },
                    },
                },
                MiniAppCallCancellation::default(),
                20,
            )
            .await,
        Err(MiniAppPlatformError::StaleBridgePort)
    ));
}

#[tokio::test]
async fn ui_only_bridge_rejects_service_only_storage_handles() {
    let storage = Arc::new(InMemoryMiniAppManagedStorage::new());
    let service = Arc::new(InMemoryMiniAppServiceHost::new(Arc::new(
        TestProcessFactory::default(),
    )));
    let bridge = InMemoryMiniAppBridgeHost::new(storage, service);
    let owner = MiniAppId::from("ui-storage-rejection");
    let (service_storage, _) = service_storage(&owner, "ui-storage-rejection");
    assert!(matches!(
        bridge
            .open(MiniAppBridgeBinding {
                miniapp_id: owner.clone(),
                surface_session_id: MiniAppSurfaceSessionId::from("surface-ui-only"),
                active: pointer(&owner, 1),
                storage: service_storage,
                service_spec: None,
            })
            .await,
        Err(MiniAppPlatformError::InvalidState(_))
    ));
}

#[test]
fn kv_revision_increment_is_checked_and_reports_overflow() {
    assert_eq!(next_kv_revision(None).unwrap(), 1);
    assert_eq!(next_kv_revision(Some(41)).unwrap(), 42);
    assert!(matches!(
        next_kv_revision(Some(u64::MAX)),
        Err(MiniAppPlatformError::KvRevisionOverflow)
    ));
}

#[tokio::test]
async fn managed_storage_rejects_foreign_handles_and_keeps_kv_cas_owner_scoped() {
    let storage = InMemoryMiniAppManagedStorage::new();
    let owner = MiniAppId::from("storage-owner");
    let foreign = MiniAppId::from("storage-foreign");
    let descriptor = service_storage(&owner, "storage-owner").0;
    storage
        .register(descriptor.clone(), service_storage(&owner, "storage-owner").1)
        .await
        .unwrap();
    let files_handle = descriptor
        .files_dir
        .as_ref()
        .expect("Service files")
        .handle_id
        .clone();

    assert!(matches!(
        storage.resolve(&foreign, &files_handle).await,
        Err(MiniAppPlatformError::UnknownStorageHandle)
    ));

    let foreign_collision = MiniAppServiceStorageDescriptor {
        kv: MiniAppKvHandleDescriptor {
            handle_id: descriptor.kv.handle_id.clone(),
            miniapp_id: foreign,
            namespace_revision: 1,
        },
        files_dir: None,
        private_database: None,
    };
    assert!(matches!(
        storage.register(foreign_collision, None).await,
        Err(MiniAppPlatformError::UnknownStorageHandle)
    ));
}

#[tokio::test]
async fn database_cancellation_only_wins_before_the_commit_boundary() {
    let storage = InMemoryMiniAppManagedStorage::new();
    let owner = MiniAppId::from("database-cancel-owner");
    let (descriptor, ledger) = service_storage(&owner, "database-cancel-owner");
    let database_handle = descriptor
        .private_database
        .as_ref()
        .expect("Service database")
        .handle_id
        .clone();
    storage.register(descriptor, ledger).await.unwrap();

    let canceled = MiniAppCallCancellation::default();
    canceled.cancel();
    let query = MiniAppDatabaseStatement {
        sql: "SELECT value FROM notes WHERE id = ?".into(),
        parameters: StrictJsonValue(serde_json::json!(["1"])),
    };
    let execute = MiniAppDatabaseStatement {
        sql: "INSERT INTO notes (id) VALUES (?)".into(),
        parameters: StrictJsonValue(serde_json::json!(["1"])),
    };
    assert!(matches!(
        storage
            .query(&owner, &database_handle, query, canceled.clone())
            .await,
        Err(MiniAppPlatformError::Canceled)
    ));
    assert!(matches!(
        storage
            .execute(&owner, &database_handle, execute.clone(), canceled.clone())
            .await,
        Err(MiniAppPlatformError::Canceled)
    ));
    assert!(matches!(
        storage
            .batch(
                &owner,
                &database_handle,
                vec![execute.clone()],
                canceled,
            )
            .await,
        Err(MiniAppPlatformError::Canceled)
    ));
    assert!(
        storage
            .recorded_statements(&owner, &database_handle)
            .await
            .unwrap()
            .is_empty()
    );

    let commits_then_cancels = MiniAppCallCancellation::default();
    crate::storage::test_database_commit_boundary(&commits_then_cancels).unwrap();
    assert!(commits_then_cancels.is_canceled());

    let committed = MiniAppCallCancellation::default();
    storage
        .execute(&owner, &database_handle, execute.clone(), committed.clone())
        .await
        .unwrap();
    committed.cancel();
    storage
        .query(
            &owner,
            &database_handle,
            MiniAppDatabaseStatement {
                sql: "SELECT value FROM notes".into(),
                parameters: StrictJsonValue(serde_json::json!([])),
            },
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    storage
        .batch(
            &owner,
            &database_handle,
            vec![execute.clone(), execute],
            MiniAppCallCancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .recorded_statements(&owner, &database_handle)
            .await
            .unwrap()
            .len(),
        4,
        "query/execute/batch effects remain committed after their pre-commit check"
    );
}

#[tokio::test]
async fn private_database_contract_rejects_runtime_ddl_and_appends_migration_ledger_by_cas() {
    let storage = InMemoryMiniAppManagedStorage::new();
    let owner = MiniAppId::from("database-owner");
    let (descriptor, ledger) = service_storage(&owner, "database-owner");
    let ledger = ledger.expect("Service database ledger");
    let database_handle = descriptor
        .private_database
        .as_ref()
        .expect("Service database")
        .handle_id
        .clone();
    storage
        .register(descriptor, Some(ledger.clone()))
        .await
        .unwrap();

    let ddl = MiniAppDatabaseStatement {
        sql: "CREATE TABLE notes (id TEXT)".into(),
        parameters: StrictJsonValue(serde_json::json!([])),
    };
    assert!(matches!(
        storage
            .execute(
                &owner,
                &database_handle,
                ddl,
                MiniAppCallCancellation::default(),
            )
            .await,
        Err(MiniAppPlatformError::InvalidDatabaseRequest(_))
    ));

    let migration = MiniAppMigration::new(
        MiniAppMigrationId::from("001_create_notes"),
        vec![MiniAppAdditiveMigrationAction::CreateTable {
            table_name: "notes".into(),
            columns: vec![MiniAppMigrationColumn {
                name: "id".into(),
                declared_type: "TEXT".into(),
                nullable: false,
                default_literal: None,
            }],
            primary_key_columns: vec!["id".into()],
        }],
    )
    .unwrap();
    let release = release_ref("database-owner", 1);
    let next = storage
        .apply_additive_migrations(
            &owner,
            &database_handle,
            &ledger.ledger_digest,
            &release,
            std::slice::from_ref(&migration),
            100,
        )
        .await
        .unwrap();
    assert_eq!(next.schema_epoch, ledger.schema_epoch + 1);
    assert_eq!(next.entries.len(), 1);
    assert!(matches!(
        storage
            .apply_additive_migrations(
                &owner,
                &database_handle,
                &ledger.ledger_digest,
                &release,
                &[migration],
                101,
            )
            .await,
        Err(MiniAppPlatformError::StorageConflict)
    ));
}

async fn running_fence(
    host: &InMemoryMiniAppServiceHost,
    miniapp_id: &MiniAppId,
) -> MiniAppServiceGenerationFence {
    match host.state(miniapp_id).await.expect("Host state") {
        MiniAppServiceHostState::Running { fence } => fence,
        state => panic!("expected running Host, got {state:?}"),
    }
}

fn pointer(miniapp_id: &MiniAppId, epoch: u64) -> MiniAppReleasePointerState {
    MiniAppReleasePointerState {
        miniapp_id: miniapp_id.clone(),
        pointer_revision: epoch,
        active_release_epoch: epoch,
        ready_release: None,
        active_release: Some(release_ref(miniapp_id.as_ref(), epoch)),
        previous_release: None,
        materialized_catalog_digest: digest(&format!("catalog-{epoch}")),
    }
}

fn service_spec(
    suffix: &str,
    epoch: u64,
    lifecycle: MiniAppServiceLifecycle,
) -> ResolvedMiniAppServiceSpec {
    let miniapp_id = MiniAppId::from(format!("miniapp-{suffix}"));
    let (storage, _) = service_storage(&miniapp_id, suffix);
    ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
        miniapp_id: miniapp_id.clone(),
        release: release_ref(suffix, epoch),
        active_release_epoch: epoch,
        service_module_digest: digest(&format!("service-module-{suffix}-{epoch}")),
        lifecycle,
        host_protocol_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
        sdk_contract_version:
            nomifun_agent_contracts::MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
        runtime: MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("runtime-test"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest("runtime-executable"),
            node_version: VersionString::from("24.1.0"),
        },
        config_schema_digest: digest("config-schema"),
        config_snapshot_digest: digest("config-snapshot"),
        credential_slots_digest: digest("credential-slots"),
        resource_contract_digest: digest("resource-contract"),
        resource_bindings_digest: digest("resource-bindings"),
        runtime_requirements_digest: digest("runtime-requirements"),
        bridge_contract_digest: digest("bridge-contract"),
        contribution_set_digest: digest("contribution-set"),
        storage,
    })
    .unwrap()
}

fn ui_storage(miniapp_id: &MiniAppId, suffix: &str) -> MiniAppServiceStorageDescriptor {
    MiniAppServiceStorageDescriptor {
        kv: MiniAppKvHandleDescriptor {
            handle_id: MiniAppKvHandleId::from(format!("kv-{suffix}")),
            miniapp_id: miniapp_id.clone(),
            namespace_revision: 1,
        },
        files_dir: None,
        private_database: None,
    }
}

fn service_storage(
    miniapp_id: &MiniAppId,
    suffix: &str,
) -> (
    MiniAppServiceStorageDescriptor,
    Option<MiniAppMigrationLedger>,
) {
    let database_handle = MiniAppDatabaseHandleId::from(format!("db-{suffix}"));
    let ledger =
        MiniAppMigrationLedger::empty(miniapp_id.clone(), database_handle.clone(), 1).unwrap();
    (
        MiniAppServiceStorageDescriptor {
            kv: MiniAppKvHandleDescriptor {
                handle_id: MiniAppKvHandleId::from(format!("kv-{suffix}")),
                miniapp_id: miniapp_id.clone(),
                namespace_revision: 1,
            },
            files_dir: Some(MiniAppFilesDirDescriptor {
                handle_id: MiniAppFilesHandleId::from(format!("files-{suffix}")),
                miniapp_id: miniapp_id.clone(),
                absolute_path: format!("C:\\NomiFun\\miniapps\\{suffix}\\files"),
            }),
            private_database: Some(MiniAppPrivateDatabaseDescriptor {
                handle_id: database_handle,
                miniapp_id: miniapp_id.clone(),
                schema_epoch: ledger.schema_epoch,
                migration_ledger_digest: ledger.ledger_digest.clone(),
            }),
        },
        Some(ledger),
    )
}

fn release_ref(suffix: &str, epoch: u64) -> MiniAppReleaseRef {
    MiniAppReleaseRef {
        release_id: MiniAppReleaseId::from(format!("release-{suffix}-{epoch}")),
        artifact_id: ArtifactId::from(format!("artifact-{suffix}-{epoch}")),
        release_digest: digest(&format!("release-digest-{suffix}-{epoch}")),
        manifest_digest: digest(&format!("manifest-digest-{suffix}-{epoch}")),
    }
}

fn digest(seed: &str) -> DigestHex {
    digest_payload(&seed).unwrap()
}

fn json_object(key: &str, value: i64) -> StrictJsonValue {
    StrictJsonValue(serde_json::json!({ key: value }))
}
