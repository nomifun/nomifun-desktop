use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ArtifactId, CredentialSlotDeclaration, CredentialSlotKey, CredentialSlotKind, DigestHex,
    JavaScriptBuildProfile, LocalizedMetadata, MiniAppDatabaseHandleId,
    MiniAppFilesDirDescriptor, MiniAppFilesHandleId, MiniAppId, MiniAppKvHandleDescriptor,
    MiniAppKvHandleId, MiniAppNonUiReleaseFingerprint, MiniAppPointerExpectation,
    MiniAppPrivateDatabaseDescriptor, MiniAppProductLifecycleState, MiniAppProjectId,
    MiniAppPublishAuthorization, MiniAppPublishRequest, MiniAppReadyOrigin, MiniAppReadyRelease,
    MiniAppReleaseArtifactV1, MiniAppReleaseFile, MiniAppReleaseId, MiniAppReleaseRef,
    MiniAppReleaseV1Manifest, MiniAppResourceContract, MiniAppRollbackRequest,
    MiniAppServiceLifecycle, MiniAppServiceReleaseDescriptor, MiniAppServiceRuntimeFingerprint,
    MiniAppServiceStorageDescriptor, MiniAppSourceLineage, MiniAppUiOnlyAutoPublishAuthorization,
    MiniAppUiOnlyAutoPublishProof, MiniAppUserAuthorizationId, OperationId, PackageContributions,
    PackageId, PackageRef, ResolvedMiniAppServiceSpec, ResolvedMiniAppServiceSpecInputs,
    ResourceKind, RuntimeInstallationId, RuntimeTarget, StrictJsonValue, VersionString,
    MINIAPP_M1_SCHEMA_VERSION, MINIAPP_RELEASE_PROFILE_VERSION,
    MINIAPP_SERVICE_HOST_PROTOCOL_VERSION, MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
    canonical_ui_tree_digest, digest_bytes, digest_payload,
};

use crate::runtime::*;

const T0: i64 = 1_800_000_000_000;

#[tokio::test]
async fn repository_contract_keeps_release_slots_and_catalog_atomic() {
    let repository: Arc<dyn PluginRuntimeRepository> = Arc::new(InMemoryPluginRuntimeRepository::new());
    let first = create_root(&repository, "miniapp-a", PluginRuntimeKind::UiOnly).await;
    let second = create_root(&repository, "miniapp-b", PluginRuntimeKind::UiOnly).await;
    let second_expectation = PluginRuntimeMutationExpectation::from_snapshot(&second);

    let first = complete_ready(
        &repository,
        &first,
        artifact("a-v1", false),
        "import-a-v1",
        T0 + 10,
    )
    .await;
    let second = complete_ready_with_expectation(
        &repository,
        second_expectation,
        artifact("b-v1", false),
        "import-b-v1",
        T0 + 11,
    )
    .await;
    assert!(
        second.root.product.pointers.ready_release.is_some(),
        "a mutation to another Plugin must not invalidate this owner CAS"
    );

    let first = publish_repository(&repository, &first, "catalog-a-v1", T0 + 20).await;
    assert_eq!(
        first.root.product.lifecycle,
        MiniAppProductLifecycleState::Disabled
    );
    assert!(first.catalog.is_none());
    assert_eq!(first.root.product.pointers.active_release_epoch, 1);
    assert!(first.root.product.pointers.ready_release.is_none());

    let first = lifecycle_repository(
        &repository,
        &first,
        PluginRuntimeLifecycleCommand::Enable,
        T0 + 30,
    )
    .await;
    assert_eq!(
        first
            .catalog
            .as_ref()
            .expect("enabled Plugin publishes Catalog")
            .catalog_digest,
        digest("catalog-a-v1")
    );

    let first = complete_ready(
        &repository,
        &first,
        artifact("a-v2", false),
        "import-a-v2",
        T0 + 40,
    )
    .await;
    let first = publish_repository(&repository, &first, "catalog-a-v2", T0 + 50).await;
    let v2 = first
        .root
        .product
        .pointers
        .active_release
        .clone()
        .expect("v2 active");
    let v1 = first
        .root
        .product
        .pointers
        .previous_release
        .clone()
        .expect("v1 previous");
    assert_eq!(first.root.releases.len(), 2);
    assert_eq!(
        first.catalog.as_ref().expect("catalog").active_release,
        v2
    );

    let rollback = MiniAppRollbackRequest {
        miniapp_id: first.root.product.miniapp_id.clone(),
        expected: MiniAppPointerExpectation::from_state(&first.root.product.pointers),
        rollback_target: v1.clone(),
        target_catalog_digest: digest("catalog-a-v1-restored"),
        actor_id: "user-1".into(),
    };
    let rolled_back = repository
        .commit_release(CommitPluginRuntimeRelease {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&first),
            command: PluginRuntimeReleaseCommand::Rollback(rollback),
            now_ms: T0 + 60,
        })
        .await
        .unwrap();
    assert_eq!(
        rolled_back.root.product.pointers.active_release.as_ref(),
        Some(&v1)
    );
    assert_eq!(
        rolled_back.root.product.pointers.previous_release.as_ref(),
        Some(&v2)
    );
    assert_eq!(rolled_back.root.product.pointers.active_release_epoch, 3);
    assert_eq!(
        rolled_back
            .catalog
            .as_ref()
            .expect("rollback updates catalog")
            .catalog_digest,
        digest("catalog-a-v1-restored")
    );
    assert_eq!(rolled_back.root.releases.len(), 2);
    rolled_back.validate().unwrap();
}

