//! Focused integration coverage for the canonical Fresh-v4 Remote REST front door.
//!
//! This test intentionally composes the same coordinator-selected host used by
//! production startup.  It does not use the legacy `AppServices` router: the
//! Remote endpoints are a Fresh-v4 surface and must share the v4 pool,
//! installation token validator, and AgentSession store.

use std::collections::BTreeMap;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::Router;
use clap::Parser;
use nomifun_agent_contracts::{PresetRevisionRef, UserId as ContractUserId};
use nomifun_agent_control_plane::ControlPlaneStore;
use nomifun_api_types::{
    AgentBindingValueDto, CreateAgentPresetFromTemplateRequest, CreateRemoteBindingRequest,
};
use nomifun_app::bootstrap::{self, FreshV4Application};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use uuid::Uuid;

struct FreshV4Fixture {
    _root: TempDir,
    _environment: nomifun_app::bootstrap::ServerEnvironment,
    application: FreshV4Application,
}

impl FreshV4Fixture {
    async fn close(self) {
        self.application
            .close()
            .await
            .expect("Fresh-v4 application should shut down cleanly");
    }
}

async fn fresh_v4_fixture() -> FreshV4Fixture {
    let root = tempfile::tempdir().expect("Fresh-v4 test root");
    let cli = nomifun_app::cli::Cli::parse_from([
        "remote-rest-e2e",
        "--data-dir",
        root.path().to_str().expect("UTF-8 test root"),
        "--local",
    ]);
    let environment =
        bootstrap::init_environment(&cli, "").expect("Fresh-v4 environment bootstrap");
    let host = environment
        .canonical_host()
        .expect("startup must select the Fresh-v4 canonical host");
    let application = host
        .compose(&environment.config)
        .await
        .expect("Fresh-v4 canonical host composition");

    FreshV4Fixture {
        _root: root,
        _environment: environment,
        application,
    }
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read HTTP response body");
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "response body must be JSON ({error}): {}",
            String::from_utf8_lossy(&bytes)
        )
    })
}

fn request(
    method: &str,
    uri: impl Into<String>,
    token: Option<&str>,
    body: Option<Value>,
) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(uri.into());
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    if body.is_some() {
        builder = builder.header(header::CONTENT_TYPE, "application/json");
    }
    let body = body
        .map(|value| Body::from(serde_json::to_vec(&value).expect("serialize request JSON")))
        .unwrap_or_else(Body::empty);
    builder.body(body).expect("build HTTP request")
}

async fn remote_json(
    router: &Router,
    method: &str,
    uri: impl Into<String>,
    token: Option<&str>,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(request(method, uri, token, body))
        .await
        .expect("dispatch HTTP request");
    let status = response.status();
    let body = response_json(response).await;
    (status, body)
}

async fn mcp_response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 4 * 1024 * 1024)
        .await
        .expect("read MCP response body");
    let text = String::from_utf8_lossy(&bytes);
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        return value;
    }
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| panic!("MCP response had no JSON data: {text}"));
    serde_json::from_str(data)
        .unwrap_or_else(|error| panic!("MCP data line is not JSON ({error}): {data}"))
}

async fn mint_remote_token(router: &Router) -> String {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/webui/access-token")
                .body(Body::empty())
                .expect("build token mint request"),
        )
        .await
        .expect("mint token request");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_json(response).await;
    body["data"]["token"]
        .as_str()
        .expect("token mint response must contain data.token")
        .to_owned()
}

