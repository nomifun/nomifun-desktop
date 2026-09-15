use super::*;

#[tokio::test]
#[ignore = "requires compiled Rust SDK echo executable; see native plugin developer guide"]
async fn native_release_import_publish_enable_agent_invoke_without_node() {
    tokio::time::timeout(std::time::Duration::from_secs(45), async {
        let database = init_database_memory().await.unwrap();
        let owner = installation_owner_id(database.pool()).await.unwrap();
        let root = tempfile::tempdir().unwrap();
        let application = Arc::new(
            PluginRuntimeApplicationService::new_with_root(
                Arc::new(SqlitePluginRuntimeRepository::new(database.pool().clone())),
                root.path(),
            )
            .unwrap(),
        );
        let runtime = native_support::binding(root.path(), true);
        application.install_service_runtime(runtime.clone()).await;
        let original = callable_service_artifact();
        let materialized = native_support::materialization(native_support::executable());
        let mut files = release_files(&original);
        files.retain(|file| file.normalized_relative_path != "service/main.mjs");
        files.push(PluginRuntimeReleaseFileBytes::new(
            &materialized.file.normalized_relative_path,
            materialized.file.bytes.clone(),
        ));
        let mut manifest = original.manifest.payload.clone();
        manifest.service = Some(materialized.descriptor);
        let artifact = nomifun_agent_contracts::PluginReleaseArtifactV1::new(
            original.artifact_id.clone(),
            manifest,
            files
                .iter()
                .map(|file| nomifun_agent_contracts::PluginReleaseFile {
                    normalized_relative_path: file.normalized_relative_path.clone(),
                    digest: digest_bytes(&file.bytes),
                    size_bytes: file.bytes.len() as u64,
                })
                .collect(),
        )
        .unwrap();
        let store =
            PluginRuntimeReleaseStore::new(root.path().join("fixture-release-store")).unwrap();
        let stored = store
            .publish(PluginRuntimeReleasePublishRequest::service(
                PluginRuntimeSourceScope::new("fixture-owner", "fixture-plugin", "fixture-project")
                    .unwrap(),
                digest("source"),
                artifact.manifest.payload.dependency_lock_digest.clone(),
                1,
                artifact.clone(),
                files,
            ))
            .unwrap()
            .stored;
        let share = root.path().join("share");
        let bundle = PluginRuntimeShareBundleFilesystem::default()
            .export(
                PluginRuntimeShareBundleExport {
                    bundle_id: "native-bundle".into(),
                    source_plugin_product_id: Some("fixture-plugin".into()),
                    release: &stored,
                    source: None,
                    test_provenance: None,
                },
                &share,
            )
            .unwrap();
        let imported = application
            .import_prebuilt(
                &owner,
                ImportPluginRuntimeArtifactRequest {
                    expected_library_revision: 0,
                    source_path: share.join("release").display().to_string(),
                    expected_artifact_digest: bundle.release.artifact_digest.as_ref().to_owned(),
                    display_name: "Rust Echo".into(),
                },
            )
            .await
            .unwrap();
        let ready = imported.ready.as_ref().unwrap();
        let tested = application
            .test_ready_service(
                &owner,
                TestPluginRuntimeReleaseRequest {
                    plugin_id: imported.plugin.plugin_id.clone(),
                    expected_product_revision: imported.plugin.product_revision,
                    expected_pointer_revision: imported.plugin.releases.pointer_revision,
                    project_id: imported.project_id.clone(),
                    expected_project_revision: imported.project_revision,
                    expected_build_generation: imported.build_generation,
                    release_id: ready.release.release_id.clone(),
                    expected_release_digest: ready.release.release_digest.clone(),
                    expected_config_revision: imported.config.config_revision,
                    expected_credential_bindings_revision: imported.credential_bindings_revision,
                    resolved_test_input_digest: digest_bytes(b"").as_ref().to_owned(),
                },
            )
            .await
            .unwrap();
        let ready = tested.ready.as_ref().unwrap();
        // The existing Test Host probes startup, not arbitrary action inputs.
        // Preserve its honest warning; actual Agent invocation is tested below.
        assert_eq!(
            ready.test.status,
            nomifun_api_types::PluginRuntimeTestStatusDto::NeedsTestInput
        );
        let published = application
            .publish(
                &owner,
                PublishPluginRuntimeRequest {
                    plugin_id: tested.plugin.plugin_id.clone(),
                    expected_product_revision: tested.plugin.product_revision,
                    expected_pointer_revision: tested.plugin.releases.pointer_revision,
                    expected_active_release_epoch: tested.plugin.releases.active_release_epoch,
                    ready_release_id: ready.release.release_id.clone(),
                    expected_ready_release_digest: ready.release.release_digest.clone(),
                    expected_active_release_digest: None,
                    expected_service_test_receipt_id: ready.test.receipt_id.clone(),
                    acknowledge_test_warning: true,
                },
            )
            .await
            .unwrap();
        let active = published.plugin.releases.active.clone().unwrap();
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
        let catalog = nomifun_plugin_platform::runtime::plugin_catalog_digest(
            &enabled.plugin.plugin_id,
            &contract_release_ref(&active),
            &artifact.manifest.payload.contributions,
        )
        .unwrap();
        let value = application
            .invoke_agent_capability(agent_invocation(
                &owner,
                &enabled.plugin.plugin_id,
                &active,
                enabled.plugin.releases.active_release_epoch,
                catalog.clone(),
                BTreeSet::from([CALLABLE_ACTION_ID.into()]),
                "rust-agent-call",
            ))
            .await
            .unwrap();
        assert_eq!(value.0["payload"], json!({"value":7}));
        assert_eq!(value.0["method"], json!(CALLABLE_ACTION_ID));
        let native_id: PluginProductId = enabled.plugin.plugin_id.clone().into();
        let before = runtime.state(&native_id).await;
        application
            .shutdown_node_service_runtime(&owner)
            .await
            .unwrap();
        assert_eq!(
            runtime.state(&native_id).await,
            before,
            "Node switch stopped native Service"
        );
        let candidate = nomifun_js_runtime::ResolvedNodeRuntime {
            executable_path: root.path().join("nonexistent-node.exe"),
            fingerprint: nomifun_agent_contracts::NodeRuntimeFingerprint {
                runtime_installation_id: "unused-node".into(),
                source_kind: nomifun_agent_contracts::NodeRuntimeSourceKind::Managed,
                node_version: "24.1.0".into(),
                node_major: 24,
                runtime_target: "windows-x86_64".into(),
                executable_digest: digest("unused-node"),
                javascript_host_protocol_version:
                    nomifun_agent_contracts::JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version:
                    nomifun_agent_contracts::JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            },
        };
        application
            .validate_service_runtime_candidate(&owner, &candidate)
            .await
            .unwrap();
        assert!(
            runtime
                .validate_candidate(&candidate)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            application
                .invoke_agent_capability(agent_invocation(
                    &owner,
                    &enabled.plugin.plugin_id,
                    &active,
                    enabled.plugin.releases.active_release_epoch,
                    catalog,
                    BTreeSet::from(["plugin.callable.other".into()]),
                    "rust-denied"
                ))
                .await
                .is_err()
        );
        runtime
            .stop(&enabled.plugin.plugin_id.into())
            .await
            .unwrap();
    })
    .await
    .expect("native Product integration timed out");
}
