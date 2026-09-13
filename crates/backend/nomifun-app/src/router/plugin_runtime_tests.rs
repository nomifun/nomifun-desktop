use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use axum::{Extension, Router};
use http_body_util::BodyExt;
use nomifun_agent_contracts::{
    DigestHex, MiniAppBridgeCallId, MiniAppId, ResolvedMiniAppServiceSpec,
    StrictJsonValue,
};
use nomifun_api_types::{
    ApiResponse, BuildPluginRuntimeRequest, CreatePluginRuntimeProjectRequest,
    DeletePluginRuntimeRequest,
    DurableOperationKindDto, DurableOperationOwnerDto,
    DurableOperationStateDto, DurableOperationSummaryDto, ErrorResponse,
    PluginRuntimeKindDto, PluginRuntimeLibraryResponseDto, PluginRuntimeLifecycleDto,
    PluginRuntimeServiceHealthDto, PluginRuntimeSourceFileDto, PluginRuntimeSurfaceLaunchDescriptorDto,
    PluginRuntimeWorkshopDto, PublishPluginRuntimeRequest, RestorePluginRuntimeRequest,
    RetryPluginRuntimeDeleteRequest, SetPluginRuntimeEnabledRequest, TestPluginRuntimeReleaseRequest,
    TrashPluginRuntimeRequest,
};
use nomifun_auth::CurrentUser;
use nomifun_common::{AppError, UserId};
use nomifun_db::{
    DbError, IMiniAppM1Repository, MiniAppM1ManagedSourceLineage,
    SqliteMiniAppM1Repository, StartMiniAppM1BuildOperationParams,
    init_database_memory, installation_owner_id,
};
use nomifun_plugin_platform::runtime::{
    PluginRuntimeCallCancellation, PluginRuntimeM1ApplicationService,
    PluginRuntimePlatformError, PluginRuntimePlatformResult, PluginRuntimeServiceHostState,
    PluginRuntimeServiceRuntimeBinding, PluginRuntimeServiceSpecInput,
};
use serde::de::DeserializeOwned;
use serde_json::json;
use tower::ServiceExt;

use super::{
    PluginRuntimeM1RouterState, application_error, miniapp_m1_read_routes,
    miniapp_m1_surface_routes, miniapp_m1_write_routes,
};

const MISMATCHED_MINIAPP_ID: &str =
    "0190f5fe-7c00-7000-8000-000000000993";