async fn create_owner_binding(
    application: &FreshV4Application,
) -> (ContractUserId, nomifun_api_types::RemoteBindingDto) {
    let owner = ContractUserId::from(application.owner_id().to_owned());
    let preset = application
        .platform()
        .control_plane()
        .create_from_template(
            &owner,
            "chat.minimal",
            CreateAgentPresetFromTemplateRequest {
                display_name: "Remote REST integration preset".to_owned(),
                description: None,
                resource_bindings: Vec::new(),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
            },
        )
        .await
        .expect("create canonical preset from the official template");
    let revision = preset.revision.expect("template creation must persist a revision");
    let revision_ref: PresetRevisionRef =
        serde_json::from_value(serde_json::to_value(&revision.reference).unwrap())
            .expect("decode preset revision reference");
    let snapshot = application
        .platform()
        .control_store()
        .get_snapshot(&revision_ref)
        .await
        .expect("load exact persisted snapshot")
        .expect("template snapshot must be materialized");
    let agent_binding = AgentBindingValueDto {
        preset_revision_ref: revision.reference,
        resolved_snapshot_ref: serde_json::from_value(
            serde_json::to_value(snapshot.snapshot_ref).unwrap(),
        )
        .expect("decode resolved snapshot reference"),
        typed_resource_bindings: Vec::new(),
        binding_version: 1,
    };
    let binding = application
        .platform()
        .control_plane()
        .create_remote_binding(
            &owner,
            CreateRemoteBindingRequest {
                name: "Remote REST integration binding".to_owned(),
                agent_binding,
            },
        )
        .await
        .expect("create owner-scoped RemoteBinding");
    (owner, binding)
}

fn observe_uri(session_id: &str, after_seq: u64) -> String {
    format!(
        "/api/remote/observe?agent_session_id={session_id}&after_seq={after_seq}&limit=100"
    )
}

