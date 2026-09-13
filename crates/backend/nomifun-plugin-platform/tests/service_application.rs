use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::Arc,
};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    capability_surface_declarations, digest_payload, ActionId, CapabilityActionDescriptor,
    CapabilityContributions, CapabilityConsumer, CapabilityId, CapabilityKind, CapabilityManifest,
    CapabilityRef, CanonicalSchemaRef, DigestHex, EffectClass, LocalizedMetadata,
    MiniAppBridgeCallId, MiniAppBridgeTarget, MiniAppId, MiniAppReleaseRef,
    MiniAppResourceContract, MiniAppServiceLifecycle, MiniAppShareBundleId,
    PackageContributions, PackageId, PackageRef, PlatformConstraint,
    MiniAppServiceRuntimeFingerprint, MiniAppServiceTestCredentialMode,
    MiniAppServiceTestOutcome, MiniAppServiceTestReceipt, ResolvedMiniAppServiceSpec,
    ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId, RuntimeTarget, StrictJsonValue,
    ToolPresentationKind, VersionString, MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
    MINIAPP_SERVICE_SDK_CONTRACT_VERSION, MINIAPP_SERVICE_TEST_CONTRACT_VERSION, digest_bytes,
};
use nomifun_api_types::{
    BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest, ImportPluginRuntimeArtifactRequest, PluginRuntimeKindDto,
    PluginRuntimeServiceLifecycleDto, PublishPluginRuntimeRequest, SetPluginRuntimeEnabledRequest,
    TestPluginRuntimeReleaseRequest,
};
use nomifun_db::{
    IMiniAppM1Repository, SqliteMiniAppM1Repository, init_database_memory,
    installation_owner_id,
};
use nomifun_plugin_platform::runtime::{
    InMemoryPluginRuntimeManagedStorage, InMemoryPluginRuntimeServiceHost, PluginRuntimeCallCancellation,
    PluginRuntimeAgentCapabilityInvocation, PluginRuntimeAgentCapabilityPort,
    PluginRuntimeM1ApplicationService, PluginRuntimePlatformResult, PluginRuntimeServiceHostPort,
    PluginRuntimeServiceInvocation, PluginRuntimeServiceProcess, PluginRuntimeServiceProcessError,
    PluginRuntimeServiceProcessFactory, PluginRuntimeServiceRuntimeBinding, PluginRuntimeServiceSpecInput,
    PluginRuntimeServiceStoragePort, PluginRuntimeServiceTestRunInput,
    PluginRuntimeServiceTestStorageResolution, PluginRuntimeReleaseFileBytes, PluginRuntimeReleasePublishRequest,
    PluginRuntimeReleaseStore, PluginRuntimeShareBundleExport, PluginRuntimeShareBundleFilesystem,
    PluginRuntimeSourceScope, PluginRuntimeStaticBundleBuilder,
    PluginRuntimeStaticBundleFile, PluginRuntimeStaticBundleInput, PluginRuntimeStaticServiceInput,
    materialize_surface_entrypoint,
};
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

const CALLABLE_CAPABILITY_ID: &str = "miniapp.callable.echo";
const CALLABLE_ACTION_ID: &str = "miniapp.callable.echo.invoke";

#[derive(Default)]
struct TestProcessFactory;

struct TestProcess;

