use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::Extension;
use http_body_util::BodyExt;
use nomifun_agent_contracts::{
    AgentBindingValue, PresetRevisionRef, PrincipalRef, RuntimeProfileKind, RuntimeTarget,
    VersionString,
    official_preset_seed_manifest_payload,
};
use nomifun_agent_control_plane::{
    CompilerReleaseInputs, ControlPlaneStore,
};
use nomifun_agent_kernel::{CompilerEnvironment, MaterializationPolicy};
use nomifun_agent_platform::{AgentPlatform, AgentPlatformConfig};
use nomifun_api_types::{
    AgentPresetEditorResponse, ApiResponse, CreateAgentSessionRequestDto,
    CreateAgentSessionResponseDto, ForkAgentSessionRequestDto, ForkAgentSessionResponseDto,
    ResolveAgentPresetPreviewResponse, SaveAgentPresetRevisionResponse,
};
use nomifun_auth::CurrentUser;
use nomifun_chat_model_broker::{
    ChatBrokerPort, ChatModelError, ChatModelErrorCode, ChatModelRequest,
    ChatModelStream, ChatRetryDirective,
};
use nomifun_codex_runtime::{
    CodexRuntimeSupervisor, FROZEN_PROTOCOL_VERSION,
};
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FreshV4Coordinator,
    canonical_schema_manifest_digest,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions,
    SqliteSynchronous,
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use uuid::Uuid;

const BUILD_IDENTITY: &str = "nomifun-app-agent-platform-e2e";

struct UnusedBroker;

#[async_trait]
impl ChatBrokerPort for UnusedBroker {
    async fn open_chat_stream(
        &self,
        _request: ChatModelRequest,
    ) -> Result<ChatModelStream, ChatModelError> {
        Err(ChatModelError::new(
            ChatModelErrorCode::AdapterUnavailable,
            "the route E2E does not execute a model turn",
            ChatRetryDirective::Never,
        ))
    }
}