async fn wait_for_open_terminal(
    application: &FreshV4Application,
    session_id: &str,
) -> nomifun_agent_session::SessionHeadProjection {
    let session_id = nomifun_agent_contracts::AgentSessionId::from(session_id.to_owned());
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let head = application
                .platform()
                .session_store()
                .head(&session_id)
                .await
                .expect("read Remote Session head");
            if head.status != "opening" {
                return head;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Remote open must reach ready or open_failed within the bounded test window")
}

#[tokio::test]
async fn canonical_fresh_v4_remote_rest_is_authenticated_owner_scoped_and_provenance_stable() {
    let fixture = fresh_v4_fixture().await;
    let application = &fixture.application;
    let router = application.router();
    let (owner, binding) = create_owner_binding(application).await;
    let token = mint_remote_token(&router).await;

    // Every Remote REST operation is behind the installation Bearer gate.  The
    // rejection is deliberately checked for each route so a newly added route
    // cannot accidentally bypass the canonical middleware.
    let unauthenticated_routes = [
        ("POST", "/api/remote/open", Some(json!({}))),
        ("POST", "/api/remote/turn", Some(json!({}))),
        (
            "GET",
            "/api/remote/observe?agent_session_id=0190f5fe-7c00-7a00-8000-000000000001",
            None,
        ),
        ("POST", "/api/remote/cancel", Some(json!({}))),
    ];
    for (method, uri, body) in unauthenticated_routes {
        let (status, body_json) = remote_json(&router, method, uri, None, body.clone()).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "missing bearer must be rejected for {method} {uri}"
        );
        assert_eq!(body_json["code"], "REMOTE_AUTH_REQUIRED");

        let (status, body_json) =
            remote_json(&router, method, uri, Some("not-the-installation-token"), body).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "bad bearer must be rejected for {method} {uri}"
        );
        assert_eq!(body_json["code"], "REMOTE_AUTH_REQUIRED");
    }

    // Selector/profile query parameters are retired from the canonical Remote
    // contract. They must fail before binding/session lookup, even with a valid
    // installation token.
    let (status, body) = remote_json(
        &router,
        "POST",
        "/api/remote/open?profile=agent",
        Some(&token),
        Some(json!({
            "binding_id": binding.remote_binding_id,
            "idempotency_key": "retired-profile-query"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "REMOTE_INVALID_REQUEST");

    // Insert a valid-looking binding owned by another installation.  The
    // Remote ingress must treat it as absent rather than resolving it by ID
    // alone or allowing the authenticated owner to cross the binding boundary.
    let foreign_owner = Uuid::now_v7().to_string();
    let foreign_binding_id = "foreign-owner-binding";
    nomifun_db::sqlx::query(
        "INSERT INTO remote_bindings \
         (remote_binding_id, owner_user_id, name, agent_binding_json) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(foreign_binding_id)
    .bind(&foreign_owner)
    .bind("foreign binding")
    .bind(serde_json::to_string(&binding.agent_binding).unwrap())
    .execute(application.pool())
    .await
    .expect("insert foreign-owner binding fixture");

    let (status, body) = remote_json(
        &router,
        "POST",
        "/api/remote/open",
        Some(&token),
        Some(json!({
            "binding_id": foreign_binding_id,
            "idempotency_key": "owner-scope-open"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["code"], "REMOTE_BINDING_NOT_FOUND");

    // Opening is an explicit durable state, and the Session row/event freeze
    // the binding ID + version before any runtime becomes ready.
    let (status, open) = remote_json(
        &router,
        "POST",
        "/api/remote/open",
        Some(&token),
        Some(json!({
            "binding_id": binding.remote_binding_id,
            "idempotency_key": "owner-scope-open-valid"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(open["open_state"], json!({ "state": "opening" }));
    let session_id = open["agent_session_id"]
        .as_str()
        .expect("open response must contain agent_session_id")
        .to_owned();
    assert_eq!(open["cursor"]["agent_session_id"], session_id);
    let terminal_head = wait_for_open_terminal(application, &session_id).await;
    assert_eq!(
        terminal_head.status, "open_failed",
        "the test host has no packaged Codex Runtime sidecar, so opening must fail closed"
    );
    let open_failed_event_count: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM session_events \
         WHERE session_id = ? AND kind = 'session/open-failed'",
    )
    .bind(&session_id)
    .fetch_one(application.pool())
    .await
    .expect("read persisted session/open-failed event");
    assert_eq!(
        open_failed_event_count, 1,
        "a failed Remote open must converge through one durable terminal event"
    );

    let (status, replayed_open) = remote_json(
        &router,
        "POST",
        "/api/remote/open",
        Some(&token),
        Some(json!({
            "binding_id": binding.remote_binding_id,
            "idempotency_key": "owner-scope-open-valid"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(replayed_open["agent_session_id"], session_id);
    assert_eq!(
        replayed_open["open_state"],
        json!({
            "state": "failed",
            "code": "REMOTE_OPEN_FAILED",
            "recoverable": true
        })
    );

    let persisted: (String, i64, String) = nomifun_db::sqlx::query_as(
        "SELECT remote_binding_id, remote_binding_version, owner_ref_json \
         FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(&session_id)
    .fetch_one(application.pool())
    .await
    .expect("read persisted Remote Session provenance");
    assert_eq!(persisted.0, binding.remote_binding_id);
    assert_eq!(persisted.1, 1);
    let owner_ref: Value =
        serde_json::from_str(&persisted.2).expect("persisted owner reference JSON");
    assert_eq!(owner_ref["principal_kind"], "user");
    assert_eq!(owner_ref["principal_id"], application.owner_id());

    let opening_payload: Option<String> = nomifun_db::sqlx::query_scalar(
        "SELECT inline_json FROM session_events \
         WHERE session_id = ? AND kind = 'session/opening'",
    )
    .bind(&session_id)
    .fetch_one(application.pool())
    .await
    .expect("read opening event payload");
    let opening_payload: Value = serde_json::from_str(
        &opening_payload.expect("opening event must retain inline provenance"),
    )
    .expect("opening payload JSON");
    assert_eq!(
        opening_payload["remote_binding_provenance"]["remote_binding_id"],
        binding.remote_binding_id
    );
    assert_eq!(
        opening_payload["remote_binding_provenance"]["binding_version"],
        1
    );

    let (status, turn_failure) = remote_json(
        &router,
        "POST",
        "/api/remote/turn",
        Some(&token),
        Some(json!({
            "agent_session_id": session_id,
            "input": {"text": "must not execute"},
            "idempotency_key": "turn-before-runtime"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(turn_failure["code"], "REMOTE_OPEN_FAILED");

    // The first page starts at seq 0.  Advancing after_seq to the first
    // returned event must exclude that event and every earlier event.
    let (status, first_observation) = remote_json(
        &router,
        "GET",
        observe_uri(&session_id, 0),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first_events = first_observation["events"]
        .as_array()
        .expect("observe response events array");
    assert!(
        first_events
            .iter()
            .any(|event| event["kind"] == "session/opening"),
        "the initial observation must include session/opening"
    );
    let first_seq = first_events
        .first()
        .and_then(|event| event["seq"].as_u64())
        .expect("events must expose sequence numbers");
    let (status, after_observation) = remote_json(
        &router,
        "GET",
        observe_uri(&session_id, first_seq),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let after_events = after_observation["events"]
        .as_array()
        .expect("after_seq observe response events array");
    assert!(
        after_events
            .iter()
            .all(|event| event["seq"].as_u64().is_some_and(|seq| seq > first_seq)),
        "after_seq must be an exclusive cursor: {after_observation}"
    );
    assert!(
        !after_events
            .iter()
            .any(|event| event["kind"] == "session/opening"),
        "after_seq must not replay session/opening"
    );
    let open_failed_event_count_after_replay: i64 = nomifun_db::sqlx::query_scalar(
        "SELECT COUNT(*) FROM session_events \
         WHERE session_id = ? AND kind = 'session/open-failed'",
    )
    .bind(&session_id)
    .fetch_one(application.pool())
    .await
    .expect("read session/open-failed count after idempotent requests");
    assert_eq!(
        open_failed_event_count_after_replay, 1,
        "replayed open/turn/observe requests must not launch another admission attempt"
    );

    // Deleting a mutable RemoteBinding prevents future opens but must not
    // rewrite the immutable provenance already frozen into this Session.
    application
        .platform()
        .control_plane()
        .delete_remote_binding(&owner, &binding.remote_binding_id)
        .await
        .expect("delete owner RemoteBinding");
    assert!(
        application
            .platform()
            .control_plane()
            .get_remote_binding(&owner, &binding.remote_binding_id)
            .await
            .expect("lookup deleted RemoteBinding")
            .is_none()
    );

    let persisted_after_delete: (String, i64) = nomifun_db::sqlx::query_as(
        "SELECT remote_binding_id, remote_binding_version \
         FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(&session_id)
    .fetch_one(application.pool())
    .await
    .expect("read Session provenance after binding deletion");
    assert_eq!(
        persisted_after_delete,
        (binding.remote_binding_id.clone(), 1)
    );
    let (status, _) = remote_json(
        &router,
        "GET",
        observe_uri(&session_id, 0),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "existing Remote Session remains observable after binding deletion"
    );

    // Revocation is published through the same validator instance used by the
    // Remote router.  A token accepted before revoke must fail on the next
    // Remote request with the canonical auth code.
    let revoke = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/webui/access-token")
                .body(Body::empty())
                .expect("build token revoke request"),
        )
        .await
        .expect("revoke token request");
    assert_eq!(revoke.status(), StatusCode::OK);

    let (status, body) = remote_json(
        &router,
        "GET",
        observe_uri(&session_id, 0),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "REMOTE_AUTH_REQUIRED");

    fixture.close().await;
}

#[tokio::test]
async fn canonical_fresh_v4_mcp_exposes_only_the_four_remote_operations() {
    let fixture = fresh_v4_fixture().await;
    let router = fixture.application.router();
    let (_owner, binding) = create_owner_binding(&fixture.application).await;
    let token = mint_remote_token(&router).await;

    let post = |session_id: Option<&str>, body: Value| {
        let mut request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, "127.0.0.1")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }
        request
            .body(Body::from(
                serde_json::to_vec(&body).expect("serialize MCP request"),
            ))
            .expect("build MCP request")
    };

    let queried_initialize = Request::builder()
        .method("POST")
        .uri("/mcp?domains=agent")
        .header(header::HOST, "127.0.0.1")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::from(
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 0,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "canonical-query-e2e", "version": "1.0"}
                }
            }))
            .expect("serialize queried MCP initialize"),
        ))
        .expect("build queried MCP initialize");
    let queried_initialize_response = router
        .clone()
        .oneshot(queried_initialize)
        .await
        .expect("dispatch queried MCP initialize");
    assert_eq!(queried_initialize_response.status(), StatusCode::BAD_REQUEST);
    let queried_body = response_json(queried_initialize_response).await;
    assert_eq!(queried_body["code"], "REMOTE_INVALID_REQUEST");

    let initialize = router
        .clone()
        .oneshot(post(
            None,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "canonical-e2e", "version": "1.0"}
                }
            }),
        ))
        .await
        .expect("dispatch MCP initialize");
    assert_eq!(initialize.status(), StatusCode::OK);
    let session_id = initialize
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .expect("canonical MCP initialize must return a transport session id")
        .to_owned();
    let initialize_response = mcp_response_json(initialize).await;
    assert_eq!(initialize_response["id"], 1);

    let initialized = router
        .clone()
        .oneshot(post(
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        ))
        .await
        .expect("dispatch MCP initialized notification");
    assert_eq!(initialized.status(), StatusCode::ACCEPTED);

    let tools = router
        .clone()
        .oneshot(post(
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list",
                "params": {}
            }),
        ))
        .await
        .expect("dispatch canonical MCP tools/list");
    assert_eq!(tools.status(), StatusCode::OK);
    let tools_response = mcp_response_json(tools).await;
    let names = tools_response["result"]["tools"]
        .as_array()
        .expect("tools/list result must contain tools")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(names, ["open", "turn", "observe", "cancel"]);

    let open_call = router
        .clone()
        .oneshot(post(
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "open",
                    "arguments": {
                        "binding_id": binding.remote_binding_id,
                        "idempotency_key": "canonical-mcp-open"
                    }
                }
            }),
        ))
        .await
        .expect("dispatch canonical MCP open");
    assert_eq!(open_call.status(), StatusCode::OK);
    let open_response = mcp_response_json(open_call).await;
    let open_text = open_response["result"]["content"][0]["text"]
        .as_str()
        .expect("canonical MCP open must return a text result");
    let open_payload: Value =
        serde_json::from_str(open_text).expect("canonical MCP open text must be JSON");
    let agent_session_id = open_payload["agent_session_id"]
        .as_str()
        .expect("canonical MCP open must return agent_session_id")
        .to_owned();
    assert_eq!(open_payload["open_state"], json!({ "state": "opening" }));

    let observe_call = router
        .oneshot(post(
            Some(&session_id),
            json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "observe",
                    "arguments": {
                        "agent_session_id": agent_session_id.clone(),
                        "after_cursor": {
                            "agent_session_id": agent_session_id,
                            "seq": 0
                        },
                        "limit": 100
                    }
                }
            }),
        ))
        .await
        .expect("dispatch canonical MCP observe");
    assert_eq!(observe_call.status(), StatusCode::OK);
    let observe_response = mcp_response_json(observe_call).await;
    let observe_text = observe_response["result"]["content"][0]["text"]
        .as_str()
        .expect("canonical MCP observe must return a text result");
    let observe_payload: Value =
        serde_json::from_str(observe_text).expect("canonical MCP observe text must be JSON");
    assert!(
        observe_payload["events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "session/opening"))
    );

    fixture.close().await;
}
