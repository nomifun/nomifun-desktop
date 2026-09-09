use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, CapabilityId, CapabilityRef, DigestHex, MiniAppBridgeCallId,
    MiniAppBridgeTarget, MiniAppId,
    MiniAppServiceRuntimeFingerprint, MiniAppServiceTestCredentialMode,
    MiniAppServiceTestOutcome, MiniAppServiceTestReceipt, ResolvedMiniAppServiceSpec,
    ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId, RuntimeTarget, StrictJsonValue,
    VersionString, MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
    MINIAPP_SERVICE_SDK_CONTRACT_VERSION, MINIAPP_SERVICE_TEST_CONTRACT_VERSION, digest_bytes,
};
use nomifun_api_types::{
    BuildMiniAppRequest, CreateMiniAppProjectRequest, MiniAppKindDto,
    MiniAppServiceLifecycleDto, PublishMiniAppRequest, SetMiniAppEnabledRequest,
    TestMiniAppReleaseRequest,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_miniapp_platform::{
    InMemoryMiniAppManagedStorage, InMemoryMiniAppServiceHost, MiniAppCallCancellation,
    MiniAppAgentCapabilityInvocation, MiniAppAgentCapabilityPort,
    MiniAppM1ApplicationService, MiniAppPlatformResult, MiniAppServiceHostPort,
    MiniAppServiceInvocation, MiniAppServiceProcess, MiniAppServiceProcessError,
    MiniAppServiceProcessFactory, MiniAppServiceRuntimeBinding, MiniAppServiceSpecInput,
    MiniAppServiceStoragePort, MiniAppServiceTestRunInput,
    MiniAppServiceTestStorageResolution,
};
use serde_json::json;
use tokio::sync::Mutex;

#[derive(Default)]
struct TestProcessFactory;

struct TestProcess;

#[async_trait]
impl MiniAppServiceProcess for TestProcess {
    async fn invoke(
        &self,
        invocation: MiniAppServiceInvocation,
        cancellation: MiniAppCallCancellation,
    ) -> Result<StrictJsonValue, MiniAppServiceProcessError> {
        if cancellation.is_canceled() {
            return Err(MiniAppServiceProcessError::Rejected(
                "test invocation canceled".into(),
            ));
        }
        Ok(StrictJsonValue(json!({
            "method": invocation.method,
            "payload": invocation.payload.0,
            "call_id": invocation.call_id.0,
        })))
    }

    async fn stop(&self) {}
}

#[async_trait]
impl MiniAppServiceProcessFactory for TestProcessFactory {
    async fn start(
        &self,
        _launch: nomifun_miniapp_platform::MiniAppServiceLaunch,
    ) -> MiniAppPlatformResult<Arc<dyn MiniAppServiceProcess>> {
        Ok(Arc::new(TestProcess))
    }
}

struct TestRuntime {
    host: Arc<InMemoryMiniAppServiceHost>,
    storage: Arc<InMemoryMiniAppManagedStorage>,
    started: Mutex<Vec<MiniAppId>>,
}

impl TestRuntime {
    fn new() -> Self {
        Self {
            host: Arc::new(InMemoryMiniAppServiceHost::new(Arc::new(
                TestProcessFactory,
            ))),
            storage: Arc::new(InMemoryMiniAppManagedStorage::new()),
            started: Mutex::new(Vec::new()),
        }
    }

    fn runtime_fingerprint() -> MiniAppServiceRuntimeFingerprint {
        MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("test-runtime"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest("runtime"),
            node_version: VersionString::from("24.0.0"),
        }
    }
}

#[async_trait]
impl MiniAppServiceRuntimeBinding for TestRuntime {
    async fn resolve_spec(
        &self,
        input: MiniAppServiceSpecInput,
    ) -> MiniAppPlatformResult<ResolvedMiniAppServiceSpec> {
        input.validate()?;
        let runtime = Self::runtime_fingerprint();
        ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: input.miniapp_id,
            release: input.release,
            active_release_epoch: input.active_release_epoch,
            service_module_digest: input.descriptor.module_digest,
            lifecycle: input.descriptor.lifecycle,
            host_protocol_version: input.descriptor.host_protocol_version,
            sdk_contract_version: input.descriptor.sdk_contract_version,
            runtime,
            config_schema_digest: input.config_schema_digest,
            config_snapshot_digest: input.config_snapshot_digest,
            credential_slots_digest: input.credential_slots_digest,
            resource_contract_digest: input.resource_contract_digest,
            resource_bindings_digest: input.resource_bindings_digest,
            runtime_requirements_digest: input.descriptor.runtime_requirements_digest,
            bridge_contract_digest: input.bridge_contract_digest,
            contribution_set_digest: input.contribution_set_digest,
            storage: input.storage,
        })
        .map_err(nomifun_miniapp_platform::MiniAppPlatformError::Contract)
    }

    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> MiniAppPlatformResult<()> {
        self.host.bind_active(spec, enabled).await
    }

    async fn start(
        &self,
        spec: ResolvedMiniAppServiceSpec,
    ) -> MiniAppPlatformResult<()> {
        let miniapp_id = spec.miniapp_id.clone();
        self.host.bind_active(spec, true).await?;
        if !matches!(
            self.host.state(&miniapp_id).await,
            Some(nomifun_miniapp_platform::MiniAppServiceHostState::Running { .. })
        ) {
            self.host.retry(&miniapp_id).await?;
        }
        self.started.lock().await.push(miniapp_id);
        Ok(())
    }

    async fn invoke(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
        call_id: MiniAppBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        self.host
            .invoke(spec, call_id, method, payload, cancellation, now_ms)
            .await
    }

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId) {
        self.host.cancel(miniapp_id, call_id).await;
    }

    async fn stop(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        self.host.stop(miniapp_id).await
    }

    async fn retry(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        self.host.retry(miniapp_id).await
    }

    async fn state(
        &self,
        miniapp_id: &MiniAppId,
    ) -> Option<nomifun_miniapp_platform::MiniAppServiceHostState> {
        self.host.state(miniapp_id).await
    }

    async fn maintain(&self, now_ms: i64) -> MiniAppPlatformResult<()> {
        self.host.reap_idle(now_ms, 60_000).await?;
        self.host.reconcile_continuous(now_ms).await.map(|_| ())
    }

    async fn register_module(
        &self,
        _miniapp_id: MiniAppId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppServiceTestStorageResolution> {
        self.storage
            .create_service_test_storage(
                owner_user_id,
                miniapp_id,
                test_id,
                uses_files,
                uses_private_database,
            )
            .await
    }

    async fn purge_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
    ) -> MiniAppPlatformResult<()> {
        self.storage
            .purge_service_test_storage(owner_user_id, miniapp_id, test_id)
            .await
    }

    async fn run_service_test(
        &self,
        input: MiniAppServiceTestRunInput,
    ) -> MiniAppPlatformResult<MiniAppServiceTestReceipt> {
        let receipt = MiniAppServiceTestReceipt {
            receipt_id: input.receipt_id,
            miniapp_id: input.spec.miniapp_id.clone(),
            release: input.spec.release.clone(),
            service_run_key: input.spec.service_run_key.clone(),
            outcome: if input.requires_managed_input {
                MiniAppServiceTestOutcome::NeedsTestInput
            } else {
                MiniAppServiceTestOutcome::Passed
            },
            error_code: None,
            runtime: input.spec.runtime.clone(),
            host_target: input.spec.runtime.runtime_target.clone(),
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            test_contract_version: MINIAPP_SERVICE_TEST_CONTRACT_VERSION.into(),
            resolved_test_input_digest: input.resolved_test_input_digest,
            copied_kv_digest: input.copied_kv_digest,
            copied_private_database_digest: input.copied_private_database_digest,
            empty_files_dir: input.empty_files_dir,
            migration_ledger_digest: input.migration_ledger_digest,
            credential_mode: MiniAppServiceTestCredentialMode::None,
            host_generation: 1,
            issued_at_ms: nomifun_common::now_ms().max(1),
        };
        receipt
            .validate_for(&input.ready, &input.spec)
            .map_err(nomifun_miniapp_platform::MiniAppPlatformError::Contract)?;
        Ok(receipt)
    }

    async fn current_runtime_fingerprint(
        &self,
    ) -> MiniAppPlatformResult<Option<MiniAppServiceRuntimeFingerprint>> {
        Ok(Some(Self::runtime_fingerprint()))
    }
}