#[tokio::test]
async fn repository_contract_enforces_lifecycle_without_intermediate_disabled_commit() {
    let repository: Arc<dyn PluginRuntimeRepository> = Arc::new(InMemoryPluginRuntimeRepository::new());
    let created = create_root(&repository, "miniapp-lifecycle", PluginRuntimeKind::UiOnly).await;
    let enable_without_active = repository
        .commit_lifecycle(CommitPluginRuntimeLifecycle {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&created),
            command: PluginRuntimeLifecycleCommand::Enable,
            now_ms: T0 + 1,
        })
        .await;
    assert!(matches!(
        enable_without_active,
        Err(PluginRuntimePlatformError::InvalidState(_))
    ));
    assert_eq!(
        repository
            .get(&created.root.product.miniapp_id)
            .await
            .unwrap()
            .root
            .product
            .lifecycle,
        MiniAppProductLifecycleState::Disabled
    );

    let ready = complete_ready(
        &repository,
        &created,
        artifact("lifecycle-v1", false),
        "import-lifecycle",
        T0 + 10,
    )
    .await;
    let published = publish_repository(&repository, &ready, "catalog-lifecycle", T0 + 20).await;
    let mut running = DurablePluginRuntimeOperation::running(
        OperationId::from("lifecycle-build"),
        published.root.product.miniapp_id.clone(),
        PluginRuntimeOperationKind::Build,
        true,
        T0 + 21,
    )
    .unwrap();
    repository.start_operation(running.clone()).await.unwrap();
    let blocked = repository
        .commit_lifecycle(CommitPluginRuntimeLifecycle {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&published),
            command: PluginRuntimeLifecycleCommand::Enable,
            now_ms: T0 + 22,
        })
        .await;
    assert!(matches!(blocked, Err(PluginRuntimePlatformError::OwnerBusy(_))));
    running.cancel(1, T0 + 23).unwrap();
    repository.finish_operation(1, running).await.unwrap();
    let enabled = lifecycle_repository(
        &repository,
        &published,
        PluginRuntimeLifecycleCommand::Enable,
        T0 + 30,
    )
    .await;
    let enabled_revision = enabled.root.product.product_revision;
    let trashed = lifecycle_repository(
        &repository,
        &enabled,
        PluginRuntimeLifecycleCommand::Trash,
        T0 + 40,
    )
    .await;
    assert_eq!(
        trashed.root.product.product_revision,
        enabled_revision + 1,
        "Trash from Enabled is one owner transaction"
    );
    assert_eq!(
        trashed.root.product.lifecycle,
        MiniAppProductLifecycleState::Trashed
    );
    assert!(trashed.catalog.is_none());

    let publish_while_trashed =
        publish_repository_result(&repository, &trashed, "catalog-forbidden", T0 + 41).await;
    assert!(matches!(
        publish_while_trashed,
        Err(PluginRuntimePlatformError::LifecycleConflict(_))
            | Err(PluginRuntimePlatformError::InvalidState(_))
    ));

    let restored = lifecycle_repository(
        &repository,
        &trashed,
        PluginRuntimeLifecycleCommand::Restore,
        T0 + 50,
    )
    .await;
    assert_eq!(
        restored.root.product.lifecycle,
        MiniAppProductLifecycleState::Disabled
    );
    assert!(restored.catalog.is_none());
}

