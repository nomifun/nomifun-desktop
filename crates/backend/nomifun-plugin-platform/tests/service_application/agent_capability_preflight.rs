use super::*;
use nomifun_plugin_platform::runtime::*;

struct UnsupportedOwner;

#[async_trait]
impl PluginRuntimeAgentCapabilityPort for UnsupportedOwner {
    async fn invoke_agent_capability(
        &self,
        _request: PluginRuntimeAgentCapabilityInvocation,
    ) -> Result<StrictJsonValue, PluginRuntimeApplicationError> {
        panic!("unsupported preflight must not invoke")
    }
}

struct UntouchableRuntime;

#[async_trait]
impl PluginRuntimeServiceRuntimeBinding for UntouchableRuntime {
    async fn resolve_spec(
        &self,
        _input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedPluginServiceSpec> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn bind_active(
        &self,
        _spec: ResolvedPluginServiceSpec,
        _enabled: bool,
    ) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn start(
        &self,
        _spec: ResolvedPluginServiceSpec,
    ) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn invoke(
        &self,
        _spec: &ResolvedPluginServiceSpec,
        _call_id: PluginBridgeCallId,
        _method: String,
        _payload: StrictJsonValue,
        _cancellation: PluginRuntimeCallCancellation,
        _now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn cancel(&self, _plugin_product_id: &PluginProductId, _call_id: &PluginBridgeCallId) { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn stop(&self, _plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn retry(&self, _plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn state(&self, _plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn maintain(&self, _now_ms: i64) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }

    async fn register_module(
        &self,
        _plugin_product_id: PluginProductId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()> { panic!("preflight or rejected invoke touched the Service runtime") }
    async fn resolve_storage(
        &self,
        _owner_user_id: &str,
        _plugin_product_id: &PluginProductId,
        _uses_files: bool,
        _uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceStorageResolution> {
        panic!("preflight or rejected invoke resolved storage")
    }
}

#[tokio::test]
async fn preflight_checks_current_product_without_touching_the_service_runtime() {
    let database = init_database_memory().await.unwrap();
    let owner = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IPluginRuntimeRepository> = Arc::new(
        SqlitePluginRuntimeRepository::new(database.pool().clone()),
    );
    let root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeApplicationService::new_with_root(repository, root.path()).unwrap(),
    );
    let runtime = Arc::new(TestRuntime::new());
    application.install_service_runtime(runtime.clone()).await;

    // Build a real immutable Service Release with one Agent-callable
    // contribution using the same PluginReleaseV1 builder as M1 builds.
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
                "fixture-plugin",
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
                bundle_id: PluginShareBundleId::from("callable-service-bundle"),
                source_plugin_product_id: Some(PluginProductId::from("fixture-plugin")),
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
    assert_eq!(imported.plugin.kind, PluginRuntimeKindDto::Plugin);
    let ready = imported.ready.as_ref().expect("prebuilt Service is Ready");

    let published = application
        .publish(
            &owner,
            PublishPluginRuntimeRequest {
                plugin_id: imported.plugin.plugin_id.clone(),
                expected_product_revision: imported.plugin.product_revision,
                expected_pointer_revision: imported.plugin.releases.pointer_revision,
                expected_active_release_epoch: imported
                    .plugin
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
        .plugin
        .releases
        .active
        .clone()
        .expect("published Service has an Active Release");
    let active_ref = contract_release_ref(&active);
    let catalog_digest = nomifun_plugin_platform::runtime::plugin_catalog_digest(
        &published.plugin.plugin_id,
        &active_ref,
        &contributions,
    )
    .unwrap();

    let enabled = application
        .set_enabled(
            &owner,
            SetPluginRuntimeEnabledRequest {
                plugin_id: published.plugin.plugin_id.clone(),
                expected_product_revision: published.plugin.product_revision,
                expected_pointer_revision: published.plugin.releases.pointer_revision,
                expected_active_release_digest: Some(active.release_digest.clone()),
                enabled: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        enabled.plugin.service_health,
        nomifun_api_types::PluginRuntimeServiceHealthDto::Stopped
    );


    application.install_service_runtime(Arc::new(UntouchableRuntime)).await;
    let request = agent_invocation(
        &owner, &enabled.plugin.plugin_id, &active,
        enabled.plugin.releases.active_release_epoch, catalog_digest,
        BTreeSet::from([ActionId::from(CALLABLE_ACTION_ID)]), "preflight",
    );
    assert!(UnsupportedOwner.preflight_agent_capability(&request).await
        .unwrap_err().to_string().contains("unsupported"));
    // A valid preflight must work even though every Service runtime entrypoint panics.
    application.preflight_agent_capability(&request).await.unwrap();
    let port: &dyn PluginRuntimeAgentCapabilityPort = application.as_ref();
    port.preflight_agent_capability(&request).await.unwrap();
    for mutation in ["owner", "epoch", "catalog", "release", "action", "allowlist", "schema"] {
        let mut denied = request.clone();
        match mutation {
            "owner" => denied.owner_user_id = "another-owner".into(),
            "epoch" => denied.active_release_epoch += 1,
            "catalog" => denied.catalog_digest = digest("stale-catalog"),
            "release" => denied.active_release.release_digest = digest("stale-release"),
            "action" => denied.action_id = ActionId::from("undeclared-action"),
            "allowlist" => denied.action_allowlist = BTreeSet::from([ActionId::from("other-action")]),
            _ => denied.payload = StrictJsonValue(json!({"value": "PRIVATE_INVALID_PAYLOAD"})),
        }
        let error = application.preflight_agent_capability(&denied).await.unwrap_err();
        assert!(!error.to_string().contains("PRIVATE_INVALID_PAYLOAD"));
        let final_error = application.invoke_agent_capability(denied).await.unwrap_err();
        assert_eq!(error.to_string(), final_error.to_string(), "{mutation}");
    }
    // Cancellation already observed by the application owner must not resolve
    // storage/specs or start Node, even for otherwise valid frozen authority.
    let mut canceled = request.clone();
    canceled.cancellation = Default::default();
    canceled.cancellation.cancel();
    let error = application.invoke_agent_capability(canceled).await.unwrap_err();
    assert!(error.to_string().contains("canceled before dispatch"));

    // A successful preflight never grants future access after the Product is disabled.
    application.install_service_runtime(runtime.clone()).await;
    application.set_enabled(&owner, SetPluginRuntimeEnabledRequest {
        plugin_id: enabled.plugin.plugin_id,
        expected_product_revision: enabled.plugin.product_revision,
        expected_pointer_revision: enabled.plugin.releases.pointer_revision,
        expected_active_release_digest: Some(active.release_digest),
        enabled: false,
    }).await.unwrap();
    application.install_service_runtime(Arc::new(UntouchableRuntime)).await;
    assert!(application.preflight_agent_capability(&request).await.is_err());
    assert!(application.invoke_agent_capability(request).await.is_err());
}