#[tokio::test]
async fn canonical_agent_routes_use_the_fresh_v4_platform() {
    let directory = tempfile::tempdir().unwrap();
    let canonical_root = directory.path().join("data");
    let bootstrap = FreshV4Coordinator::default()
        .bootstrap(&canonical_root, BUILD_IDENTITY, &[])
        .await
        .unwrap();
    let pool = open_pool(
        &bootstrap.canonical_root.join(FRESH_V4_DATABASE_FILE),
    )
    .await;
    let platform = build_platform(pool).await;

    let owner_id = Uuid::now_v7().to_string();
    let current_user = CurrentUser {
        id: nomifun_common::UserId::parse(owner_id.clone()).unwrap(),
        username: "owner".to_owned(),
    };
    let router = nomifun_app::create_agent_platform_router(Arc::clone(&platform))
        .layer(Extension(current_user));

    let editor_response = post_json::<AgentPresetEditorResponse>(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Minimal route E2E",
            "description": null,
            "model_route_refs": {}
        }),
    )
    .await;
    let preset_id = editor_response.preset.preset_id.clone();
    let revision = editor_response
        .revision
        .expect("template creation must commit an ordinary Revision");
    let revision_ref: PresetRevisionRef =
        serde_json::from_value(serde_json::to_value(&revision.reference).unwrap())
            .unwrap();
    let snapshot = platform
        .control_store()
        .get_snapshot(&revision_ref)
        .await
        .unwrap()
        .expect("template Revision must persist a Snapshot");

    let rejected = post_response(
        &router,
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id,
            "agent_binding": {
                "preset_revision_ref": revision.reference,
                "resolved_snapshot_ref": snapshot.snapshot_ref,
                "typed_resource_bindings": [],
                "binding_version": 1
            }
        }),
    )
    .await;
    assert_eq!(
        rejected.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "deny_unknown_fields must reject the removed agent_binding input"
    );

    let other_owner = CurrentUser {
        id: nomifun_common::UserId::parse(Uuid::now_v7().to_string()).unwrap(),
        username: "other-owner".to_owned(),
    };
    let other_router = nomifun_app::create_agent_platform_router(Arc::clone(&platform))
        .layer(Extension(other_owner));
    let forbidden = post_response(
        &other_router,
        "/api/agent-sessions",
        json!({
            "preset_id": preset_id,
            "title": "Forbidden route session"
        }),
    )
    .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
    let forbidden_error: nomifun_api_types::ErrorResponse =
        response_json(forbidden).await;
    assert_eq!(forbidden_error.code, "RESOURCE_OWNER_MISMATCH");

    let create = post_json::<CreateAgentSessionResponseDto>(
        &router,
        "/api/agent-sessions",
        serde_json::to_value(CreateAgentSessionRequestDto {
            model: None,
            preset_id: preset_id.clone(),
            title: Some("Route session".to_owned()),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(create.state, "opening");
    assert_eq!(create.cursor.seq, 2);
    assert_eq!(create.agent_binding.preset_revision_ref, revision.reference);
    assert_eq!(
        create.agent_binding.resolved_snapshot_ref.snapshot_id,
        snapshot.snapshot_ref.snapshot_id.as_ref()
    );
    assert!(create.agent_binding.typed_resource_bindings.is_empty());
    assert_eq!(create.agent_binding.binding_version, 1);

    let mut next_draft = editor_response.draft;
    next_draft.document.instructions = "Revision two".to_owned();
    let next_preview = post_json::<ResolveAgentPresetPreviewResponse>(
        &router,
        &format!("/api/agent-presets/{preset_id}/resolve-preview"),
        json!({
            "expected_current_revision": revision.reference,
            "draft": next_draft,
            "scene": "agent_settings",
            "surface": "desktop",
            "audience": "owner"
        }),
    )
    .await;
    let next_revision = post_json::<SaveAgentPresetRevisionResponse>(
        &router,
        &format!("/api/agent-presets/{preset_id}/revisions"),
        json!({
            "expected_current_revision": revision.reference,
            "preview_digest": next_preview.preview_digest,
            "draft": next_draft,
            "reason": "prove Session binding revision freeze"
        }),
    )
    .await;
    assert_eq!(next_revision.revision.reference.revision, 2);
    let persisted_session = platform
        .session_store()
        .get_live_session(&create.agent_session_id.clone().into())
        .await
        .unwrap();
    assert_eq!(
        persisted_session.agent_binding.preset_revision_ref.revision,
        revision.reference.revision,
        "advancing current_stable_revision must not rewrite an existing Session binding"
    );

    platform
        .session_store()
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: create.agent_session_id.clone().into(),
            event_id: nomifun_agent_contracts::EventId::from(format!(
                "route-ready:{}",
                create.agent_session_id
            )),
            producer_id: nomifun_agent_contracts::EventProducerId::from(
                "runtime_supervisor",
            ),
            idempotency_key: nomifun_agent_contracts::IdempotencyKey::from(
                format!("route-ready:{}", create.agent_session_id),
            ),
            runtime_binding_id: None,
            runtime_producer_seq: None,
            semantic_event:
                nomifun_agent_contracts::SemanticSessionEventDraft {
                    kind: nomifun_agent_contracts::SessionEventKind(
                        "session/ready".to_owned(),
                    ),
                    kind_version: 1,
                    correlation_id:
                        nomifun_agent_contracts::CorrelationId::from(
                            create.agent_session_id.clone(),
                        ),
                    causation_event_id: None,
                    payload:
                        nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                            nomifun_agent_contracts::StrictJsonValue(json!({
                                "resolved_snapshot_ref":
                                    create.agent_binding.resolved_snapshot_ref,
                                "protocol_version": FROZEN_PROTOCOL_VERSION
                            })),
                        ),
                },
        })
        .await
        .unwrap();

    for path in [
        format!("/api/agent-sessions/{}", create.agent_session_id),
        format!(
            "/api/agent-sessions/{}/capabilities",
            create.agent_session_id
        ),
        format!(
            "/api/agent-sessions/{}/events?after_seq=0&limit=100",
            create.agent_session_id
        ),
        format!(
            "/api/agent-sessions/{}/messages?after_seq=0&limit=100",
            create.agent_session_id
        ),
    ] {
        let response = get(&router, &path).await;
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let envelope: ApiResponse<Value> = response_json(response).await;
        assert!(envelope.success, "{path}");
        assert!(envelope.data.is_some(), "{path}");
    }

    let catalog = platform
        .session_capability_catalog(
            &PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: owner_id.clone(),
            },
            &create.agent_session_id.clone().into(),
        )
        .await
        .unwrap();
    assert_eq!(catalog.agent_session_id.as_ref(), create.agent_session_id);
    assert_eq!(catalog.owner_ref.principal_id, owner_id);
    let expected_snapshot: nomifun_agent_contracts::ResolvedSnapshotRef =
        serde_json::from_value(
            serde_json::to_value(&create.agent_binding.resolved_snapshot_ref).unwrap(),
        )
        .unwrap();
    assert_eq!(
        catalog.resolved_snapshot_ref,
        expected_snapshot
    );

    let fork = post_json::<ForkAgentSessionResponseDto>(
        &router,
        &format!(
            "/api/agent-sessions/{}/forks",
            create.agent_session_id
        ),
        serde_json::to_value(ForkAgentSessionRequestDto {
            target_agent_binding: create.agent_binding.clone(),
            parent_through_seq: create.cursor.seq,
            title: Some("Forked route session".to_owned()),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(fork.parent_agent_session_id, create.agent_session_id);
    assert!(fork.child_base_is_self_contained);
    assert!(!fork.copies_full_transcript);
    assert!(!fork.migrates_runtime_private_handles);
    assert!(!fork.replays_tool_or_effect);
    let active_binding_json = serde_json::to_string(&create.agent_binding).unwrap();
    sqlx::query(
        "INSERT INTO agent_bindings \
         (target_kind, target_id, agent_binding_json) \
         VALUES ('conversation', 'retirement-target', ?)",
    )
    .bind(&active_binding_json)
    .execute(platform.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json) \
         VALUES ('retirement-remote', ?, 'Retirement Remote', ?)",
    )
    .bind(&owner_id)
    .bind(&active_binding_json)
    .execute(platform.pool())
    .await
    .unwrap();

    let cross_owner_delete = delete(
        &other_router,
        &format!("/api/agent-presets/{preset_id}"),
    )
    .await;
    assert_eq!(cross_owner_delete.status(), StatusCode::NOT_FOUND);
    let cross_owner_error: nomifun_api_types::ErrorResponse =
        response_json(cross_owner_delete).await;
    assert_eq!(cross_owner_error.code, "AGENT_PRESET_NOT_FOUND");

    let retired = delete(&router, &format!("/api/agent-presets/{preset_id}")).await;
    assert_eq!(retired.status(), StatusCode::OK);
    let retired_at_ms: Option<i64> = sqlx::query_scalar(
        "SELECT retired_at_ms FROM agent_presets WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(platform.pool())
    .await
    .unwrap();
    assert!(retired_at_ms.is_some());
    let active_agent_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM agent_bindings")
            .fetch_one(platform.pool())
            .await
            .unwrap();
    let active_remote_binding_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM remote_bindings")
            .fetch_one(platform.pool())
            .await
            .unwrap();
    assert_eq!(active_agent_binding_count, 0);
    assert_eq!(active_remote_binding_count, 0);
    let revision_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_preset_revisions WHERE preset_id = ?",
    )
    .bind(&preset_id)
    .fetch_one(platform.pool())
    .await
    .unwrap();
    assert_eq!(revision_count, 2);
    let frozen_binding: AgentBindingValue =
        serde_json::from_value(serde_json::to_value(&create.agent_binding).unwrap()).unwrap();
    let recompiled = platform
        .compile_saved_binding(
            &PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: owner_id.clone(),
            },
            &frozen_binding,
            "agent_session",
            "desktop",
            "owner",
        )
        .await
        .expect("historical Session binding must compile after Preset retirement");
    assert!(recompiled.resource_bindings().is_empty());

    let retired_editor = get(
        &router,
        &format!("/api/agent-presets/{preset_id}/editor"),
    )
    .await;
    assert_eq!(retired_editor.status(), StatusCode::NOT_FOUND);
    let retired_editor_error: nomifun_api_types::ErrorResponse =
        response_json(retired_editor).await;
    assert_eq!(retired_editor_error.code, "AGENT_PRESET_NOT_FOUND");
    let retired_session = post_response(
        &router,
        "/api/agent-sessions",
        json!({ "preset_id": preset_id, "title": "Must not start" }),
    )
    .await;
    assert_eq!(retired_session.status(), StatusCode::NOT_FOUND);
    let retired_session_error: nomifun_api_types::ErrorResponse =
        response_json(retired_session).await;
    assert_eq!(retired_session_error.code, "AGENT_PRESET_NOT_FOUND");
    assert_eq!(
        get(
            &router,
            &format!("/api/agent-sessions/{}", create.agent_session_id),
        )
        .await
        .status(),
        StatusCode::OK,
        "retiring a Preset must not invalidate an existing Session"
    );

    for session_id in [
        fork.child_agent_session_id.as_str(),
        create.agent_session_id.as_str(),
    ] {
        let response = delete(
            &router,
            &format!("/api/agent-sessions/{session_id}"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let envelope: ApiResponse<Value> = response_json(response).await;
        let tombstone = envelope.data.expect("delete response data");
        assert_eq!(tombstone["agent_session_id"], session_id);
        assert_eq!(tombstone["state"], "deleted");
        assert!(tombstone["deleted_at"].as_i64().is_some());
    }

    let deleted = get(
        &router,
        &format!("/api/agent-sessions/{}", create.agent_session_id),
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::GONE);
    let error: nomifun_api_types::ErrorResponse =
        response_json(deleted).await;
    assert_eq!(error.code, "SESSION_DELETED");

    let no_stable = post_json::<AgentPresetEditorResponse>(
        &router,
        "/api/agent-presets",
        json!({
            "display_name": "No stable revision",
            "description": null,
            "fork_from_revision": null
        }),
    )
    .await;
    assert!(no_stable.preset.current_stable_revision.is_none());
    let no_stable_response = post_response(
        &router,
        "/api/agent-sessions",
        json!({ "preset_id": no_stable.preset.preset_id }),
    )
    .await;
    assert_eq!(
        no_stable_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let no_stable_error: nomifun_api_types::ErrorResponse =
        response_json(no_stable_response).await;
    assert_eq!(no_stable_error.code, "CAPABILITY_NOT_MATERIALIZED");

    let missing_snapshot = post_json::<AgentPresetEditorResponse>(
        &router,
        "/api/agent-presets/from-template/chat.minimal",
        json!({
            "display_name": "Missing snapshot",
            "description": null,
            "model_route_refs": {}
        }),
    )
    .await;
    let missing_snapshot_revision = missing_snapshot
        .revision
        .as_ref()
        .expect("template creation must persist a revision");
    let missing_snapshot_ref: PresetRevisionRef = serde_json::from_value(
        serde_json::to_value(&missing_snapshot_revision.reference).unwrap(),
    )
    .unwrap();
    let missing_snapshot_envelope = platform
        .control_store()
        .get_snapshot(&missing_snapshot_ref)
        .await
        .unwrap()
        .expect("template creation must persist a Snapshot");
    sqlx::query("DELETE FROM agent_runtime_snapshots WHERE snapshot_id = ?")
        .bind(missing_snapshot_envelope.snapshot_ref.snapshot_id.as_ref())
        .execute(platform.pool())
        .await
        .unwrap();
    let missing_snapshot_response = post_response(
        &router,
        "/api/agent-sessions",
        json!({ "preset_id": missing_snapshot.preset.preset_id }),
    )
    .await;
    assert_eq!(
        missing_snapshot_response.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let missing_snapshot_error: nomifun_api_types::ErrorResponse =
        response_json(missing_snapshot_response).await;
    assert_eq!(missing_snapshot_error.code, "CAPABILITY_NOT_MATERIALIZED");
}

async fn build_platform(pool: SqlitePool) -> Arc<AgentPlatform> {
    let seed = official_preset_seed_manifest_payload();
    let runtime_inventory:
        nomifun_agent_contracts::CodingRuntimeFeatureInventoryPayload =
        serde_json::from_str(include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json"
        ))
        .unwrap();
    let schema_digest = canonical_schema_manifest_digest().unwrap();
    let protocol = VersionString::from(FROZEN_PROTOCOL_VERSION);
    let release = CompilerReleaseInputs {
        resolver_version: protocol.clone(),
        runtime_protocol_version: protocol.clone(),
        runtime_feature_inventory_digest: seed
            .target_runtime_feature_inventory_digest
            .clone(),
        canonical_schema_manifest_digest: schema_digest.clone(),
        target_contribution_manifest_digest: seed
            .target_first_party_contribution_digest
            .clone(),
        availability_evidence_revision: BUILD_IDENTITY.to_owned(),
    };
    let environment = CompilerEnvironment {
        resolver_version: protocol.clone(),
        required_runtime_protocol_version: protocol,
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: release
            .runtime_feature_inventory_digest
            .clone(),
        available_runtime_features: runtime_inventory.runtime_features,
        installation_role_bindings: BTreeMap::new(),
        canonical_schema_manifest_digest: schema_digest,
        target_contribution_manifest_digest: release
            .target_contribution_manifest_digest
            .clone(),
        host_target: RuntimeTarget::from(native_target()),
        host_surface: "desktop".to_owned(),
        availability_evidence_revision: BUILD_IDENTITY.to_owned(),
    };
    AgentPlatform::from_pool(AgentPlatformConfig::with_supervisor(
        pool,
        MaterializationPolicy::stable(FROZEN_PROTOCOL_VERSION),
        release,
        environment,
        Arc::new(CodexRuntimeSupervisor::new()),
        Arc::new(UnusedBroker),
    ))
    .await
    .unwrap()
}

async fn open_pool(path: &Path) -> SqlitePool {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(5));
    SqlitePoolOptions::new()
        .max_connections(5)
        .connect_with(options)
        .await
        .unwrap()
}

async fn post_json<T: DeserializeOwned>(
    router: &axum::Router,
    path: &str,
    body: Value,
) -> T {
    let response = post_response(router, path, body).await;
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    assert_eq!(
        status,
        StatusCode::OK,
        "{path}: {}",
        String::from_utf8_lossy(&body)
    );
    let envelope: ApiResponse<T> = serde_json::from_slice(&body).unwrap();
    assert!(envelope.success, "{path}");
    envelope.data.expect("success response data")
}

async fn post_response(
    router: &axum::Router,
    path: &str,
    body: Value,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get(router: &axum::Router, path: &str) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn delete(
    router: &axum::Router,
    path: &str,
) -> axum::response::Response {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn response_json<T: DeserializeOwned>(
    response: axum::response::Response,
) -> T {
    let body = response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
    serde_json::from_slice(&body).unwrap()
}

fn native_target() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows_desktop_x64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos_desktop_arm64"
    } else if cfg!(target_os = "macos") {
        "macos_desktop_x64"
    } else {
        "linux_desktop_x64"
    }
}