#[tokio::test]
async fn repository_contract_keeps_delete_intent_and_operation_outside_owner_root() {
    let repository: Arc<dyn PluginRuntimeRepository> = Arc::new(InMemoryPluginRuntimeRepository::new());
    let created = create_root(&repository, "miniapp-delete", PluginRuntimeKind::UiOnly).await;
    let ready = complete_ready(
        &repository,
        &created,
        artifact("delete-v1", false),
        "import-delete",
        T0 + 10,
    )
    .await;
    let published = publish_repository(&repository, &ready, "catalog-delete", T0 + 20).await;
    let enabled = lifecycle_repository(
        &repository,
        &published,
        PluginRuntimeLifecycleCommand::Enable,
        T0 + 30,
    )
    .await;
    let trashed = lifecycle_repository(
        &repository,
        &enabled,
        PluginRuntimeLifecycleCommand::Trash,
        T0 + 40,
    )
    .await;

    let first_operation = OperationId::from("delete-operation-1");
    let deleting = repository
        .begin_delete(BeginPluginRuntimeDelete {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&trashed),
            operation_id: first_operation.clone(),
            now_ms: T0 + 50,
        })
        .await
        .unwrap();
    let deletion = deleting.deletion.as_ref().expect("deleting intent");
    assert_eq!(
        deleting.root.product.lifecycle,
        MiniAppProductLifecycleState::Deleting
    );
    assert!(!deletion.operation.cancelable);
    assert!(deleting.catalog.is_none());
    let mut cancel_attempt = deletion.operation.clone();
    assert!(matches!(
        cancel_attempt.cancel(1, T0 + 51),
        Err(PluginRuntimePlatformError::OperationNotCancelable(_))
    ));

    let failed = repository
        .fail_delete(FailPluginRuntimeDelete {
            miniapp_id: deleting.root.product.miniapp_id.clone(),
            operation_id: first_operation.clone(),
            expected_operation_revision: 1,
            error: "managed_data_unavailable".into(),
            now_ms: T0 + 60,
        })
        .await
        .unwrap();
    assert_eq!(
        failed
            .deletion
            .as_ref()
            .expect("failed intent remains")
            .operation
            .state,
        PluginRuntimeOperationState::Failed
    );

    let second_operation = OperationId::from("delete-operation-2");
    let restarted = repository
        .restart_delete(RestartPluginRuntimeDelete {
            miniapp_id: failed.root.product.miniapp_id.clone(),
            expected_operation_id: first_operation.clone(),
            operation_id: second_operation.clone(),
            now_ms: T0 + 70,
        })
        .await
        .unwrap();
    assert_eq!(
        restarted
            .deletion
            .as_ref()
            .expect("replacement intent")
            .operation
            .operation_id,
        second_operation
    );
    assert_eq!(
        repository
            .get_operation(&first_operation)
            .await
            .unwrap()
            .state,
        PluginRuntimeOperationState::Failed
    );

    repository
        .finalize_delete(FinalizePluginRuntimeDelete {
            miniapp_id: restarted.root.product.miniapp_id.clone(),
            operation_id: second_operation.clone(),
            expected_operation_revision: 1,
            now_ms: T0 + 80,
        })
        .await
        .unwrap();
    assert!(matches!(
        repository.get(&restarted.root.product.miniapp_id).await,
        Err(PluginRuntimePlatformError::NotFound(_))
    ));
    assert_eq!(
        repository
            .get_operation(&second_operation)
            .await
            .unwrap()
            .state,
        PluginRuntimeOperationState::Succeeded
    );
}

