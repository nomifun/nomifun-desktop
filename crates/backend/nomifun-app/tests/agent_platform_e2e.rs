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
    VersionString, official_preset_seed_manifest_payload,
};
use nomifun_agent_control_plane::{
    CompilerReleaseInputs, ControlPlaneStore,
};
use nomifun_agent_kernel::{CompilerEnvironment, MaterializationPolicy};
use nomifun_agent_platform::{AgentPlatform, AgentPlatformConfig};
use nomifun_api_types::{
    AgentBindingValueDto, AgentPresetEditorResponse, ApiResponse,
    CreateAgentSessionRequestDto, CreateAgentSessionResponseDto,
    ForkAgentSessionRequestDto, ForkAgentSessionResponseDto,
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
            "resource_bindings": [],
            "model_route_refs": {}
        }),
    )
    .await;
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
    let binding = AgentBindingValue {
        preset_revision_ref: revision_ref,
        resolved_snapshot_ref: snapshot.snapshot_ref,
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let binding_dto: AgentBindingValueDto =
        serde_json::from_value(serde_json::to_value(binding).unwrap()).unwrap();

    let create = post_json::<CreateAgentSessionResponseDto>(
        &router,
        "/api/agent-sessions",
        serde_json::to_value(CreateAgentSessionRequestDto {
            agent_binding: binding_dto.clone(),
            title: Some("Route session".to_owned()),
        })
        .unwrap(),
    )
    .await;
    assert_eq!(create.state, "opening");
    assert_eq!(create.cursor.seq, 2);
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
            target_agent_binding: binding_dto,
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
    let response = router
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
        .unwrap();
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