#[tokio::test]
async fn split_routes_preserve_owner_scope_and_api_envelopes() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let state = PluginRuntimeM1RouterState::new(Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(
            repository,
            store_root.path(),
        )
        .unwrap(),
    ));
    let owner = current_user(&owner_id, "owner");

    let read =
        miniapp_m1_read_routes(state.clone()).layer(Extension(owner.clone()));
    let write =
        miniapp_m1_write_routes(state.clone()).layer(Extension(owner));

    let response = send(&read, Method::POST, "/api/plugins/runtimes/projects", None)
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(&write, Method::GET, "/api/plugins/runtimes", None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(
        &write,
        Method::POST,
        "/api/plugins/runtimes/projects",
        Some(json!({
            "expected_library_revision": 0,
            "display_name": "Route M1",
            "description": "owner-scoped route",
            "kind": "ui_only"
        })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let created: PluginRuntimeWorkshopDto = response_data(response).await;
    assert_eq!(created.miniapp.display_name, "Route M1");
    assert_eq!(created.miniapp.kind, PluginRuntimeKindDto::UiOnly);

    let build_path = format!(
        "/api/plugins/runtimes/{}/build",
        created.miniapp.miniapp_id
    );
    let build_body = build_request(&created);
    let response = send(
        &read,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&build_body).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let test_path = format!(
        "/api/plugins/runtimes/{}/test",
        created.miniapp.miniapp_id
    );
    let test_request = TestPluginRuntimeReleaseRequest {
        miniapp_id: created.miniapp.miniapp_id.clone(),
        expected_product_revision: created.miniapp.product_revision,
        expected_pointer_revision: created.miniapp.releases.pointer_revision,
        project_id: created.project_id.clone(),
        expected_project_revision: created.project_revision,
        expected_build_generation: created.build_generation,
        release_id: MISMATCHED_MINIAPP_ID.to_owned(),
        expected_release_digest: "a".repeat(64),
        expected_config_revision: created.config.config_revision,
        expected_credential_bindings_revision: created.credential_bindings_revision,
        resolved_test_input_digest: "b".repeat(64),
    };
    let response = send(
        &read,
        Method::POST,
        &test_path,
        Some(serde_json::to_value(&test_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send(
        &write,
        Method::POST,
        &test_path,
        Some(
            serde_json::to_value(TestPluginRuntimeReleaseRequest {
                miniapp_id: MISMATCHED_MINIAPP_ID.to_owned(),
                ..test_request.clone()
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = send(
        &write,
        Method::POST,
        &test_path,
        Some(serde_json::to_value(test_request).unwrap()),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "UI-only MiniApps must not enter the Service Test application path"
    );

    let operation_id = "0190f5fe-7c00-7000-8000-000000000991";
    let cancel_path = format!(
        "/api/plugins/runtimes/{}/operations/{operation_id}/cancel",
        created.miniapp.miniapp_id
    );
    let response = send(
        &read,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 4 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = send(&read, Method::GET, "/api/plugins/runtimes", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let library: PluginRuntimeLibraryResponseDto = response_data(response).await;
    assert_eq!(library.library_revision, 1);
    assert_eq!(library.miniapps.len(), 1);
    assert_eq!(
        library.miniapps[0].miniapp_id,
        created.miniapp.miniapp_id
    );

    let workshop_path = format!(
        "/api/plugins/runtimes/{}/workshop",
        created.miniapp.miniapp_id
    );
    let response = send(&read, Method::GET, &workshop_path, None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let workshop: PluginRuntimeWorkshopDto = response_data(response).await;
    assert_eq!(workshop, created);

    let surface_path = format!(
        "/api/plugins/runtimes/{}/surface/open",
        created.miniapp.miniapp_id
    );
    let response = send(
        &read,
        Method::POST,
        &surface_path,
        Some(json!({ "plugin_id": created.miniapp.miniapp_id })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "Surface capability signing must not be mounted in read routes"
    );
    let response = send(&write, Method::GET, &surface_path, None).await;
    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "Surface capability signing must be POST-only"
    );
    let response = send(
        &write,
        Method::GET,
        &format!("/api/plugins/runtimes/{}/surface", created.miniapp.miniapp_id),
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "the legacy GET Surface signer must not remain mounted"
    );
    let response = send(
        &write,
        Method::POST,
        &surface_path,
        Some(json!({
            "plugin_id": "0190f5fe-7c00-7000-8000-000000000993"
        })),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "Surface open must reject a body identity different from the route"
    );

    let other = CurrentUser {
        id: UserId::new(),
        username: "other".to_owned(),
    };
    nomifun_db::sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(other.id.as_str())
    .bind(other.id.as_str())
    .execute(database.pool())
    .await
    .unwrap();
    let other_read =
        miniapp_m1_read_routes(state).layer(Extension(other));
    let response =
        send(&other_read, Method::GET, "/api/plugins/runtimes", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let library: PluginRuntimeLibraryResponseDto = response_data(response).await;
    assert_eq!(library.library_revision, 0);
    assert!(library.miniapps.is_empty());

    let response =
        send(&other_read, Method::GET, &workshop_path, None).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "NOT_FOUND");
}

#[tokio::test]
async fn source_routes_are_local_mutations_with_exact_project_cas() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let state = PluginRuntimeM1RouterState::new(Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(repository, store_root.path()).unwrap(),
    ));
    let owner = current_user(&owner_id, "owner");
    let read = miniapp_m1_read_routes(state.clone()).layer(Extension(owner.clone()));
    let write = miniapp_m1_write_routes(state).layer(Extension(owner));
    let response = send(
        &write,
        Method::POST,
        "/api/plugins/runtimes/projects",
        Some(json!({
            "expected_library_revision": 0,
            "display_name": "Editable Route",
            "kind": "ui_only"
        })),
    )
    .await;
    let created: PluginRuntimeWorkshopDto = response_data(response).await;
    let source_path = format!(
        "/api/plugins/runtimes/{}/source/files/ui%2Findex.html",
        created.miniapp.miniapp_id
    );
    assert_eq!(
        send(&read, Method::GET, &source_path, None).await.status(),
        StatusCode::NOT_FOUND,
        "Source text must remain behind the local-product route group"
    );
    let source_response = send(&write, Method::GET, &source_path, None).await;
    assert_eq!(source_response.status(), StatusCode::OK);
    let source: PluginRuntimeSourceFileDto = response_data(source_response).await;
    assert_eq!(source.path, "ui/index.html");
    assert!(!source.content.is_empty());
    assert_eq!(source.build_generation, created.build_generation);
    assert_eq!(
        Some(source.source_snapshot_digest.as_str()),
        created.source_snapshot_digest.as_deref()
    );

    let edit_path = format!(
        "/api/plugins/runtimes/{}/source/edit",
        created.miniapp.miniapp_id
    );
    let request = json!({
        "plugin_id": created.miniapp.miniapp_id.clone(),
        "expected_product_revision": created.miniapp.product_revision,
        "project_id": created.project_id.clone(),
        "expected_project_revision": created.project_revision,
        "expected_build_generation": created.build_generation,
        "expected_source_snapshot_digest": source.source_snapshot_digest,
        "path": source.path,
        "content": "<!doctype html><main>edited through route</main>"
    });
    let edited_response = send(&write, Method::POST, &edit_path, Some(request.clone())).await;
    assert_eq!(edited_response.status(), StatusCode::OK);
    let edited: PluginRuntimeWorkshopDto = response_data(edited_response).await;
    assert_eq!(edited.project_revision, created.project_revision + 1);
    assert_eq!(edited.build_generation, created.build_generation + 1);
    assert_ne!(edited.source_snapshot_digest, created.source_snapshot_digest);

    let stale_response = send(&write, Method::POST, &edit_path, Some(request)).await;
    assert_eq!(stale_response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn build_and_cancel_routes_use_real_application_state() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(
            repository.clone(),
            store_root.path(),
        )
        .unwrap(),
    );
    let workshop = application
        .create(
            &owner_id,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: "Build Route".to_owned(),
                description: Some("exact request forwarding".to_owned()),
                kind: PluginRuntimeKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let miniapp_id = workshop.miniapp.miniapp_id.clone();
    let state = PluginRuntimeM1RouterState::new(application);
    let write = miniapp_m1_write_routes(state)
        .layer(Extension(current_user(&owner_id, "owner")));

    let build_request = build_request(&workshop);
    let build_path = format!("/api/plugins/runtimes/{miniapp_id}/build");
    let response = send(
        &write,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&build_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let built: PluginRuntimeWorkshopDto = response_data(response).await;
    let ready = built.ready.as_ref().expect("Build must commit Ready");
    assert_eq!(ready.project_build_generation, workshop.build_generation);
    assert!(ready.created_at_ms > 0);
    assert_eq!(
        built.miniapp.releases.ready.as_ref(),
        Some(&ready.release)
    );
    let completed = repository
        .list_build_operations(&owner_id, &miniapp_id)
        .await
        .unwrap();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].state, "succeeded");
    assert_eq!(completed[0].owner_kind, "miniapp");
    assert_eq!(completed[0].owner_id, miniapp_id);

    let mismatched_request = BuildPluginRuntimeRequest {
        miniapp_id: "0190f5fe-7c00-7000-8000-000000000993".to_owned(),
        ..build_request.clone()
    };
    let response = send(
        &write,
        Method::POST,
        &build_path,
        Some(serde_json::to_value(&mismatched_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "BAD_REQUEST");
    assert_eq!(
        repository
            .list_build_operations(&owner_id, &miniapp_id)
            .await
            .unwrap()
            .len(),
        1,
        "path/body identity mismatch must fail before starting an operation"
    );

    let snapshot = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .expect("built Plugin");
    let operation_id = "0190f5fe-7c00-7000-8000-000000000992";
    let started_at_ms = nomifun_common::now_ms()
        .max(snapshot.product.created_at)
        .max(snapshot.project.updated_at)
        .max(1);
    repository
        .start_build_operation(&StartMiniAppM1BuildOperationParams {
            owner_user_id: owner_id.clone(),
            miniapp_id: miniapp_id.clone(),
            project_id: snapshot.project.project_id.clone(),
            operation_id: operation_id.to_owned(),
            expected_project_revision: snapshot.project.project_revision,
            expected_source: MiniAppM1ManagedSourceLineage {
                managed_source_path: snapshot
                    .project
                    .managed_source_path
                    .clone()
                    .expect("managed source path"),
                source_head_digest: snapshot
                    .project
                    .source_head_digest
                    .clone()
                    .expect("source head digest"),
                dependency_lock_digest: snapshot
                    .project
                    .dependency_lock_digest
                    .clone()
                    .expect("dependency lock digest"),
                build_profile_version: snapshot
                    .project
                    .build_profile_version
                    .clone()
                    .expect("build profile version"),
                build_generation: snapshot.project.build_generation,
            },
            bounded_log_tail: vec!["Build started for route cancellation".to_owned()],
            started_at_ms,
        })
        .await
        .unwrap();
    let cancel_path = format!(
        "/api/plugins/runtimes/{miniapp_id}/operations/{operation_id}/cancel"
    );
    let response = send(
        &write,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 1 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let canceled: DurableOperationSummaryDto = response_data(response).await;
    assert_eq!(canceled.operation_id, operation_id);
    assert_eq!(canceled.operation_revision, 2);
    assert_eq!(canceled.kind, DurableOperationKindDto::Build);
    assert_eq!(
        canceled.owner,
        DurableOperationOwnerDto::Miniapp {
            miniapp_id: miniapp_id.clone(),
        }
    );
    assert_eq!(canceled.state, DurableOperationStateDto::Canceled);
    assert!(!canceled.cancelable);
    assert!(canceled.completed_at_ms.is_some());
    let persisted = repository
        .get_build_operation(&owner_id, &miniapp_id, operation_id)
        .await
        .unwrap()
        .expect("canceled operation");
    assert_eq!(persisted.state, "canceled");

    let response = send(
        &write,
        Method::POST,
        &cancel_path,
        Some(json!({ "expected_operation_revision": 1 })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, "CONFLICT");
}

#[tokio::test]
async fn lifecycle_routes_enforce_identity_owner_state_and_surface_revocation() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let other = insert_other_user(&database, "lifecycle-other").await;
    let repository = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(
            repository.clone(),
            store_root.path(),
        )
        .unwrap(),
    );
    let enabled = create_enabled_ui_miniapp(
        application.as_ref(),
        &owner_id,
        "Lifecycle Routes",
    )
    .await;
    let miniapp_id = enabled.miniapp.miniapp_id.clone();
    let active = enabled
        .miniapp
        .releases
        .active
        .clone()
        .expect("enabled Plugin must retain its Active Release");
    let persisted = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .expect("enabled Plugin");
    let source_path = store_root
        .path()
        .join("source")
        .join(
            persisted
                .project
                .managed_source_path
                .as_ref()
                .expect("managed source path"),
        );
    let release_managed_path: String = nomifun_db::sqlx::query_scalar(
        "SELECT artifact.managed_path
         FROM miniapp_release_artifacts artifact
         JOIN miniapp_releases release
           ON release.owner_user_id = artifact.owner_user_id
          AND release.artifact_id = artifact.artifact_id
         WHERE release.owner_user_id = ? AND release.miniapp_id = ?
         LIMIT 1",
    )
    .bind(&owner_id)
    .bind(&miniapp_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let release_path = store_root
        .path()
        .join("release")
        .join(release_managed_path);
    assert!(source_path.is_dir());
    assert!(release_path.is_dir());

    let state = PluginRuntimeM1RouterState::new(application);
    let owner = current_user(&owner_id, "owner");
    let owner_read =
        miniapp_m1_read_routes(state.clone()).layer(Extension(owner.clone()));
    let owner_write =
        miniapp_m1_write_routes(state.clone()).layer(Extension(owner));
    let other_write =
        miniapp_m1_write_routes(state.clone()).layer(Extension(other));
    let surface_routes = miniapp_m1_surface_routes(state);

    let open_path = format!("/api/plugins/runtimes/{miniapp_id}/surface/open");
    let response = send(
        &owner_write,
        Method::POST,
        &open_path,
        Some(json!({ "plugin_id": miniapp_id })),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let surface: PluginRuntimeSurfaceLaunchDescriptorDto =
        response_data(response).await;
    let asset_path = format!(
        "/api/plugins/runtimes/{miniapp_id}/surface/assets/{}/{}/{}/{}",
        surface.surface_capability,
        surface.active_release_epoch,
        surface.expected_release_digest,
        surface.ui_entrypoint
    );
    let response =
        send(&surface_routes, Method::GET, &asset_path, None).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the issued Surface capability must work before Trash"
    );

    let trash_path = format!("/api/plugins/runtimes/{miniapp_id}/trash");
    let trash_request = TrashPluginRuntimeRequest {
        miniapp_id: miniapp_id.clone(),
        expected_product_revision: enabled.miniapp.product_revision,
        expected_pointer_revision: enabled.miniapp.releases.pointer_revision,
        expected_active_release_digest: Some(active.release_digest.clone()),
    };
    let mismatched_trash = TrashPluginRuntimeRequest {
        miniapp_id: MISMATCHED_MINIAPP_ID.to_owned(),
        ..trash_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &trash_path,
            Some(serde_json::to_value(mismatched_trash).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    assert_api_error(
        send(
            &other_write,
            Method::POST,
            &trash_path,
            Some(serde_json::to_value(&trash_request).unwrap()),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    assert_eq!(
        send(&surface_routes, Method::GET, &asset_path, None)
            .await
            .status(),
        StatusCode::OK,
        "rejected Trash requests must not revoke the owner's Surface"
    );

    let response = send(
        &owner_write,
        Method::POST,
        &trash_path,
        Some(serde_json::to_value(trash_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let trashed: PluginRuntimeWorkshopDto = response_data(response).await;
    assert_eq!(trashed.miniapp.lifecycle, PluginRuntimeLifecycleDto::Trashed);
    assert_eq!(
        trashed.miniapp.product_revision,
        enabled.miniapp.product_revision + 1
    );
    assert_eq!(
        trashed.miniapp.releases.pointer_revision,
        enabled.miniapp.releases.pointer_revision
    );
    assert!(!trashed.miniapp.surface_available);
    assert_api_error(
        send(&surface_routes, Method::GET, &asset_path, None).await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;

    let restore_path = format!("/api/plugins/runtimes/{miniapp_id}/restore");
    let restore_request = RestorePluginRuntimeRequest {
        miniapp_id: miniapp_id.clone(),
        expected_product_revision: trashed.miniapp.product_revision,
        expected_lifecycle: PluginRuntimeLifecycleDto::Trashed,
        expected_pointer_revision: trashed.miniapp.releases.pointer_revision,
    };
    let mismatched_restore = RestorePluginRuntimeRequest {
        miniapp_id: MISMATCHED_MINIAPP_ID.to_owned(),
        ..restore_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &restore_path,
            Some(serde_json::to_value(mismatched_restore).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    assert_api_error(
        send(
            &other_write,
            Method::POST,
            &restore_path,
            Some(serde_json::to_value(&restore_request).unwrap()),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    let invalid_restore = RestorePluginRuntimeRequest {
        expected_lifecycle: PluginRuntimeLifecycleDto::Disabled,
        ..restore_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &restore_path,
            Some(serde_json::to_value(invalid_restore).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;

    let response = send(
        &owner_write,
        Method::POST,
        &restore_path,
        Some(serde_json::to_value(restore_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let restored: PluginRuntimeWorkshopDto = response_data(response).await;
    assert_eq!(
        restored.miniapp.lifecycle,
        PluginRuntimeLifecycleDto::Disabled,
        "Restore must never reactivate an enabled Plugin"
    );
    assert_eq!(
        restored.miniapp.product_revision,
        trashed.miniapp.product_revision + 1
    );
    assert_eq!(
        restored.miniapp.releases.pointer_revision,
        trashed.miniapp.releases.pointer_revision
    );
    assert!(!restored.miniapp.surface_available);
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &open_path,
            Some(json!({ "plugin_id": miniapp_id })),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;

    let response = send(
        &owner_write,
        Method::POST,
        &trash_path,
        Some(
            serde_json::to_value(TrashPluginRuntimeRequest {
                miniapp_id: miniapp_id.clone(),
                expected_product_revision: restored.miniapp.product_revision,
                expected_pointer_revision: restored
                    .miniapp
                    .releases
                    .pointer_revision,
                expected_active_release_digest: Some(
                    active.release_digest.clone(),
                ),
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let trashed_again: PluginRuntimeWorkshopDto = response_data(response).await;

    let delete_path = format!("/api/plugins/runtimes/{miniapp_id}/delete");
    let delete_request = DeletePluginRuntimeRequest {
        miniapp_id: miniapp_id.clone(),
        expected_product_revision: trashed_again.miniapp.product_revision,
        expected_lifecycle: PluginRuntimeLifecycleDto::Trashed,
        expected_pointer_revision: trashed_again
            .miniapp
            .releases
            .pointer_revision,
        expected_active_release_digest: Some(active.release_digest),
    };
    let mismatched_delete = DeletePluginRuntimeRequest {
        miniapp_id: MISMATCHED_MINIAPP_ID.to_owned(),
        ..delete_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &delete_path,
            Some(serde_json::to_value(mismatched_delete).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    assert_api_error(
        send(
            &other_write,
            Method::POST,
            &delete_path,
            Some(serde_json::to_value(&delete_request).unwrap()),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    let invalid_delete = DeletePluginRuntimeRequest {
        expected_lifecycle: PluginRuntimeLifecycleDto::Disabled,
        ..delete_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &delete_path,
            Some(serde_json::to_value(invalid_delete).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;

    let response = send(
        &owner_write,
        Method::POST,
        &delete_path,
        Some(serde_json::to_value(delete_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let deleted_library: PluginRuntimeLibraryResponseDto =
        response_data(response).await;
    assert!(
        deleted_library
            .miniapps
            .iter()
            .all(|miniapp| miniapp.miniapp_id != miniapp_id)
    );
    assert!(repository.get(&owner_id, &miniapp_id).await.unwrap().is_none());
    assert!(!source_path.exists(), "Delete must purge managed Source");
    assert!(!release_path.exists(), "Delete must purge managed Release");

    let response = send(&owner_read, Method::GET, "/api/plugins/runtimes", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reloaded_library: PluginRuntimeLibraryResponseDto =
        response_data(response).await;
    assert_eq!(reloaded_library, deleted_library);
    assert_api_error(
        send(
            &owner_read,
            Method::GET,
            &format!("/api/plugins/runtimes/{miniapp_id}/workshop"),
            None,
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
}

#[tokio::test]
async fn trash_route_stops_the_exact_service_runtime_identity() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let repository: Arc<dyn IMiniAppM1Repository> = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(
            repository,
            store_root.path(),
        )
        .unwrap(),
    );
    let runtime = Arc::new(LifecycleTestRuntime::new(false));
    application.install_service_runtime(runtime.clone()).await;
    let created = application
        .create(
            &owner_id,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: "Service Trash".to_owned(),
                description: None,
                kind: PluginRuntimeKindDto::Service,
            },
        )
        .await
        .unwrap();
    let miniapp_id = created.miniapp.miniapp_id.clone();
    let write = miniapp_m1_write_routes(PluginRuntimeM1RouterState::new(application))
        .layer(Extension(current_user(&owner_id, "owner")));

    let response = send(
        &write,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/trash"),
        Some(
            serde_json::to_value(TrashPluginRuntimeRequest {
                miniapp_id: miniapp_id.clone(),
                expected_product_revision: created.miniapp.product_revision,
                expected_pointer_revision: created
                    .miniapp
                    .releases
                    .pointer_revision,
                expected_active_release_digest: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let trashed: PluginRuntimeWorkshopDto = response_data(response).await;
    assert_eq!(trashed.miniapp.lifecycle, PluginRuntimeLifecycleDto::Trashed);
    assert_eq!(
        trashed.miniapp.service_health,
        PluginRuntimeServiceHealthDto::Stopped
    );
    assert_eq!(runtime.stopped_ids(), vec![miniapp_id]);
}

#[tokio::test]
async fn retry_delete_route_recovers_failed_cleanup_with_exact_owner_and_revision() {
    let database = init_database_memory().await.unwrap();
    let owner_id = installation_owner_id(database.pool()).await.unwrap();
    let other = insert_other_user(&database, "retry-other").await;
    let repository = Arc::new(
        SqliteMiniAppM1Repository::new(database.pool().clone()),
    );
    let store_root = tempfile::tempdir().unwrap();
    let application = Arc::new(
        PluginRuntimeM1ApplicationService::new_with_root(
            repository.clone(),
            store_root.path(),
        )
        .unwrap(),
    );
    let runtime = Arc::new(LifecycleTestRuntime::new(true));
    application.install_service_runtime(runtime.clone()).await;
    let created = application
        .create(
            &owner_id,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: "Retry Delete".to_owned(),
                description: None,
                kind: PluginRuntimeKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = application
        .build(&owner_id, build_request(&created))
        .await
        .unwrap();
    let miniapp_id = built.miniapp.miniapp_id.clone();
    let persisted = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .expect("built Plugin");
    let source_path = store_root
        .path()
        .join("source")
        .join(
            persisted
                .project
                .managed_source_path
                .as_ref()
                .expect("managed source path"),
        );
    let release_managed_path: String = nomifun_db::sqlx::query_scalar(
        "SELECT artifact.managed_path
         FROM miniapp_release_artifacts artifact
         JOIN miniapp_releases release
           ON release.owner_user_id = artifact.owner_user_id
          AND release.artifact_id = artifact.artifact_id
         WHERE release.owner_user_id = ? AND release.miniapp_id = ?
         LIMIT 1",
    )
    .bind(&owner_id)
    .bind(&miniapp_id)
    .fetch_one(database.pool())
    .await
    .unwrap();
    let release_path = store_root
        .path()
        .join("release")
        .join(release_managed_path);

    let state = PluginRuntimeM1RouterState::new(application);
    let owner_write = miniapp_m1_write_routes(state.clone())
        .layer(Extension(current_user(&owner_id, "owner")));
    let owner_read = miniapp_m1_read_routes(state.clone())
        .layer(Extension(current_user(&owner_id, "owner")));
    let other_write =
        miniapp_m1_write_routes(state).layer(Extension(other));

    let response = send(
        &owner_write,
        Method::POST,
        &format!("/api/plugins/runtimes/{miniapp_id}/trash"),
        Some(
            serde_json::to_value(TrashPluginRuntimeRequest {
                miniapp_id: miniapp_id.clone(),
                expected_product_revision: built.miniapp.product_revision,
                expected_pointer_revision: built
                    .miniapp
                    .releases
                    .pointer_revision,
                expected_active_release_digest: None,
            })
            .unwrap(),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let trashed: PluginRuntimeWorkshopDto = response_data(response).await;

    let delete_path = format!("/api/plugins/runtimes/{miniapp_id}/delete");
    let delete_request = DeletePluginRuntimeRequest {
        miniapp_id: miniapp_id.clone(),
        expected_product_revision: trashed.miniapp.product_revision,
        expected_lifecycle: PluginRuntimeLifecycleDto::Trashed,
        expected_pointer_revision: trashed.miniapp.releases.pointer_revision,
        expected_active_release_digest: None,
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &delete_path,
            Some(serde_json::to_value(delete_request).unwrap()),
        )
        .await,
        StatusCode::INTERNAL_SERVER_ERROR,
        "INTERNAL_ERROR",
    )
    .await;
    assert_eq!(runtime.purge_calls(), 1);
    assert!(source_path.is_dir());
    assert!(release_path.is_dir());

    let deleting = repository
        .get(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .expect("failed Delete must retain the deleting Product");
    assert_eq!(deleting.product.lifecycle, "deleting");
    let failed = repository
        .list_miniapp_operations(&owner_id, &miniapp_id)
        .await
        .unwrap()
        .into_iter()
        .find(|operation| operation.kind == "miniapp_permanent_delete")
        .expect("failed permanent Delete operation");
    assert_eq!(failed.state, "failed");
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("miniapp_delete_cleanup_failed")
    );
    assert!(failed.finished_at_ms.is_some());

    let retry_path =
        format!("/api/plugins/runtimes/{miniapp_id}/delete/retry");
    let retry_request = RetryPluginRuntimeDeleteRequest {
        miniapp_id: miniapp_id.clone(),
        failed_operation_id: failed.operation_id.clone(),
        expected_operation_revision: 2,
    };
    let mismatched_retry = RetryPluginRuntimeDeleteRequest {
        miniapp_id: MISMATCHED_MINIAPP_ID.to_owned(),
        ..retry_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &retry_path,
            Some(serde_json::to_value(mismatched_retry).unwrap()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BAD_REQUEST",
    )
    .await;
    assert_api_error(
        send(
            &other_write,
            Method::POST,
            &retry_path,
            Some(serde_json::to_value(&retry_request).unwrap()),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NOT_FOUND",
    )
    .await;
    let stale_retry = RetryPluginRuntimeDeleteRequest {
        expected_operation_revision: 1,
        ..retry_request.clone()
    };
    assert_api_error(
        send(
            &owner_write,
            Method::POST,
            &retry_path,
            Some(serde_json::to_value(stale_retry).unwrap()),
        )
        .await,
        StatusCode::CONFLICT,
        "CONFLICT",
    )
    .await;

    let response = send(
        &owner_write,
        Method::POST,
        &retry_path,
        Some(serde_json::to_value(retry_request).unwrap()),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let library: PluginRuntimeLibraryResponseDto = response_data(response).await;
    assert!(
        library
            .miniapps
            .iter()
            .all(|miniapp| miniapp.miniapp_id != miniapp_id)
    );
    assert_eq!(runtime.purge_calls(), 2);
    assert!(!source_path.exists());
    assert!(!release_path.exists());
    assert!(repository.get(&owner_id, &miniapp_id).await.unwrap().is_none());

    let deletion_states: Vec<String> = nomifun_db::sqlx::query_scalar(
        "SELECT state FROM product_operations
         WHERE owner_kind = 'miniapp' AND owner_id = ?
           AND kind = 'miniapp_permanent_delete'",
    )
    .bind(&miniapp_id)
    .fetch_all(database.pool())
    .await
    .unwrap();
    assert_eq!(deletion_states.len(), 2);
    assert_eq!(
        deletion_states
            .iter()
            .filter(|state| state.as_str() == "failed")
            .count(),
        1
    );
    assert_eq!(
        deletion_states
            .iter()
            .filter(|state| state.as_str() == "succeeded")
            .count(),
        1
    );

    let response = send(&owner_read, Method::GET, "/api/plugins/runtimes", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    let reloaded: PluginRuntimeLibraryResponseDto = response_data(response).await;
    assert_eq!(reloaded, library);
}

#[test]
fn application_errors_map_to_app_error_semantics() {
    let invalid = application_error(
        nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationError::Invalid(
            "bad revision".to_owned(),
        ),
    );
    assert!(matches!(invalid, AppError::BadRequest(_)));

    let not_found = application_error(
        nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationError::NotFound,
    );
    assert!(matches!(not_found, AppError::NotFound(_)));

    let runtime = application_error(
        nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationError::Runtime(
            "cleanup failed".to_owned(),
        ),
    );
    assert!(matches!(runtime, AppError::Internal(_)));

    let conflict = application_error(
        nomifun_plugin_platform::runtime::PluginRuntimeM1ApplicationError::Database(
            DbError::Conflict("stale library revision".to_owned()),
        ),
    );
    assert!(matches!(conflict, AppError::Conflict(_)));
}

struct LifecycleTestRuntime {
    fail_next_purge: AtomicBool,
    purge_calls: AtomicUsize,
    stopped_ids: StdMutex<Vec<String>>,
}

impl LifecycleTestRuntime {
    fn new(fail_next_purge: bool) -> Self {
        Self {
            fail_next_purge: AtomicBool::new(fail_next_purge),
            purge_calls: AtomicUsize::new(0),
            stopped_ids: StdMutex::new(Vec::new()),
        }
    }

    fn purge_calls(&self) -> usize {
        self.purge_calls.load(Ordering::SeqCst)
    }

    fn stopped_ids(&self) -> Vec<String> {
        self.stopped_ids.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl PluginRuntimeServiceRuntimeBinding for LifecycleTestRuntime {
    async fn purge_storage(
        &self,
        _owner_user_id: &str,
        _miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<()> {
        self.purge_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_purge.swap(false, Ordering::SeqCst) {
            return Err(PluginRuntimePlatformError::Runtime(
                "injected Plugin storage purge failure".to_owned(),
            ));
        }
        Ok(())
    }

    async fn resolve_spec(
        &self,
        _input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedMiniAppServiceSpec> {
        Err(PluginRuntimePlatformError::Runtime(
            "Service resolution is outside this lifecycle route fixture"
                .to_owned(),
        ))
    }

    async fn bind_active(
        &self,
        _spec: ResolvedMiniAppServiceSpec,
        _enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn start(
        &self,
        _spec: ResolvedMiniAppServiceSpec,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn invoke(
        &self,
        _spec: &ResolvedMiniAppServiceSpec,
        _call_id: MiniAppBridgeCallId,
        _method: String,
        _payload: StrictJsonValue,
        _cancellation: PluginRuntimeCallCancellation,
        _now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Service invocation is outside this lifecycle route fixture"
                .to_owned(),
        ))
    }

    async fn cancel(
        &self,
        _miniapp_id: &MiniAppId,
        _call_id: &MiniAppBridgeCallId,
    ) {
    }

    async fn stop(
        &self,
        miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<()> {
        self.stopped_ids
            .lock()
            .unwrap()
            .push(miniapp_id.as_ref().to_owned());
        Ok(())
    }

    async fn retry(
        &self,
        _miniapp_id: &MiniAppId,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn state(
        &self,
        _miniapp_id: &MiniAppId,
    ) -> Option<PluginRuntimeServiceHostState> {
        Some(PluginRuntimeServiceHostState::Stopped)
    }

    async fn maintain(&self, _now_ms: i64) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn register_module(
        &self,
        _miniapp_id: MiniAppId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }
}

async fn create_enabled_ui_miniapp(
    application: &PluginRuntimeM1ApplicationService,
    owner_id: &str,
    display_name: &str,
) -> PluginRuntimeWorkshopDto {
    let created = application
        .create(
            owner_id,
            CreatePluginRuntimeProjectRequest {
                expected_library_revision: 0,
                display_name: display_name.to_owned(),
                description: None,
                kind: PluginRuntimeKindDto::UiOnly,
            },
        )
        .await
        .unwrap();
    let built = application
        .build(owner_id, build_request(&created))
        .await
        .unwrap();
    let ready = built
        .ready
        .as_ref()
        .expect("Build must commit Ready")
        .release
        .clone();
    let published = application
        .publish(
            owner_id,
            PublishPluginRuntimeRequest {
                miniapp_id: built.miniapp.miniapp_id.clone(),
                expected_product_revision: built.miniapp.product_revision,
                expected_pointer_revision: built
                    .miniapp
                    .releases
                    .pointer_revision,
                expected_active_release_epoch: built
                    .miniapp
                    .releases
                    .active_release_epoch,
                ready_release_id: ready.release_id,
                expected_ready_release_digest: ready.release_digest,
                expected_active_release_digest: None,
                expected_service_test_receipt_id: None,
                acknowledge_test_warning: false,
            },
        )
        .await
        .unwrap();
    let active = published
        .miniapp
        .releases
        .active
        .as_ref()
        .expect("Publish must commit Active");
    application
        .set_enabled(
            owner_id,
            SetPluginRuntimeEnabledRequest {
                miniapp_id: published.miniapp.miniapp_id.clone(),
                expected_product_revision: published.miniapp.product_revision,
                expected_pointer_revision: published
                    .miniapp
                    .releases
                    .pointer_revision,
                expected_active_release_digest: Some(
                    active.release_digest.clone(),
                ),
                enabled: true,
            },
        )
        .await
        .unwrap()
}

async fn insert_other_user(
    database: &nomifun_db::Database,
    username: &str,
) -> CurrentUser {
    let user = CurrentUser {
        id: UserId::new(),
        username: username.to_owned(),
    };
    nomifun_db::sqlx::query(
        "INSERT INTO users (
            user_id, username, password_hash, jwt_secret, created_at, updated_at
         ) VALUES (?, ?, '', '', 1, 1)",
    )
    .bind(user.id.as_str())
    .bind(&user.username)
    .execute(database.pool())
    .await
    .unwrap();
    user
}

fn build_request(workshop: &PluginRuntimeWorkshopDto) -> BuildPluginRuntimeRequest {
    BuildPluginRuntimeRequest {
        miniapp_id: workshop.miniapp.miniapp_id.clone(),
        expected_product_revision: workshop.miniapp.product_revision,
        project_id: workshop.project_id.clone(),
        expected_project_revision: workshop.project_revision,
        expected_build_generation: workshop.build_generation,
        expected_source_snapshot_digest: workshop
            .source_snapshot_digest
            .clone()
            .expect("editable source snapshot"),
        expected_dependency_lock_digest: workshop
            .dependency_lock_digest
            .clone()
            .expect("dependency lock"),
        service_lifecycle: None,
    }
}

fn current_user(id: &str, username: &str) -> CurrentUser {
    CurrentUser {
        id: UserId::parse(id).unwrap(),
        username: username.to_owned(),
    }
}

async fn send(
    router: &Router,
    method: Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> axum::response::Response {
    let mut request = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&value).unwrap())
        }
        None => Body::empty(),
    };
    router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap()
}

async fn response_data<T>(response: axum::response::Response) -> T
where
    T: DeserializeOwned,
{
    let response: ApiResponse<T> = response_json(response).await;
    assert!(response.success);
    response.data.expect("success response must contain data")
}

async fn assert_api_error(
    response: axum::response::Response,
    expected_status: StatusCode,
    expected_code: &str,
) {
    assert_eq!(response.status(), expected_status);
    let error: ErrorResponse = response_json(response).await;
    assert_eq!(error.code, expected_code);
}

async fn response_json<T>(response: axum::response::Response) -> T
where
    T: DeserializeOwned,
{
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&body).unwrap()
}