fn digest(seed: &str) -> DigestHex {
    digest_bytes(seed.as_bytes())
}

#[tokio::test]
async fn service_product_runs_the_application_surface_bridge_lifecycle() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        MiniAppM1ApplicationService::new_with_root(repository, store_root.path()).unwrap(),
    );
    let runtime = Arc::new(TestRuntime::new());
    application.install_service_runtime(runtime.clone()).await;

    let created = application
        .create(
            &owner,
            CreateMiniAppProjectRequest {
                expected_library_revision: 0,
                display_name: "Service Board".into(),
                description: Some("application service lifecycle".into()),
                kind: MiniAppKindDto::Service,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("Service Bridge failed: {error:?}"));
    assert_eq!(created.miniapp.kind, MiniAppKindDto::Service);

    let built = application
        .build(
            &owner,
            BuildMiniAppRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: created.miniapp.product_revision,
                project_id: created.project_id.clone(),
                expected_project_revision: created.project_revision,
                expected_build_generation: created.build_generation,
                expected_source_snapshot_digest: created
                    .source_snapshot_digest
                    .clone()
                    .unwrap(),
                expected_dependency_lock_digest: created
                    .dependency_lock_digest
                    .clone()
                    .unwrap(),
                service_lifecycle: Some(MiniAppServiceLifecycleDto::OnDemand),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("Service Bridge failed: {error:?}"));
    let ready = built.ready.as_ref().unwrap();
    assert_eq!(ready.kind, MiniAppKindDto::Service);
    assert_eq!(
        ready.service.as_ref().unwrap().lifecycle,
        MiniAppServiceLifecycleDto::OnDemand
    );
    assert_eq!(
        ready.test.status,
        nomifun_api_types::MiniAppTestStatusDto::NotRun
    );

    let tested = application
        .test_ready_service(
            &owner,
            TestMiniAppReleaseRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: built.miniapp.product_revision,
                expected_pointer_revision: built.miniapp.releases.pointer_revision,
                project_id: built.project_id.clone(),
                expected_project_revision: built.project_revision,
                expected_build_generation: built.build_generation,
                release_id: ready.release.release_id.clone(),
                expected_release_digest: ready.release.release_digest.clone(),
                expected_config_revision: built.config.config_revision,
                expected_credential_bindings_revision: built
                    .credential_bindings_revision,
                resolved_test_input_digest:
                    "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                        .into(),
            },
        )
        .await
        .unwrap();
    let tested_ready = tested.ready.as_ref().unwrap();
    assert_eq!(
        tested_ready.test.status,
        nomifun_api_types::MiniAppTestStatusDto::Passed
    );
    assert!(tested_ready.test.receipt_id.is_some());

    let published = application
        .publish(
            &owner,
            PublishMiniAppRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: tested.miniapp.product_revision,
                expected_pointer_revision: tested.miniapp.releases.pointer_revision,
                expected_active_release_epoch: tested.miniapp.releases.active_release_epoch,
                ready_release_id: ready.release.release_id.clone(),
                expected_ready_release_digest: ready.release.release_digest.clone(),
                expected_active_release_digest: None,
                expected_service_test_receipt_id: tested_ready.test.receipt_id.clone(),
                acknowledge_test_warning: false,
            },
        )
        .await
        .unwrap();
    let active = published.miniapp.releases.active.as_ref().unwrap();

    let enabled = application
        .set_enabled(
            &owner,
            SetMiniAppEnabledRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: published.miniapp.product_revision,
                expected_pointer_revision: published.miniapp.releases.pointer_revision,
                expected_active_release_digest: Some(active.release_digest.clone()),
                enabled: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        enabled.miniapp.service_health,
        nomifun_api_types::MiniAppServiceHealthDto::Stopped
    );
    let started = application
        .set_service_running(
            &owner,
            nomifun_api_types::SetMiniAppServiceRunningRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: enabled.miniapp.product_revision,
                expected_pointer_revision: enabled.miniapp.releases.pointer_revision,
                expected_active_release_epoch: enabled
                    .miniapp
                    .releases
                    .active_release_epoch,
                expected_active_release_digest: active.release_digest.clone(),
                running: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        started.miniapp.service_health,
        nomifun_api_types::MiniAppServiceHealthDto::Ready {
            release_id: active.release_id.clone(),
            expected_release_digest: active.release_digest.clone(),
            started_at_ms: started.miniapp.updated_at_ms,
        }
    );

    let stale_catalog = application
        .invoke_agent_capability(MiniAppAgentCapabilityInvocation {
            owner_user_id: owner.clone(),
            miniapp_id: created.miniapp.miniapp_id.clone().into(),
            capability: CapabilityRef {
                id: CapabilityId::from("miniapp.missing"),
                version: VersionString::from("1.0.0"),
            },
            action_id: ActionId::from("miniapp.missing.invoke"),
            active_release: nomifun_agent_contracts::MiniAppReleaseRef {
                release_id: active.release_id.clone().into(),
                artifact_id: active.artifact_id.clone().into(),
                release_digest: active.release_digest.clone().into(),
                manifest_digest: active.manifest_digest.clone().into(),
            },
            active_release_epoch: started.miniapp.releases.active_release_epoch,
            catalog_digest: digest("stale-catalog"),
            operation_id: "operation-stale-catalog".into(),
            call_id: MiniAppBridgeCallId::from("agent-call-stale-catalog"),
            payload: StrictJsonValue(json!({})),
        })
        .await
        .unwrap_err();
    assert!(stale_catalog
        .to_string()
        .contains("stale Catalog digest"));

    let surface = application
        .open_surface(&owner, &created.miniapp.miniapp_id)
        .await
        .unwrap();
    assert_eq!(surface.kind, MiniAppKindDto::Service);
    let result = application
        .surface_bridge_request(
            &owner,
            &created.miniapp.miniapp_id,
            &surface.surface_capability,
            surface.active_release_epoch,
            &surface.expected_release_digest,
            nomifun_agent_contracts::MiniAppBridgeRequest {
                call_id: MiniAppBridgeCallId::from("service-call-1"),
                target: MiniAppBridgeTarget::Service {
                    method: "echo".into(),
                    payload: StrictJsonValue(json!({"value": 7})),
                },
            },
        )
        .await
        .unwrap_or_else(|error| panic!("Service Bridge failed: {error:?}"));
    assert_eq!(result.0["payload"], json!({"value": 7}));
    assert_eq!(result.0["method"], "echo");

    let stopped = application
        .set_service_running(
            &owner,
            nomifun_api_types::SetMiniAppServiceRunningRequest {
                miniapp_id: created.miniapp.miniapp_id.clone(),
                expected_product_revision: started.miniapp.product_revision,
                expected_pointer_revision: started.miniapp.releases.pointer_revision,
                expected_active_release_epoch: started
                    .miniapp
                    .releases
                    .active_release_epoch,
                expected_active_release_digest: active.release_digest.clone(),
                running: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        stopped.miniapp.service_health,
        nomifun_api_types::MiniAppServiceHealthDto::Stopped
    );
    assert!(!runtime.started.lock().await.is_empty());
}