#[tokio::test]
async fn repository_contract_rejects_cross_shape_release_and_legacy_state() {
    let repository: Arc<dyn PluginRuntimeRepository> = Arc::new(InMemoryPluginRuntimeRepository::new());
    let ui = create_root(&repository, "miniapp-ui", PluginRuntimeKind::UiOnly).await;
    let service_artifact = artifact("service-on-ui", true);
    let operation = start_import(&repository, &ui, "import-service-on-ui").await;
    let completed = complete_import_operation(operation, &service_artifact, T0 + 10);
    let result = repository
        .complete_ready_release(CompleteReadyReleaseCommit {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&ui),
            release: stored_release(
                &ui.root.product.miniapp_id,
                service_artifact,
                &completed.operation_id,
                T0 + 10,
            ),
            completed_operation: completed,
            now_ms: T0 + 10,
        })
        .await;
    assert!(matches!(result, Err(PluginRuntimePlatformError::InvalidState(_))));

    let service = create_root(&repository, "miniapp-service", PluginRuntimeKind::Service).await;
    let ui_artifact = artifact("ui-on-service", false);
    let operation = start_import(&repository, &service, "import-ui-on-service").await;
    let completed = complete_import_operation(operation, &ui_artifact, T0 + 20);
    let result = repository
        .complete_ready_release(CompleteReadyReleaseCommit {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&service),
            release: stored_release(
                &service.root.product.miniapp_id,
                ui_artifact,
                &completed.operation_id,
                T0 + 20,
            ),
            completed_operation: completed,
            now_ms: T0 + 20,
        })
        .await;
    assert!(matches!(result, Err(PluginRuntimePlatformError::InvalidState(_))));

    let encoded = serde_json::to_value(&service.root).unwrap();
    let text = serde_json::to_string(&encoded).unwrap();
    for legacy in [
        "conversation_id",
        "guid_mode",
        "legacy_miniapp_id",
        "html",
        "resolved_service_spec",
        "service_health",
    ] {
        assert!(!text.contains(legacy), "legacy field leaked: {legacy}");
    }
}

#[tokio::test]
async fn application_service_allows_strict_ui_change_for_service_miniapp() {
    let repository: Arc<dyn PluginRuntimeRepository> = Arc::new(InMemoryPluginRuntimeRepository::new());
    let runtime = Arc::new(RecordingRuntime::default());
    let service = PluginRuntimeApplicationService::new(PluginRuntimeApplicationDependencies {
        repository: repository.clone(),
        runtime: runtime.clone(),
        managed_data: Arc::new(NoopPluginRuntimeManagedData),
        mutations: Arc::new(PluginRuntimeOwnerMutationCoordinator::new()),
    });
    let miniapp_id = MiniAppId::from("miniapp-service-auto");
    let created = service
        .create(CreatePluginRuntime {
            expected_library_revision: 0,
            miniapp_id: miniapp_id.clone(),
            project_id: MiniAppProjectId::from("project-service-auto"),
            display_name: "Service Auto".into(),
            description: None,
            kind: PluginRuntimeKind::Service,
            storage: storage(&miniapp_id, PluginRuntimeKind::Service),
            now_ms: T0,
        })
        .await
        .unwrap();

    let first_artifact = artifact("service-auto-v1", true);
    let first = complete_ready(
        &repository,
        &created,
        first_artifact,
        "import-service-auto-v1",
        T0 + 10,
    )
    .await;
    let first_target = first.root.ready_release().unwrap().clone();
    let first_spec = service_spec(&first, &first_target, 1);
    let first = service
        .publish(PublishPluginRuntime {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&first),
            authorization: MiniAppPublishAuthorization::ManualUser {
                actor_id: "user-1".into(),
            },
            target_catalog_digest: digest("catalog-service-auto-v1"),
            current_service_spec: None,
            target_service_spec: Some(first_spec.clone()),
            now_ms: T0 + 20,
        })
        .await
        .unwrap();
    let first = service
        .change_lifecycle(ChangePluginRuntimeLifecycle {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&first),
            command: PluginRuntimeLifecycleCommand::Enable,
            active_service_spec: Some(first_spec.clone()),
            now_ms: T0 + 30,
        })
        .await
        .unwrap();

    let second_artifact = artifact("service-auto-v2", true);
    let second = complete_ready(
        &repository,
        &first,
        second_artifact,
        "import-service-auto-v2",
        T0 + 40,
    )
    .await;
    let current = second.root.active_release().unwrap().clone();
    let target = second.root.ready_release().unwrap().clone();
    let target_spec = service_spec(&second, &target, 2);
    assert_eq!(
        first_spec.service_run_key, target_spec.service_run_key,
        "UI-only changes must keep the Service run key stable"
    );
    let proof = MiniAppUiOnlyAutoPublishProof {
        current_release: current.release_ref().clone(),
        target_release: target.release_ref().clone(),
        current_ui_tree_digest: current.artifact.manifest.payload.ui.as_ref().unwrap().ui_tree_digest.clone(),
        target_ui_tree_digest: target.artifact.manifest.payload.ui.as_ref().unwrap().ui_tree_digest.clone(),
        current_non_ui: non_ui_fingerprint(&current, &first_spec),
        target_non_ui: non_ui_fingerprint(&target, &target_spec),
        changed_source_paths: BTreeSet::from(["ui/app.js".into()]),
        changed_output_paths: BTreeSet::from(["ui/app.js".into()]),
        project_head_matches_ready_source: true,
        static_validation_passed: true,
        no_unknown_changes: true,
    };
    let authorization = MiniAppUiOnlyAutoPublishAuthorization {
        authorization_id: MiniAppUserAuthorizationId::from("auto-service-ui"),
        miniapp_id: miniapp_id.clone(),
        enabled: true,
        authorization_revision: 1,
        user_authorized_at_ms: T0 + 1,
    };
    let forged = service
        .publish(PublishPluginRuntime {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&second),
            authorization: MiniAppPublishAuthorization::AutoUiOnly {
                authorization: authorization.clone(),
                proof: Box::new(proof.clone()),
            },
            target_catalog_digest: digest("catalog-service-auto-v2"),
            current_service_spec: Some(first_spec.clone()),
            target_service_spec: Some(target_spec.clone()),
            now_ms: T0 + 49,
        })
        .await;
    assert!(matches!(forged, Err(PluginRuntimePlatformError::InvalidState(_))));
    let authorized = service
        .set_auto_publish(SetPluginRuntimeAutoPublish {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&second),
            authorization: Some(authorization.clone()),
            now_ms: T0 + 49,
        })
        .await
        .unwrap();
    let published = service
        .publish(PublishPluginRuntime {
            expected: PluginRuntimeMutationExpectation::from_snapshot(&authorized),
            authorization: MiniAppPublishAuthorization::AutoUiOnly {
                authorization,
                proof: Box::new(proof),
            },
            target_catalog_digest: digest("catalog-service-auto-v2"),
            current_service_spec: Some(first_spec),
            target_service_spec: Some(target_spec),
            now_ms: T0 + 50,
        })
        .await
        .unwrap();
    assert_eq!(published.root.product.pointers.active_release_epoch, 2);
    assert_eq!(
        runtime.release_restart_flags.lock().unwrap().as_slice(),
        &[true, false],
        "first Publish starts Service; strict UI Publish reuses it"
    );
}