#[async_trait]
impl PluginRuntimeServiceProcess for TestProcess {
    async fn invoke(
        &self,
        invocation: PluginRuntimeServiceInvocation,
        cancellation: PluginRuntimeCallCancellation,
    ) -> Result<StrictJsonValue, PluginRuntimeServiceProcessError> {
        if cancellation.is_canceled() {
            return Err(PluginRuntimeServiceProcessError::Rejected(
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
impl PluginRuntimeServiceProcessFactory for TestProcessFactory {
    async fn start(
        &self,
        _launch: nomifun_plugin_platform::runtime::PluginRuntimeServiceLaunch,
    ) -> PluginRuntimePlatformResult<Arc<dyn PluginRuntimeServiceProcess>> {
        Ok(Arc::new(TestProcess))
    }
}

struct TestRuntime {
    host: Arc<InMemoryPluginRuntimeServiceHost>,
    storage: Arc<InMemoryPluginRuntimeManagedStorage>,
    started: Mutex<Vec<MiniAppId>>,
}

impl TestRuntime {
    fn new() -> Self {
        Self {
            host: Arc::new(InMemoryPluginRuntimeServiceHost::new(Arc::new(
                TestProcessFactory,
            ))),
            storage: Arc::new(InMemoryPluginRuntimeManagedStorage::new()),
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
impl PluginRuntimeServiceRuntimeBinding for TestRuntime {
    async fn resolve_spec(
        &self,
        input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedMiniAppServiceSpec> {
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
        .map_err(nomifun_plugin_platform::runtime::PluginRuntimePlatformError::Contract)
    }

    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        self.host.bind_active(spec, enabled).await
    }

    async fn start(
        &self,
        spec: ResolvedMiniAppServiceSpec,
    ) -> PluginRuntimePlatformResult<()> {
        let miniapp_id = spec.miniapp_id.clone();
        self.host.bind_active(spec, true).await?;
        if !matches!(
            self.host.state(&miniapp_id).await,
            Some(nomifun_plugin_platform::runtime::PluginRuntimeServiceHostState::Running { .. })
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
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        self.host
            .invoke(spec, call_id, method, payload, cancellation, now_ms)
            .await
    }

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId) {
        self.host.cancel(miniapp_id, call_id).await;
    }

    async fn stop(&self, miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<()> {
        self.host.stop(miniapp_id).await
    }

    async fn retry(&self, miniapp_id: &MiniAppId) -> PluginRuntimePlatformResult<()> {
        self.host.retry(miniapp_id).await
    }

    async fn state(
        &self,
        miniapp_id: &MiniAppId,
    ) -> Option<nomifun_plugin_platform::runtime::PluginRuntimeServiceHostState> {
        self.host.state(miniapp_id).await
    }

    async fn maintain(&self, now_ms: i64) -> PluginRuntimePlatformResult<()> {
        self.host.reap_idle(now_ms, 60_000).await?;
        self.host.reconcile_continuous(now_ms).await.map(|_| ())
    }

    async fn register_module(
        &self,
        _miniapp_id: MiniAppId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceTestStorageResolution> {
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
    ) -> PluginRuntimePlatformResult<()> {
        self.storage
            .purge_service_test_storage(owner_user_id, miniapp_id, test_id)
            .await
    }

    async fn run_service_test(
        &self,
        input: PluginRuntimeServiceTestRunInput,
    ) -> PluginRuntimePlatformResult<MiniAppServiceTestReceipt> {
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
            .map_err(nomifun_plugin_platform::runtime::PluginRuntimePlatformError::Contract)?;
        Ok(receipt)
    }

    async fn current_runtime_fingerprint(
        &self,
    ) -> PluginRuntimePlatformResult<Option<MiniAppServiceRuntimeFingerprint>> {
        Ok(Some(Self::runtime_fingerprint()))
    }
}

fn digest(seed: &str) -> DigestHex {
    digest_bytes(seed.as_bytes())
}

fn callable_service_artifact() -> nomifun_agent_contracts::MiniAppReleaseArtifactV1 {
    let input_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {
            "value": {"type": "integer"}
        },
        "required": ["value"],
        "type": "object"
    }));
    let output_schema = StrictJsonValue(json!({
        "additionalProperties": false,
        "properties": {
            "call_id": {"type": "string"},
            "method": {"type": "string"},
            "payload": {"type": "object"}
        },
        "required": ["call_id", "method", "payload"],
        "type": "object"
    }));
    let input_schema_ref = CanonicalSchemaRef::from(format!(
        "schema://miniapp.callable/echo-input@1#{}",
        digest_payload(&input_schema.0).unwrap().as_ref()
    ));
    let output_schema_ref = CanonicalSchemaRef::from(format!(
        "schema://miniapp.callable/echo-output@1#{}",
        digest_payload(&output_schema.0).unwrap().as_ref()
    ));
    let package = PackageRef {
        id: PackageId::from("miniapp.callable"),
        version: VersionString::from("1.0.0"),
    };
    let capability = CapabilityManifest {
        id: CapabilityId::from(CALLABLE_CAPABILITY_ID),
        contribution_id: "contribution:miniapp.callable.echo".into(),
        version: VersionString::from("1.0.0"),
        kind: CapabilityKind::Tool,
        package: package.clone(),
        display: LocalizedMetadata {
            name: "Callable Echo".into(),
            description: "A deterministic callable Plugin capability fixture.".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        requires: Vec::new(),
        conflicts: Vec::new(),
        supported_surfaces: capability_surface_declarations(
            ["desktop"],
            [CapabilityConsumer::Agent, CapabilityConsumer::MiniAppService],
        ),
        requires_runtime_features: Vec::new(),
        supported_platforms: vec![PlatformConstraint::Any],
        config_schema: StrictJsonValue(json!({"type": "object"})),
        contributions: CapabilityContributions {
            actions: vec![CapabilityActionDescriptor {
                action_id: ActionId::from(CALLABLE_ACTION_ID),
                input_schema: input_schema_ref.clone(),
                output_schema: output_schema_ref.clone(),
                effect_class: EffectClass::ReadLocal,
                presentation: ToolPresentationKind::FunctionTool,
            }],
            ..Default::default()
        },
    };

    PluginRuntimeStaticBundleBuilder::new()
        .build(PluginRuntimeStaticBundleInput {
            artifact_id: Uuid::now_v7().to_string().into(),
            display: LocalizedMetadata {
                name: "Callable Service".into(),
                description: "Service Release with one callable Capability.".into(),
                localized_names: BTreeMap::new(),
                localized_descriptions: BTreeMap::new(),
            },
            ui_index_html: b"<!doctype html><html><body>callable</body></html>".to_vec(),
            ui_assets: vec![PluginRuntimeStaticBundleFile::new(
                "ui/app.js",
                b"export const ready = true;\n".to_vec(),
            )],
            service: Some(PluginRuntimeStaticServiceInput {
                main_mjs: b"export async function start() { return { async invoke() { return null; } }; }\n"
                    .to_vec(),
                lifecycle: MiniAppServiceLifecycle::OnDemand,
                uses_files: false,
                uses_private_database: false,
                service_contract_digest: digest("service-contract"),
                runtime_requirements_digest: digest("runtime-requirements"),
            }),
            package_json: Some(
                br#"{"private":true,"type":"module","scripts":{}}"#.to_vec(),
            ),
            dependency_lock_digest: digest("callable-lock"),
            dependency_graph_digest: digest("callable-graph"),
            config_schema: StrictJsonValue(json!({
                "additionalProperties": false,
                "type": "object"
            })),
            credential_slots: Vec::new(),
            resource_contract: MiniAppResourceContract::default(),
            schemas: BTreeMap::from([
                (input_schema_ref, input_schema),
                (output_schema_ref, output_schema),
            ]),
            bridge_contract_digest: digest("bridge-contract"),
            contribution_package: package,
            contributions: PackageContributions {
                capabilities: vec![capability],
                ..Default::default()
            },
            migrations: Vec::new(),
        })
        .expect("callable Service fixture must build")
}

fn release_files(
    artifact: &nomifun_agent_contracts::MiniAppReleaseArtifactV1,
) -> Vec<PluginRuntimeReleaseFileBytes> {
    artifact
        .files
        .iter()
        .map(|file| {
            let bytes = match file.normalized_relative_path.as_str() {
                "ui/index.html" => materialize_surface_entrypoint(
                    b"<!doctype html><html><body>callable</body></html>",
                )
                .unwrap(),
                "ui/app.js" => b"export const ready = true;\n".to_vec(),
                "service/main.mjs" => {
                    b"export async function start() { return { async invoke() { return null; } }; }\n"
                        .to_vec()
                }
                path => panic!("unexpected callable Service fixture path: {path}"),
            };
            assert_eq!(
                digest_bytes(&bytes),
                file.digest,
                "fixture bytes must match the built Artifact"
            );
            assert_eq!(bytes.len() as u64, file.size_bytes);
            PluginRuntimeReleaseFileBytes::new(
                file.normalized_relative_path.clone(),
                bytes,
            )
        })
        .collect()
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn contract_release_ref(
    release: &nomifun_api_types::PluginRuntimeReleaseRefDto,
) -> MiniAppReleaseRef {
    MiniAppReleaseRef {
        release_id: release.release_id.clone().into(),
        artifact_id: release.artifact_id.clone().into(),
        release_digest: release.release_digest.clone().into(),
        manifest_digest: release.manifest_digest.clone().into(),
    }
}

fn agent_invocation(
    owner: &str,
    miniapp_id: &str,
    release: &nomifun_api_types::PluginRuntimeReleaseRefDto,
    active_release_epoch: u64,
    catalog_digest: DigestHex,
    action_allowlist: BTreeSet<ActionId>,
    call_id: &str,
) -> PluginRuntimeAgentCapabilityInvocation {
    PluginRuntimeAgentCapabilityInvocation {
        owner_user_id: owner.to_owned(),
        miniapp_id: MiniAppId::from(miniapp_id),
        capability: CapabilityRef {
            id: CapabilityId::from(CALLABLE_CAPABILITY_ID),
            version: VersionString::from("1.0.0"),
        },
        action_id: ActionId::from(CALLABLE_ACTION_ID),
        action_allowlist,
        active_release: contract_release_ref(release),
        active_release_epoch,
        catalog_digest,
        operation_id: format!("operation-{call_id}").into(),
        call_id: MiniAppBridgeCallId::from(call_id),
        payload: StrictJsonValue(json!({"value": 7})),
    }
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
        PluginRuntimeM1ApplicationService::new_with_root(repository, store_root.path()).unwrap(),
    );
    let runtime = Arc::new(TestRuntime::new());
    application.install_service_runtime(runtime.clone()).await;

    let created = application
        .create(
            &owner,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: "Service Board".into(),
                description: Some("application service lifecycle".into()),
                kind: PluginRuntimeKindDto::Service,
            },
        )
        .await
        .unwrap_or_else(|error| panic!("Service Bridge failed: {error:?}"));
    assert_eq!(created.miniapp.kind, PluginRuntimeKindDto::Service);

    let built = application
        .build(
            &owner,
            BuildPluginRuntimeRequest {
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
                service_lifecycle: Some(PluginRuntimeServiceLifecycleDto::OnDemand),
            },
        )
        .await
        .unwrap_or_else(|error| panic!("Service Bridge failed: {error:?}"));
    let ready = built.ready.as_ref().unwrap();
    assert_eq!(ready.kind, PluginRuntimeKindDto::Service);
    assert_eq!(
        ready.service.as_ref().unwrap().lifecycle,
        PluginRuntimeServiceLifecycleDto::OnDemand
    );
    assert_eq!(
        ready.test.status,
        nomifun_api_types::PluginRuntimeTestStatusDto::NotRun
    );

    let tested = application
        .test_ready_service(
            &owner,
            TestPluginRuntimeReleaseRequest {
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
        nomifun_api_types::PluginRuntimeTestStatusDto::Passed
    );
    assert!(tested_ready.test.receipt_id.is_some());

    let published = application
        .publish(
            &owner,
            PublishPluginRuntimeRequest {
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
            SetPluginRuntimeEnabledRequest {
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
        nomifun_api_types::PluginRuntimeServiceHealthDto::Stopped
    );
    let started = application
        .set_service_running(
            &owner,
            nomifun_api_types::SetPluginRuntimeServiceRunningRequest {
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
        nomifun_api_types::PluginRuntimeServiceHealthDto::Ready {
            release_id: active.release_id.clone(),
            expected_release_digest: active.release_digest.clone(),
            started_at_ms: started.miniapp.updated_at_ms,
        }
    );

    let stale_catalog = application
        .invoke_agent_capability(PluginRuntimeAgentCapabilityInvocation {
            owner_user_id: owner.clone(),
            miniapp_id: created.miniapp.miniapp_id.clone().into(),
            capability: CapabilityRef {
                id: CapabilityId::from("miniapp.missing"),
                version: VersionString::from("1.0.0"),
            },
            action_id: ActionId::from("miniapp.missing.invoke"),
            action_allowlist: std::collections::BTreeSet::new(),
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
    assert_eq!(surface.kind, PluginRuntimeKindDto::Service);
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
            nomifun_api_types::SetPluginRuntimeServiceRunningRequest {
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
        nomifun_api_types::PluginRuntimeServiceHealthDto::Stopped
    );
    assert!(!runtime.started.lock().await.is_empty());
}

#[tokio::test]
async fn callable_service_release_runs_build_publish_enable_start_and_agent_invoke() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(repository, root.path()).unwrap(),
    );
    let runtime = Arc::new(TestRuntime::new());
    application.install_service_runtime(runtime.clone()).await;

    // Build a real immutable Service Release with one Agent-callable
    // contribution using the same MiniAppReleaseV1 builder as M1 builds.
    let artifact = callable_service_artifact();
    assert!(artifact.manifest.payload.service.is_some());
    assert_eq!(
        artifact.manifest.payload.contributions.capabilities.len(),
        1
    );
    let contributions = artifact.manifest.payload.contributions.clone();
    let fixture_release_store =
        PluginRuntimeReleaseStore::new(root.path().join("fixture-release-store")).unwrap();
    let stored = fixture_release_store
        .publish(PluginRuntimeReleasePublishRequest::service(
            PluginRuntimeSourceScope::new(
                "fixture-owner",
                "fixture-miniapp",
                "fixture-project",
            )
            .unwrap(),
            digest("fixture-source"),
            artifact.manifest.payload.dependency_lock_digest.clone(),
            1,
            artifact.clone(),
            release_files(&artifact),
        ))
        .unwrap()
        .stored;

    // The application import path gives the prebuilt artifact a durable
    // Ready Release; the rest of the test exercises the actual Product
    // Publish -> Enable -> Start transitions.
    let share_root = root.path().join("callable-share");
    let bundle = PluginRuntimeShareBundleFilesystem::default()
        .export(
            PluginRuntimeShareBundleExport {
                bundle_id: MiniAppShareBundleId::from("callable-service-bundle"),
                source_miniapp_id: Some(MiniAppId::from("fixture-miniapp")),
                release: &stored,
                source: None,
                test_provenance: None,
            },
            &share_root,
        )
        .unwrap();
    let prebuilt_root = root.path().join("callable-prebuilt");
    copy_tree(&share_root.join("release"), &prebuilt_root);

    let imported = application
        .import_prebuilt(
            &owner,
            ImportPluginRuntimeArtifactRequest {
                expected_library_revision: 0,
                source_path: prebuilt_root.display().to_string(),
                expected_artifact_digest: bundle.release.artifact_digest.as_ref().to_owned(),
                display_name: "Callable Service".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(imported.miniapp.kind, PluginRuntimeKindDto::Service);
    let ready = imported.ready.as_ref().expect("prebuilt Service is Ready");

    let published = application
        .publish(
            &owner,
            PublishPluginRuntimeRequest {
                miniapp_id: imported.miniapp.miniapp_id.clone(),
                expected_product_revision: imported.miniapp.product_revision,
                expected_pointer_revision: imported.miniapp.releases.pointer_revision,
                expected_active_release_epoch: imported
                    .miniapp
                    .releases
                    .active_release_epoch,
                ready_release_id: ready.release.release_id.clone(),
                expected_ready_release_digest: ready.release.release_digest.clone(),
                expected_active_release_digest: None,
                expected_service_test_receipt_id: None,
                acknowledge_test_warning: true,
            },
        )
        .await
        .unwrap();
    let active = published
        .miniapp
        .releases
        .active
        .clone()
        .expect("published Service has an Active Release");
    let active_ref = contract_release_ref(&active);
    let catalog_digest = nomifun_plugin_platform::runtime::miniapp_catalog_digest(
        &published.miniapp.miniapp_id,
        &active_ref,
        &contributions,
    )
    .unwrap();

    let enabled = application
        .set_enabled(
            &owner,
            SetPluginRuntimeEnabledRequest {
                miniapp_id: published.miniapp.miniapp_id.clone(),
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
        nomifun_api_types::PluginRuntimeServiceHealthDto::Stopped
    );

    let started = application
        .set_service_running(
            &owner,
            nomifun_api_types::SetPluginRuntimeServiceRunningRequest {
                miniapp_id: enabled.miniapp.miniapp_id.clone(),
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
    assert!(matches!(
        started.miniapp.service_health,
        nomifun_api_types::PluginRuntimeServiceHealthDto::Ready { .. }
    ));

    let allowed = BTreeSet::from([ActionId::from(CALLABLE_ACTION_ID)]);
    let result = application
        .invoke_agent_capability(agent_invocation(
            &owner,
            &started.miniapp.miniapp_id,
            &active,
            started.miniapp.releases.active_release_epoch,
            catalog_digest.clone(),
            allowed.clone(),
            "agent-call-success",
        ))
        .await
        .unwrap();
    assert_eq!(result.0["method"], json!(CALLABLE_ACTION_ID));
    assert_eq!(result.0["payload"], json!({"value": 7}));
    assert_eq!(result.0["call_id"], json!("agent-call-success"));

    let denied = application
        .invoke_agent_capability(agent_invocation(
            &owner,
            &started.miniapp.miniapp_id,
            &active,
            started.miniapp.releases.active_release_epoch,
            catalog_digest.clone(),
            BTreeSet::from([ActionId::from("miniapp.callable.other")]),
            "agent-call-denied",
        ))
        .await
        .unwrap_err();
    assert!(
        denied.to_string().contains("CAPABILITY_ACTION_NOT_ALLOWED"),
        "unexpected allowlist error: {denied}"
    );

    let stale_catalog = application
        .invoke_agent_capability(agent_invocation(
            &owner,
            &started.miniapp.miniapp_id,
            &active,
            started.miniapp.releases.active_release_epoch,
            digest("stale-catalog"),
            allowed.clone(),
            "agent-call-stale-catalog",
        ))
        .await
        .unwrap_err();
    assert!(
        stale_catalog.to_string().contains("stale Catalog digest"),
        "unexpected Catalog error: {stale_catalog}"
    );

    let stale_epoch = application
        .invoke_agent_capability(agent_invocation(
            &owner,
            &started.miniapp.miniapp_id,
            &active,
            started.miniapp.releases.active_release_epoch + 1,
            catalog_digest,
            allowed,
            "agent-call-stale-epoch",
        ))
        .await
        .unwrap_err();
    assert!(
        stale_epoch
            .to_string()
            .contains("stale against the Active Release"),
        "unexpected epoch error: {stale_epoch}"
    );

    let stopped = application
        .set_service_running(
            &owner,
            nomifun_api_types::SetPluginRuntimeServiceRunningRequest {
                miniapp_id: started.miniapp.miniapp_id.clone(),
                expected_product_revision: started.miniapp.product_revision,
                expected_pointer_revision: started.miniapp.releases.pointer_revision,
                expected_active_release_epoch: started
                    .miniapp
                    .releases
                    .active_release_epoch,
                expected_active_release_digest: active.release_digest,
                running: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        stopped.miniapp.service_health,
        nomifun_api_types::PluginRuntimeServiceHealthDto::Stopped
    );
    assert!(!runtime.started.lock().await.is_empty());
}