#[derive(Default)]
struct RecordingRuntime {
    release_restart_flags: Mutex<Vec<bool>>,
}

#[async_trait]
impl PluginRuntimeRuntimePort for RecordingRuntime {
    async fn prepare_release_cutover(
        &self,
        plan: &PluginRuntimeReleaseCutoverPlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket> {
        plan.validate()?;
        self.release_restart_flags
            .lock()
            .unwrap()
            .push(plan.requires_service_restart());
        Ok(PluginRuntimeRuntimeTicket {
            ticket_id: format!("release-ticket-{}", self.release_restart_flags.lock().unwrap().len()),
        })
    }

    async fn complete_release_cutover(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn abort_release_cutover(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn prepare_lifecycle(
        &self,
        plan: &PluginRuntimeLifecyclePlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket> {
        plan.validate()?;
        Ok(PluginRuntimeRuntimeTicket {
            ticket_id: "lifecycle-ticket".into(),
        })
    }

    async fn complete_lifecycle(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn abort_lifecycle(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn prepare_delete(
        &self,
        _snapshot: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }
}

async fn create_root(
    repository: &Arc<dyn PluginRuntimeRepository>,
    id: &str,
    kind: PluginRuntimeKind,
) -> PluginRuntimeRepositorySnapshot {
    let miniapp_id = MiniAppId::from(id);
    let root = PluginRuntimeDataRoot::new(
        miniapp_id.clone(),
        MiniAppProjectId::from(format!("project-{id}")),
        id.into(),
        None,
        kind,
        storage(&miniapp_id, kind),
        T0,
    )
    .unwrap();
    repository
        .create(CreatePluginRuntimeCommit {
            expected_library_revision: repository.library_revision().await.unwrap(),
            root,
        })
        .await
        .unwrap()
}

async fn complete_ready(
    repository: &Arc<dyn PluginRuntimeRepository>,
    snapshot: &PluginRuntimeRepositorySnapshot,
    artifact: MiniAppReleaseArtifactV1,
    operation_id: &str,
    now_ms: i64,
) -> PluginRuntimeRepositorySnapshot {
    complete_ready_with_expectation(
        repository,
        PluginRuntimeMutationExpectation::from_snapshot(snapshot),
        artifact,
        operation_id,
        now_ms,
    )
    .await
}

async fn complete_ready_with_expectation(
    repository: &Arc<dyn PluginRuntimeRepository>,
    expectation: PluginRuntimeMutationExpectation,
    artifact: MiniAppReleaseArtifactV1,
    operation_id: &str,
    now_ms: i64,
) -> PluginRuntimeRepositorySnapshot {
    let operation = DurablePluginRuntimeOperation::running(
        OperationId::from(operation_id),
        expectation.miniapp_id.clone(),
        PluginRuntimeOperationKind::Import,
        true,
        now_ms - 1,
    )
    .unwrap();
    repository.start_operation(operation.clone()).await.unwrap();
    let completed = complete_import_operation(operation, &artifact, now_ms);
    let release = stored_release(
        &expectation.miniapp_id,
        artifact,
        &completed.operation_id,
        now_ms,
    );
    repository
        .complete_ready_release(CompleteReadyReleaseCommit {
            expected: expectation,
            release,
            completed_operation: completed,
            now_ms,
        })
        .await
        .unwrap()
}

async fn start_import(
    repository: &Arc<dyn PluginRuntimeRepository>,
    snapshot: &PluginRuntimeRepositorySnapshot,
    operation_id: &str,
) -> DurablePluginRuntimeOperation {
    let operation = DurablePluginRuntimeOperation::running(
        OperationId::from(operation_id),
        snapshot.root.product.miniapp_id.clone(),
        PluginRuntimeOperationKind::Import,
        true,
        T0 + 1,
    )
    .unwrap();
    repository.start_operation(operation.clone()).await.unwrap();
    operation
}

fn complete_import_operation(
    mut operation: DurablePluginRuntimeOperation,
    artifact: &MiniAppReleaseArtifactV1,
    now_ms: i64,
) -> DurablePluginRuntimeOperation {
    operation
        .succeed(
            1,
            now_ms,
            BTreeMap::from([("release".into(), artifact.artifact_digest.clone())]),
        )
        .unwrap();
    operation
}

fn stored_release(
    miniapp_id: &MiniAppId,
    artifact: MiniAppReleaseArtifactV1,
    operation_id: &OperationId,
    now_ms: i64,
) -> StoredPluginRuntimeRelease {
    let suffix = artifact.artifact_id.as_ref().replace("artifact-", "");
    let release = MiniAppReleaseRef {
        release_id: MiniAppReleaseId::from(format!("release-{suffix}")),
        artifact_id: artifact.artifact_id.clone(),
        release_digest: artifact.artifact_digest.clone(),
        manifest_digest: artifact.manifest.payload_digest.clone(),
    };
    StoredPluginRuntimeRelease::new(
        miniapp_id.clone(),
        artifact,
        MiniAppReadyRelease {
            miniapp_id: miniapp_id.clone(),
            release,
            origin_operation_id: operation_id.clone(),
            origin: MiniAppReadyOrigin::Import,
            source_lineage: MiniAppSourceLineage::RuntimeOnly,
            matching_service_test_receipt: None,
            created_at_ms: now_ms,
        },
    )
    .unwrap()
}

async fn publish_repository(
    repository: &Arc<dyn PluginRuntimeRepository>,
    snapshot: &PluginRuntimeRepositorySnapshot,
    catalog_seed: &str,
    now_ms: i64,
) -> PluginRuntimeRepositorySnapshot {
    publish_repository_result(repository, snapshot, catalog_seed, now_ms)
        .await
        .unwrap()
}

async fn publish_repository_result(
    repository: &Arc<dyn PluginRuntimeRepository>,
    snapshot: &PluginRuntimeRepositorySnapshot,
    catalog_seed: &str,
    now_ms: i64,
) -> PluginRuntimePlatformResult<PluginRuntimeRepositorySnapshot> {
    let target = snapshot.root.ready_release()?.release_ref().clone();
    repository
        .commit_release(CommitPluginRuntimeRelease {
            expected: PluginRuntimeMutationExpectation::from_snapshot(snapshot),
            command: PluginRuntimeReleaseCommand::Publish(MiniAppPublishRequest {
                miniapp_id: snapshot.root.product.miniapp_id.clone(),
                expected: MiniAppPointerExpectation::from_state(&snapshot.root.product.pointers),
                target_ready_release: target,
                target_catalog_digest: digest(catalog_seed),
                authorization: MiniAppPublishAuthorization::ManualUser {
                    actor_id: "user-1".into(),
                },
            }),
            now_ms,
        })
        .await
}

async fn lifecycle_repository(
    repository: &Arc<dyn PluginRuntimeRepository>,
    snapshot: &PluginRuntimeRepositorySnapshot,
    command: PluginRuntimeLifecycleCommand,
    now_ms: i64,
) -> PluginRuntimeRepositorySnapshot {
    repository
        .commit_lifecycle(CommitPluginRuntimeLifecycle {
            expected: PluginRuntimeMutationExpectation::from_snapshot(snapshot),
            command,
            now_ms,
        })
        .await
        .unwrap()
}

fn artifact(seed: &str, with_service: bool) -> MiniAppReleaseArtifactV1 {
    let mut files = vec![
        release_file("ui/index.html", &format!("index-{seed}")),
        release_file("ui/app.js", &format!("ui-{seed}")),
    ];
    if with_service {
        files.push(release_file("service/main.mjs", "stable-service-module"));
    }
    let entrypoint = files
        .iter()
        .find(|file| file.normalized_relative_path == "ui/index.html")
        .unwrap();
    let service = with_service.then(|| {
        let module = files
            .iter()
            .find(|file| file.normalized_relative_path == "service/main.mjs")
            .unwrap();
        MiniAppServiceReleaseDescriptor {
            entrypoint: "service/main.mjs".into(),
            module_digest: module.digest.clone(),
            lifecycle: MiniAppServiceLifecycle::OnDemand,
            uses_files: true,
            uses_private_database: true,
            service_contract_digest: digest("service-contract"),
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime_requirements_digest: digest("runtime-requirements"),
        }
    });
    let config_schema = StrictJsonValue(serde_json::json!({
        "additionalProperties": false,
        "properties": {
            "workspace": {"type": "string"}
        },
        "type": "object"
    }));
    let credential_slots = with_service
        .then(|| CredentialSlotDeclaration {
            slot_key: CredentialSlotKey::from("api_key"),
            kind: CredentialSlotKind::SecretText,
            display_name: "API key".into(),
            required: true,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let resource_contract = MiniAppResourceContract {
        required_resource_kinds: with_service
            .then(|| ResourceKind::from("knowledge.base"))
            .into_iter()
            .collect(),
    };
    let manifest = MiniAppReleaseV1Manifest {
        schema_version: MINIAPP_M1_SCHEMA_VERSION.into(),
        build_profile: JavaScriptBuildProfile::MiniAppReleaseV1,
        build_profile_version: MINIAPP_RELEASE_PROFILE_VERSION.into(),
        display: LocalizedMetadata {
            name: "Example".into(),
            description: "Example Plugin".into(),
            localized_names: BTreeMap::new(),
            localized_descriptions: BTreeMap::new(),
        },
        ui: Some(nomifun_agent_contracts::MiniAppUiReleaseDescriptor {
            entrypoint: "ui/index.html".into(),
            entrypoint_digest: entrypoint.digest.clone(),
            ui_tree_digest: canonical_ui_tree_digest(&files).unwrap(),
        }),
        service,
        dependency_lock_digest: digest("dependency-lock"),
        dependency_graph_digest: digest("dependency-graph"),
        config_schema_digest: digest_payload(&config_schema.0).unwrap(),
        config_schema,
        credential_slots_digest: digest_payload(&credential_slots).unwrap(),
        credential_slots,
        resource_contract_digest: digest_payload(&resource_contract).unwrap(),
        resource_contract,
        schemas: BTreeMap::new(),
        bridge_contract_digest: digest("bridge-contract"),
        contribution_package: PackageRef {
            id: PackageId::from("miniapp.example.release"),
            version: VersionString::from("1.0.0"),
        },
        contributions: PackageContributions::default(),
        migrations: Vec::new(),
    };
    MiniAppReleaseArtifactV1::new(ArtifactId::from(format!("artifact-{seed}")), manifest, files)
        .unwrap()
}

fn release_file(path: &str, seed: &str) -> MiniAppReleaseFile {
    MiniAppReleaseFile {
        normalized_relative_path: path.into(),
        digest: digest(seed),
        size_bytes: seed.len() as u64,
    }
}

fn digest(seed: &str) -> DigestHex {
    digest_bytes(seed.as_bytes())
}

fn storage(miniapp_id: &MiniAppId, kind: PluginRuntimeKind) -> MiniAppServiceStorageDescriptor {
    MiniAppServiceStorageDescriptor {
        kv: MiniAppKvHandleDescriptor {
            handle_id: MiniAppKvHandleId::from(format!("kv-{}", miniapp_id.as_ref())),
            miniapp_id: miniapp_id.clone(),
            namespace_revision: 1,
        },
        files_dir: (kind == PluginRuntimeKind::Service).then(|| MiniAppFilesDirDescriptor {
            handle_id: MiniAppFilesHandleId::from(format!("files-{}", miniapp_id.as_ref())),
            miniapp_id: miniapp_id.clone(),
            absolute_path: format!(
                "C:\\NomiFun\\miniapps\\{}\\files",
                miniapp_id.as_ref()
            ),
        }),
        private_database: (kind == PluginRuntimeKind::Service).then(|| {
            MiniAppPrivateDatabaseDescriptor {
                handle_id: MiniAppDatabaseHandleId::from(format!("db-{}", miniapp_id.as_ref())),
                miniapp_id: miniapp_id.clone(),
                schema_epoch: 1,
                migration_ledger_digest: digest("empty-migration-ledger"),
            }
        }),
    }
}

fn service_spec(
    snapshot: &PluginRuntimeRepositorySnapshot,
    release: &StoredPluginRuntimeRelease,
    epoch: u64,
) -> ResolvedMiniAppServiceSpec {
    let manifest = &release.artifact.manifest.payload;
    let service = manifest.service.as_ref().expect("Service Release");
    ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
        miniapp_id: snapshot.root.product.miniapp_id.clone(),
        release: release.release_ref().clone(),
        active_release_epoch: epoch,
        service_module_digest: service.module_digest.clone(),
        lifecycle: service.lifecycle,
        host_protocol_version: service.host_protocol_version.clone(),
        sdk_contract_version: service.sdk_contract_version.clone(),
        runtime: MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("runtime-1"),
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            runtime_executable_digest: digest("node-runtime"),
            node_version: VersionString::from("24.1.0"),
        },
        config_schema_digest: manifest.config_schema_digest.clone(),
        config_snapshot_digest: digest("config-snapshot"),
        credential_slots_digest: manifest.credential_slots_digest.clone(),
        resource_contract_digest: manifest.resource_contract_digest.clone(),
        resource_bindings_digest: digest("resource-bindings"),
        runtime_requirements_digest: service.runtime_requirements_digest.clone(),
        bridge_contract_digest: manifest.bridge_contract_digest.clone(),
        contribution_set_digest: manifest.contribution_set_digest().unwrap(),
        storage: snapshot.root.storage.clone(),
    })
    .unwrap()
}

fn non_ui_fingerprint(
    release: &StoredPluginRuntimeRelease,
    spec: &ResolvedMiniAppServiceSpec,
) -> MiniAppNonUiReleaseFingerprint {
    let manifest = &release.artifact.manifest.payload;
    MiniAppNonUiReleaseFingerprint {
        manifest_without_ui_digest: digest("manifest-without-ui"),
        service_run_key: Some(spec.service_run_key.clone()),
        migration_set_digest: manifest.migration_set_digest().unwrap(),
        contribution_set_digest: manifest.contribution_set_digest().unwrap(),
        bridge_contract_digest: manifest.bridge_contract_digest.clone(),
        config_schema_digest: manifest.config_schema_digest.clone(),
        credential_slots_digest: manifest.credential_slots_digest.clone(),
        resource_contract_digest: manifest.resource_contract_digest.clone(),
        runtime_requirements_digest: manifest
            .service
            .as_ref()
            .expect("Service Release")
            .runtime_requirements_digest
            .clone(),
        dependency_lock_digest: manifest.dependency_lock_digest.clone(),
    }
}
