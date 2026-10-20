//! E2E tests for cron job HTTP endpoints.
//!
//! Exercises CRUD, workbench binding, model/resource admission, timezone,
//! skills, ownership and idempotent dispatch through the production router.
//! Live provider/tool acceptance is in `nomi_core_live_provider_smoke`.

mod common;

use axum::http::{HeaderValue, StatusCode};
use serde_json::json;
use tower::ServiceExt;


use common::{
    body_json, build_app, delete_with_token,
    get_request, get_with_token, json_with_token, setup_and_login,
};

// Deterministic canonical UUIDv7 values keep conversation fixtures readable.
// Each test gets an isolated database, so reusing these values across tests
// cannot collide.
const TEST_CONV_1: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
const TEST_CONV_2: &str = "0190f5fe-7c00-7a00-8abc-012345678902";
const TEST_CONV_3: &str = "0190f5fe-7c00-7a00-8abc-012345678903";
const MISSING_CRON_JOB_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678999";
const SECONDARY_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345679993";
/// Provider used by the workbench authoring and exact frozen-binding fixtures.
const CRON_PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8abc-012345679992";
const CRON_MODEL: &str = "cron-model";

// ── Helpers ──────────────────────────────────────────────────────────

fn create_job_body(name: &str) -> serde_json::Value {
    json!({
        "name": name,
        "schedule": { "kind": "every", "every_ms": 60000, "description": "every minute" },
        "message": "test message",
        "conversation_id": TEST_CONV_1,
        "conversation_title": "Test Conv",
        "created_by": "user"
    })
}

fn create_at_job_body(name: &str, at_ms: i64) -> serde_json::Value {
    json!({
        "name": name,
        "schedule": { "kind": "at", "at_ms": at_ms, "description": "once" },
        "message": "at message",
        "conversation_id": TEST_CONV_1,
        "created_by": "user"
    })
}

fn create_cron_job_body(name: &str, expr: &str) -> serde_json::Value {
    json!({
        "name": name,
        "schedule": { "kind": "cron", "expr": expr },
        "message": "cron message",
        "conversation_id": TEST_CONV_1,
        "created_by": "user"
    })
}

async fn create_job(app: &mut axum::Router, token: &str, csrf: &str, body: serde_json::Value) -> serde_json::Value {
    let req = json_with_token("POST", "/api/cron/jobs", body, token, csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let json = body_json(resp).await;
    assert_eq!(status, StatusCode::CREATED, "cron create failed: {json}");
    assert_eq!(json["success"], true);
    json["data"].clone()
}

fn run_now_request_with_key(
    cron_job_id: &str,
    token: &str,
    csrf: &str,
    idempotency_key: &str,
) -> axum::http::Request<axum::body::Body> {
    let mut request = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{cron_job_id}/run"),
        json!({}),
        token,
        csrf,
    );
    request.headers_mut().insert(
        "idempotency-key",
        HeaderValue::try_from(idempotency_key).expect("test idempotency key"),
    );
    request
}

fn run_now_request(
    cron_job_id: &str,
    token: &str,
    csrf: &str,
) -> axum::http::Request<axum::body::Body> {
    run_now_request_with_key(
        cron_job_id,
        token,
        csrf,
        &nomifun_common::generate_id(),
    )
}

/// Seed the provider/model pair every nomi cron fixture resolves against.
/// Idempotent so a test may call it once per conversation it seeds.
async fn seed_cron_provider(services: &nomifun_app::compatibility::AppServices) {
    let credentials_encrypted = nomifun_common::encrypt_string(
        r#"{"api_keys":["cron-fixture-only"]}"#,
        &services.encryption_key,
    ).unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO providers (\
            provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
            created_at, updated_at\
         ) VALUES (?, 'openai', 'cron-fixture', 'https://example.invalid', 'bearer', ?, 1, 1, 1)",
    )
    .bind(CRON_PROVIDER_ID)
    .bind(&credentials_encrypted)
    .execute(services.database.pool())
    .await
    .unwrap();
    common::seed_openai_chat_model(services.database.pool(), CRON_PROVIDER_ID, CRON_MODEL).await;
}

/// Create a real compiled Agent configuration through the product API, then
/// reuse its immutable binding for the deterministic AgentSession ID required
/// by these Cron fixtures. No retired Conversation row is involved.
async fn seed_conversation(
    app: &mut axum::Router,
    services: &nomifun_app::compatibility::AppServices,
    token: &str,
    csrf: &str,
    id: &str,
) {
    // This test builder bypasses the production bootstrap that creates the
    // configured work root. Canonical default-workspace admission correctly
    // requires the directory to exist, so establish the fixture precondition
    // before launching the source AgentSession.
    tokio::fs::create_dir_all(&services.work_dir).await.unwrap();
    seed_cron_provider(services).await;
    let preset_response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/agent-presets/from-template/coding.codex",
            json!({
                "display_name": format!("Cron fixture {id}"),
                "reuse_existing": false,
                "model": {
                    "provider_id": CRON_PROVIDER_ID,
                    "model": CRON_MODEL,
                }
            }),
            token,
            csrf,
        ))
        .await
        .unwrap();
    assert_eq!(preset_response.status(), StatusCode::OK);
    let preset = body_json(preset_response).await;
    let preset_id = preset["data"]["preset"]["preset_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/agent-sessions",
            json!({
                "preset_id": preset_id,
                "title": "Cron binding source",
                "resource_selections": [{
                    "resource_kind": "workspace",
                    "resource_id": "default-workspace"
                }, {
                    "resource_kind": "process_session",
                    "resource_id": "managed-process-session"
                }, {
                    "resource_kind": "project_memory",
                    "resource_id": "default-project-memory"
                }]
            }),
            token,
            csrf,
        ))
        .await
        .unwrap();
    let response_status = response.status();
    let response_body = body_json(response).await;
    assert_eq!(response_status, StatusCode::OK, "{response_body}");
    let source_id = response_body["data"]["agent_session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let binding_json: String = sqlx::query_scalar(
        "SELECT agent_binding_json FROM agent_sessions WHERE agent_session_id = ?",
    )
    .bind(&source_id)
    .fetch_one(services.database.pool())
    .await
    .unwrap();
    let binding: nomifun_agent_contracts::AgentBindingValue =
        serde_json::from_str(&binding_json).unwrap();
    let store = nomifun_agent_session::AgentSessionStore::from_pool(
        services.database.pool().clone(),
    )
    .await
    .unwrap();
    let created = store
        .create_session(nomifun_agent_session::CreateSessionRequest::new(
            nomifun_agent_contracts::AgentSessionLiveRecord {
                agent_session_id: id.to_owned().into(),
                owner_ref: nomifun_agent_contracts::PrincipalRef {
                    principal_kind: "user".to_owned(),
                    principal_id: services.authoritative_user_id.to_string(),
                },
                metadata: nomifun_agent_contracts::AgentSessionMetadata {
            purpose: Default::default(),
                    app_project_id: None,
                    companion: None,
                    title: Some("Seeded Cron Session".to_owned()),
                    archived: false,
                    pinned: false,
                    reasoning_effort: None,
                },
                agent_binding: binding,
                remote_binding_provenance: None,
                parent_session_id: None,
                fork_base_payload_id: None,
                next_seq: 1,
            },
            1,
            nomifun_agent_contracts::OperationId::from(format!("cron-fixture:{id}:open")),
            nomifun_agent_contracts::EventProducerId::from("session_api"),
            nomifun_agent_contracts::IdempotencyKey::from(format!("cron-fixture:{id}:open")),
            nomifun_agent_contracts::CorrelationId::from(format!("cron-fixture:{id}:open")),
        ))
        .await
        .unwrap();
    store
        .append_event(&nomifun_agent_contracts::SessionEventAppend {
            agent_session_id: id.to_owned().into(),
            event_id: format!("cron-fixture:{id}:ready").into(),
            producer_id: "runtime_supervisor".into(),
            idempotency_key: format!("cron-fixture:{id}:ready").into(),
            semantic_event: nomifun_agent_contracts::SemanticSessionEventDraft {
                kind: nomifun_agent_contracts::SessionEventKind("session/ready".to_owned()),
                kind_version: 1,
                correlation_id: format!("cron-fixture:{id}:ready").into(),
                causation_event_id: Some(created.opening_ack.event_id),
                payload: nomifun_agent_contracts::SessionEventPayloadRef::InlineJson(
                    nomifun_agent_contracts::StrictJsonValue(json!({})),
                ),
            },
        })
        .await
        .unwrap();
    // The fixture opens this deterministic Session directly through the
    // canonical store. Materialize its default directory just as the host's
    // normal opening path does; runtime admission must not recreate a missing
    // existing user's project on a scheduled retry.
    tokio::fs::create_dir_all(services.work_dir.join("conversations").join(id)).await.unwrap();
}

// ── AU-1/AU-2: Unauthenticated requests ─────────────────────────────

#[tokio::test]
async fn au1_unauthenticated_list_returns_403() {
    let (app, _services) = build_app().await;
    let req = get_request("/api/cron/jobs");
    let resp = app.oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
        "expected 401 or 403, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn au2_unauthenticated_all_endpoints() {
    let (app, _services) = build_app().await;

    let endpoints = vec![
        ("GET", "/api/cron/jobs"),
        ("GET", concat!("/api/cron/jobs/", "0190f5fe-7c00-7a00-8abc-012345678998")),
        (
            "GET",
            concat!(
                "/api/cron/jobs/",
                "0190f5fe-7c00-7a00-8abc-012345678998",
                "/skill"
            ),
        ),
        (
            "DELETE",
            concat!(
                "/api/cron/jobs/",
                "0190f5fe-7c00-7a00-8abc-012345678998",
                "/skill"
            ),
        ),
    ];

    for (method, uri) in endpoints {
        let req = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert!(
            resp.status() == StatusCode::UNAUTHORIZED || resp.status() == StatusCode::FORBIDDEN,
            "{method} {uri} expected 401/403, got {}",
            resp.status()
        );
    }
}

#[tokio::test]
async fn au3_authenticated_users_cannot_observe_or_mutate_each_others_cron_jobs() {
    let (mut app, services) = build_app().await;
    let (owner_token, owner_csrf) =
        setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_conversation(
        &mut app,
        &services,
        &owner_token,
        &owner_csrf,
        TEST_CONV_1,
    )
    .await;
    let conversation_id = TEST_CONV_1.to_owned();

    let mut body = create_job_body("Private Owner Job");
    body["conversation_id"] = json!(conversation_id);
    let created = create_job(&mut app, &owner_token, &owner_csrf, body).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let (foreign_token, foreign_csrf) =
        setup_and_login(&mut app, &services, "secondary", "An0therStrongP@ss!").await;

    let response = app
        .clone()
        .oneshot(get_with_token("/api/cron/jobs", &foreign_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_json(response).await["data"].as_array().unwrap().is_empty());

    let foreign_requests = [
        get_with_token(&format!("/api/cron/jobs/{job_id}"), &foreign_token),
        get_with_token(&format!("/api/cron/jobs/{job_id}/runs"), &foreign_token),
        get_with_token(
            &format!("/api/cron/jobs/{job_id}/conversations"),
            &foreign_token,
        ),
        get_with_token(&format!("/api/cron/jobs/{job_id}/skill"), &foreign_token),
        json_with_token(
            "PUT",
            &format!("/api/cron/jobs/{job_id}"),
            json!({ "name": "Forged" }),
            &foreign_token,
            &foreign_csrf,
        ),
        run_now_request(&job_id, &foreign_token, &foreign_csrf),
        json_with_token(
            "POST",
            &format!("/api/cron/jobs/{job_id}/skill"),
            json!({ "content": "---\nname: forged\n---\nForeign content" }),
            &foreign_token,
            &foreign_csrf,
        ),
        delete_with_token(
            &format!("/api/cron/jobs/{job_id}/skill"),
            &foreign_token,
            &foreign_csrf,
        ),
        delete_with_token(
            &format!("/api/cron/jobs/{job_id}"),
            &foreign_token,
            &foreign_csrf,
        ),
    ];
    for request in foreign_requests {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // Workbench defaults cannot grant the secondary principal installation
    // resources. Reject explicitly rather than silently clamping a CLI bag.
    let credentials_encrypted=common::encrypted_bearer_credentials();
    sqlx::query("INSERT INTO providers (provider_id,platform,name,base_url,auth_scheme,credentials_encrypted,enabled,created_at,updated_at) VALUES (?,'openai','secondary-safe','https://example.invalid','bearer',?,1,1,1)")
        .bind(SECONDARY_PROVIDER_ID).bind(credentials_encrypted).execute(services.database.pool()).await.unwrap();
    common::seed_openai_chat_model(services.database.pool(),SECONDARY_PROVIDER_ID,"model-secondary").await;
    let response=app.clone().oneshot(json_with_token("POST","/api/cron/jobs",json!({
        "name":"Secondary cannot acquire host resources","schedule":{"kind":"every","every_ms":600000},
        "message":"scheduled work","created_by":"user","execution_mode":"new_conversation",
        "agent_selection":{"kind":"template","template_key":"assistant.general"},
        "model":{"provider_id":SECONDARY_PROVIDER_ID,"model":"model-secondary"}
    }),&foreign_token,&foreign_csrf)).await.unwrap();
    assert_eq!(response.status(),StatusCode::FORBIDDEN);
    let response=app.clone().oneshot(get_with_token("/api/cron/jobs",&foreign_token)).await.unwrap();
    assert!(body_json(response).await["data"].as_array().unwrap().is_empty());

    let response = app
        .oneshot(get_with_token(
            &format!("/api/cron/jobs/{job_id}"),
            &owner_token,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let retained = body_json(response).await;
    assert_eq!(retained["data"]["name"], "Private Owner Job");
    assert_eq!(retained["data"]["state"]["run_count"], 0);
}

// ── CJ-1: Create cron job ───────────────────────────────────────────

#[tokio::test]
async fn cj1_create_cron_job() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let data = create_job(&mut app, &token, &csrf, create_job_body("Daily Report")).await;

    assert!(nomifun_common::CronJobId::parse(
        data["cron_job_id"].as_str().expect("cron_job_id UUIDv7 string")
    )
    .is_ok());
    assert!(data.get("id").is_none());
    assert_eq!(data["name"], "Daily Report");
    assert_eq!(data["enabled"], true);
    assert!(data["state"]["next_run_at_ms"].as_i64().is_some());
    assert_eq!(data["state"]["run_count"], 0);
    assert_eq!(data["message"], "test message");
    assert_eq!(data["execution_mode"], "existing");
    assert_eq!(data["metadata"]["conversation_id"], TEST_CONV_1);
    assert!(data["metadata"].get("agent_type").is_none());
    assert_eq!(data["metadata"]["created_by"], "user");
}

#[tokio::test]
async fn cj1b_agent_preset_is_frozen_by_the_host_before_cron_persistence() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_cron_provider(&services).await;

    let preset_response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/agent-presets/from-template/chat.minimal",
            json!({
                "reuse_existing": false,
                "display_name": "Cron preset",
                "description": null,
                "model": {"provider_id": CRON_PROVIDER_ID, "model": CRON_MODEL}
            }),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(preset_response.status(), StatusCode::OK);
    let preset = body_json(preset_response).await["data"].clone();
    let preset_id = preset["preset"]["preset_id"].as_str().unwrap();
    assert!(preset["preset"]["current_stable_revision"].is_object());

    let response = app
        .clone()
        .oneshot(json_with_token(
            "POST",
            "/api/cron/jobs",
            json!({
                "name": "Preset scheduled task",
                "schedule": {
                    "kind": "every",
                    "every_ms": 600000,
                    "description": "every ten minutes"
                },
                "message": "run the saved Agent",
                        "created_by": "user",
                "execution_mode": "new_conversation",
                "agent_selection": {"kind":"preset","preset_id":preset_id},
                "model":{"provider_id":CRON_PROVIDER_ID,"model":CRON_MODEL}
            }),
            &token,
            &csrf,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let job = body_json(response).await["data"].clone();
    let binding=&job["metadata"]["agent_binding"];
    assert_eq!(job["metadata"]["agent_selection"]["preset_id"],preset_id);
    // An independent model selection can use a private execution variant;
    // the authoring preset remains the original selection above.
    assert!(serde_json::from_value::<nomifun_api_types::AgentBindingValueDto>(binding.clone()).is_ok());
    assert!(binding["preset_revision_ref"]["revision"].as_u64().unwrap()>0);
    let resources=binding["typed_resource_bindings"].as_array().unwrap();
    assert!(resources.iter().any(|resource|resource["resource_kind"]=="workspace"));
    assert!(resources.iter().any(|resource|resource["resource_kind"]=="process_session"));
    assert!(job["metadata"].get("agent_config").is_none());

    // Some current-generation migrated jobs have a valid frozen binding but
    // no author-selection hint. A model-only edit must not invent General or
    // reselect the latest authoring preset and drop its existing mounts.
    let job_id=job["cron_job_id"].as_str().unwrap();
    sqlx::query("UPDATE cron_jobs SET agent_selection=NULL WHERE cron_job_id=?")
        .bind(job_id).execute(services.database.pool()).await.unwrap();
    common::seed_openai_chat_model(services.database.pool(),CRON_PROVIDER_ID,"cron-alternate").await;
    let response=app.clone().oneshot(json_with_token("PUT",&format!("/api/cron/jobs/{job_id}"),
        json!({"model":{"provider_id":CRON_PROVIDER_ID,"model":"cron-alternate"}}),&token,&csrf)).await.unwrap();
    let status=response.status();
    let updated=body_json(response).await;
    assert_eq!(status,StatusCode::OK,"model-only exact revision failed: {updated}");
    assert_eq!(updated["data"]["metadata"]["model"]["model"],"cron-alternate");
    assert_eq!(updated["data"]["metadata"]["agent_binding"]["typed_resource_bindings"],binding["typed_resource_bindings"]);
    let source:Option<String>=sqlx::query_scalar("SELECT agent_selection FROM cron_jobs WHERE cron_job_id=?")
        .bind(job_id).fetch_one(services.database.pool()).await.unwrap();
    assert!(source.is_none(),"model overrides do not invent an author identity");
    let author=app.oneshot(get_with_token(&format!("/api/agent-presets/{preset_id}/editor"),&token)).await.unwrap();
    assert_eq!(author.status(),StatusCode::OK);
    assert_eq!(body_json(author).await["data"]["preset"]["current_stable_revision"],preset["preset"]["current_stable_revision"]);
}

// ── CJ-2: Create three schedule types ────────────────────────────────

#[tokio::test]
async fn cj2_create_three_schedule_types() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let now = nomifun_common::now_ms();

    for conversation_id in [TEST_CONV_1, TEST_CONV_2, TEST_CONV_3] {
        seed_conversation(&mut app, &services, &token, &csrf, conversation_id).await;
    }

    let at = create_job(
        &mut app,
        &token,
        &csrf,
        create_at_job_body("At Job", now + 3_600_000),
    )
    .await;
    assert_eq!(at["schedule"]["kind"], "at");
    assert_eq!(at["metadata"]["conversation_id"], TEST_CONV_1);
    assert!(at["state"]["next_run_at_ms"].as_i64().unwrap() > now);

    let mut every_body = create_job_body("Every Job");
    every_body["conversation_id"] = json!(TEST_CONV_2);
    let every = create_job(&mut app, &token, &csrf, every_body).await;
    assert_eq!(every["schedule"]["kind"], "every");
    assert_eq!(every["metadata"]["conversation_id"], TEST_CONV_2);
    let next = every["state"]["next_run_at_ms"].as_i64().unwrap();
    assert!((next - now - 60000).abs() < 3000);

    let mut cron_body = create_cron_job_body("Cron Job", "0 */5 * * * *");
    cron_body["conversation_id"] = json!(TEST_CONV_3);
    let cron = create_job(
        &mut app,
        &token,
        &csrf,
        cron_body,
    )
    .await;
    assert_eq!(cron["schedule"]["kind"], "cron");
    assert_eq!(cron["metadata"]["conversation_id"], TEST_CONV_3);
    assert!(cron["state"]["next_run_at_ms"].as_i64().unwrap() > now);
}

// ── CJ-3: Create parameter validation ────────────────────────────────

#[tokio::test]
async fn cj3_create_missing_required_fields() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let invalid_bodies = vec![
        (
            "name",
            json!({"schedule": {"kind": "every", "every_ms": 60000}, "conversation_id": TEST_CONV_1, "created_by": "user"}),
        ),
        (
            "schedule",
            json!({"name": "X", "conversation_id": TEST_CONV_1, "created_by": "user"}),
        ),
        (
            "created_by",
            json!({"name": "X", "schedule": {"kind": "every", "every_ms": 60000}, "conversation_id": TEST_CONV_1}),
        ),
    ];

    for (missing_field, body) in invalid_bodies {
        let req = json_with_token("POST", "/api/cron/jobs", body, &token, &csrf);
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "missing {missing_field} should return 400"
        );
    }
}

#[tokio::test]
async fn cj3b_create_rejects_workspace_with_edge_whitespace_segment() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_cron_provider(&services).await;
    let workspace = services.work_dir.join("Archive ").join("child");
    tokio::fs::create_dir_all(&workspace).await.unwrap();

    let body = json!({
        "name": "Whitespace Workspace",
        "schedule": { "kind": "every", "every_ms": 60000, "description": "every minute" },
        "message": "test message",
        "created_by": "user",
        "execution_mode": "new_conversation",
        "agent_selection":{"kind":"template","template_key":"assistant.general"},
        "model":{"provider_id":CRON_PROVIDER_ID,"model":CRON_MODEL},
        "workspace":workspace
    });

    let req = json_with_token("POST", "/api/cron/jobs", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let json = body_json(resp).await;
    assert_eq!(json["code"], "WORKSPACE_PATH_EDGE_WHITESPACE_UNSUPPORTED");
    assert!(
        json["error"]
            .as_str()
            .unwrap()
            .contains("begins or ends with whitespace")
    );
}

// ── CJ-4: Get single job ────────────────────────────────────────────

#[tokio::test]
async fn cj4_get_single_job() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Get Test")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let req = get_with_token(&format!("/api/cron/jobs/{job_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["cron_job_id"], json!(job_id));
    assert!(json["data"].get("id").is_none());
    assert_eq!(json["data"]["name"], "Get Test");
}

// ── CJ-5: Get nonexistent job ────────────────────────────────────────

#[tokio::test]
async fn cj5_get_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, _csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = get_with_token(&format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cj5b_missing_frozen_binding_fails_before_creating_a_session() {
    let (mut app, services)=build_app().await;
    let (token,csrf)=setup_and_login(&mut app,&services,"admin","StrongP@ss1").await;
    seed_cron_provider(&services).await;
    let created=create_job(&mut app,&token,&csrf,json!({
        "name":"Current task","schedule":{"kind":"every","every_ms":600000},"message":"Task",
        "created_by":"user","execution_mode":"new_conversation",
        "agent_selection":{"kind":"template","template_key":"assistant.general"},
        "model":{"provider_id":CRON_PROVIDER_ID,"model":CRON_MODEL}
    })).await;
    let job_id=created["cron_job_id"].as_str().unwrap();
    sqlx::query("UPDATE cron_jobs SET agent_binding=NULL,enabled=0,next_run_at=NULL WHERE cron_job_id=?")
        .bind(job_id).execute(services.database.pool()).await.unwrap();
    let count: i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(services.database.pool()).await.unwrap();
    let response=app.oneshot(run_now_request(job_id,&token,&csrf)).await.unwrap();
    assert!(!response.status().is_success());
    let after:i64=sqlx::query_scalar("SELECT COUNT(*) FROM agent_sessions").fetch_one(services.database.pool()).await.unwrap();
    assert_eq!(count,after,"an invalid task cannot materialize a ghost Session");
}

// ── CJ-6: List all jobs ─────────────────────────────────────────────

#[tokio::test]
async fn cj6_list_all_jobs() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    for (i, conversation_id) in [TEST_CONV_1, TEST_CONV_2, TEST_CONV_3]
        .into_iter()
        .enumerate()
    {
        seed_conversation(&mut app, &services, &token, &csrf, conversation_id).await;
        let mut body = create_job_body(&format!("Job {i}"));
        body["conversation_id"] = json!(conversation_id);
        create_job(&mut app, &token, &csrf, body).await;
    }

    let req = get_with_token("/api/cron/jobs", &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let items = json["data"].as_array().unwrap();
    assert!(items.len() >= 3);
}

// ── CJ-7: List by conversation ID ───────────────────────────────────

#[tokio::test]
async fn cj7_list_by_conversation() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_2).await;
    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_3).await;
    let mut body_a = create_job_body("Job A");
    body_a["conversation_id"] = json!(TEST_CONV_1);
    create_job(&mut app, &token, &csrf, body_a).await;

    let mut body_b = create_job_body("Job B");
    body_b["conversation_id"] = json!(TEST_CONV_2);
    create_job(&mut app, &token, &csrf, body_b).await;

    let mut body_c = create_job_body("Job C");
    body_c["conversation_id"] = json!(TEST_CONV_3);
    create_job(&mut app, &token, &csrf, body_c).await;

    let req = get_with_token(&format!("/api/cron/jobs?conversation_id={TEST_CONV_2}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    let items = json["data"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], "Job B");
    assert_eq!(items[0]["metadata"]["conversation_id"], TEST_CONV_2);
}

// ── CJ-8: Update job ────────────────────────────────────────────────

#[tokio::test]
async fn cj8_update_job() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Original")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let update_body = json!({"name": "Updated Name", "enabled": false});
    let req = json_with_token("PUT", &format!("/api/cron/jobs/{job_id}"), update_body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["name"], "Updated Name");
    assert_eq!(json["data"]["enabled"], false);
    assert!(
        json["data"]["metadata"]["updated_at"].as_i64().unwrap()
            >= created["metadata"]["created_at"].as_i64().unwrap()
    );
}

// ── CJ-9: Update schedule type ──────────────────────────────────────

#[tokio::test]
async fn cj9_update_schedule_type() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Schedule Change")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let update_body = json!({"schedule": {"kind": "cron", "expr": "0 */5 * * * *"}});
    let req = json_with_token("PUT", &format!("/api/cron/jobs/{job_id}"), update_body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["schedule"]["kind"], "cron");
    assert!(json["data"]["state"]["next_run_at_ms"].as_i64().is_some());
}

#[tokio::test]
async fn cj9b_update_schedule_preserves_existing_timezone_when_omitted() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(
        &mut app,
        &token,
        &csrf,
        json!({
            "name": "Schedule Change With Timezone",
            "schedule": { "kind": "cron", "expr": "0 0 9 * * *", "tz": "Asia/Shanghai" },
            "message": "cron message",
            "conversation_id": TEST_CONV_1,
                "created_by": "user"
        }),
    )
    .await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let update_body = json!({"schedule": {"kind": "cron", "expr": "0 30 9 * * *"}});
    let req = json_with_token("PUT", &format!("/api/cron/jobs/{job_id}"), update_body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["schedule"]["kind"], "cron");
    assert_eq!(json["data"]["schedule"]["expr"], "0 30 9 * * *");
    assert_eq!(json["data"]["schedule"]["tz"], "Asia/Shanghai");
}

// ── CJ-10: Update nonexistent ────────────────────────────────────────

#[tokio::test]
async fn cj10_update_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let update_body = json!({"name": "X"});
    let req = json_with_token(
        "PUT",
        &format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}"),
        update_body,
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── CJ-11: Delete job ───────────────────────────────────────────────

#[tokio::test]
async fn cj11_delete_job() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("To Delete")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let req = delete_with_token(&format!("/api/cron/jobs/{job_id}"), &token, &csrf);
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let req = get_with_token(&format!("/api/cron/jobs/{job_id}"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── CJ-12: Delete nonexistent ────────────────────────────────────────

#[tokio::test]
async fn cj12_delete_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = delete_with_token(
        &format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}"),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── RN-2: Run now nonexistent ────────────────────────────────────────

#[tokio::test]
async fn rn0_run_now_requires_exactly_one_idempotency_key() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    let uri = format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}/run");

    let missing = json_with_token("POST", &uri, json!({}), &token, &csrf);
    let response = app.clone().oneshot(missing).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let mut duplicate = json_with_token("POST", &uri, json!({}), &token, &csrf);
    duplicate
        .headers_mut()
        .append("idempotency-key", HeaderValue::from_static("first"));
    duplicate
        .headers_mut()
        .append("idempotency-key", HeaderValue::from_static("second"));
    let response = app.oneshot(duplicate).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rn1_run_now_preserves_a_specified_session_and_its_idempotent_receipt() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let conversation_id = TEST_CONV_1.to_owned();

    let mut body = create_job_body("Run Now Job");
    body["conversation_id"] = json!(conversation_id);
    let created = create_job(&mut app, &token, &csrf, body).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let idempotency_key = nomifun_common::generate_id();
    let req = run_now_request_with_key(&job_id, &token, &csrf, &idempotency_key);
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "specified-session admission failed: {body}");
    assert_eq!(body["data"]["conversation_id"], json!(conversation_id));

    let replay = run_now_request_with_key(&job_id, &token, &csrf, &idempotency_key);
    let replay = app.oneshot(replay).await.unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = body_json(replay).await;
    assert_eq!(replay["data"]["conversation_id"], json!(conversation_id));
}

#[tokio::test]
async fn rn1b_lazy_session_modes_materialize_an_immutable_agent_binding() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;
    seed_cron_provider(&services).await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"cron\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"scheduled reply\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"cron\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                ),
        )
        .expect(2)
        .mount(&upstream)
        .await;
    let credentials = nomifun_common::encrypt_string(
        r#"{"api_keys":["cron-e2e-only"]}"#,
        &services.encryption_key,
    )
    .unwrap();
    sqlx::query(
        "UPDATE providers SET base_url = ?, credentials_encrypted = ? WHERE provider_id = ?",
    )
        .bind(format!("{}/v1", upstream.uri()))
        .bind(credentials)
        .bind(CRON_PROVIDER_ID)
        .execute(services.database.pool())
        .await
        .unwrap();

    for execution_mode in ["new_conversation", "existing"] {
        let created = create_job(
            &mut app,
            &token,
            &csrf,
            json!({
                "name": format!("Workbench {execution_mode}"),
                "schedule": {
                    "kind": "every",
                    "every_ms": 600_000,
                    "description": "every ten minutes"
                },
                "message": "run the frozen workbench task",
                        "created_by": "user",
                "execution_mode": execution_mode,
                "agent_selection":{"kind":"template","template_key":"assistant.general"},
                "model":{"provider_id":CRON_PROVIDER_ID,"model":CRON_MODEL}
            }),
        )
        .await;
        let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

        let response = app
            .clone()
            .oneshot(run_now_request(&job_id, &token, &csrf))
            .await
            .unwrap();
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{execution_mode}: {body}");
        let conversation_id = body["data"]["conversation_id"]
            .as_str()
            .expect("created canonical AgentSession");

        let response = app
            .clone()
            .oneshot(get_with_token(
                &format!("/api/agent-sessions/{conversation_id}/projection"),
                &token,
            ))
            .await
            .unwrap();
        let status = response.status();
        let projection = body_json(response).await;
        assert_eq!(status, StatusCode::OK, "{execution_mode}: {projection}");
        assert_eq!(projection["data"]["model"]["provider_id"], CRON_PROVIDER_ID);
        assert_eq!(projection["data"]["model"]["model"], CRON_MODEL);
        assert!(
            projection["data"]["agent_snapshot"]["canonical_binding"].is_object(),
            "{execution_mode} must freeze the host-resolved binding: {projection}"
        );

        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let response = app
                    .clone()
                    .oneshot(get_with_token(
                        &format!("/api/cron/jobs/{job_id}"),
                        &token,
                    ))
                    .await
                    .unwrap();
                let status = response.status();
                let job = body_json(response).await;
                assert_eq!(status, StatusCode::OK, "{execution_mode}: {job}");
                match job["data"]["state"]["last_status"].as_str() {
                    Some("ok") => break,
                    Some("error" | "skipped" | "missed") => {
                        panic!("{execution_mode} did not complete successfully: {job}")
                    }
                    _ => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{execution_mode} Cron run did not settle"));
    }

    assert_eq!(upstream.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn rn2_run_now_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = run_now_request(MISSING_CRON_JOB_ID, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn scheduled_system_rules_do_not_turn_a_single_file_action_into_a_numbered_user_plan() {
    let (mut app,services)=build_app().await;
    let (token,csrf)=setup_and_login(&mut app,&services,"admin","StrongP@ss1").await;
    seed_cron_provider(&services).await;
    let upstream=wiremock::MockServer::start().await;
    let calls=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen=calls.clone();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(move |_: &wiremock::Request| {
            let first=seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst)%2==0;
            let chunks=if first { vec![
                json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"raw-cron-write","type":"function","function":{"name":"write_file","arguments":json!({"path":"raw-cron-proof.txt","content":"RAW_CRON_CONTENT\n"}).to_string()}}]},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]})
            ] } else { vec![
                json!({"choices":[{"index":0,"delta":{"content":"RAW_CRON_DONE"},"finish_reason":null}]}),
                json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]})
            ] };
            let mut body=chunks.into_iter().map(|chunk|format!("data: {chunk}\n\n")).collect::<String>();
            body.push_str("data: [DONE]\n\n");
            wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(body)
        }).expect(4).mount(&upstream).await;
    sqlx::query("UPDATE providers SET base_url=? WHERE provider_id=?")
        .bind(format!("{}/v1",upstream.uri())).bind(CRON_PROVIDER_ID).execute(services.database.pool()).await.unwrap();
    let prompt="Use write_file to create raw-cron-proof.txt with exactly RAW_CRON_CONTENT followed by one LF. Then reply RAW_CRON_DONE.";
    for mode in ["new_conversation","existing"] {
        let workspace=services.work_dir.join(format!("raw-{mode}"));
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let created=create_job(&mut app,&token,&csrf,json!({
            "name":"Raw canonical scheduled task","created_by":"user","message":prompt,
            "schedule":{"kind":"every","every_ms":600000},"execution_mode":mode,
            "model":{"provider_id":CRON_PROVIDER_ID,"model":CRON_MODEL},"workspace":workspace
        })).await;
        let job=created["cron_job_id"].as_str().unwrap();
        let response=app.clone().oneshot(run_now_request(job,&token,&csrf)).await.unwrap();
        let status=response.status();let body=body_json(response).await;
        assert_eq!(status,StatusCode::OK,"{mode}: {body}");
        tokio::time::timeout(std::time::Duration::from_secs(10),async {
            loop {
                let response=app.clone().oneshot(get_with_token(&format!("/api/cron/jobs/{job}"),&token)).await.unwrap();
                let value=body_json(response).await;
                if value["data"]["state"]["run_count"].as_u64()==Some(1) {
                    assert_eq!(value["data"]["state"]["last_status"],"ok","{mode}: {value}");break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        }).await.expect("a single task must dispatch without a fabricated numbered-action plan");
        assert_eq!(tokio::fs::read_to_string(workspace.join("raw-cron-proof.txt")).await.unwrap(),"RAW_CRON_CONTENT\n");
    }
    let requests=upstream.received_requests().await.unwrap();
    assert_eq!(requests.len(),4,"the raw single-action tasks must not enter a plan/rejection loop");
    for request in requests.iter().step_by(2) {
        let body:serde_json::Value=serde_json::from_slice(&request.body).unwrap();
        let messages=body["messages"].as_array().unwrap();
        assert!(messages.iter().any(|message|message["role"]=="user" && message["content"]==prompt));
        assert!(!messages.iter().filter(|message|message["role"]=="user").any(|message|message["content"].to_string().contains("[Scheduled Task")));
        assert!(messages.iter().filter(|message|message["role"]=="system").any(|message|message["content"].to_string().to_ascii_lowercase().contains("scheduled")));
    }
}

// ── SK-1: Save skill ────────────────────────────────────────────────

#[tokio::test]
async fn sk1_save_skill() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Skill Job")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let skill_body = json!({"content": "---\nname: test\ndescription: test skill\n---\nDo something"});
    let req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{job_id}/skill"),
        skill_body,
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── SK-2: Has skill (true) ──────────────────────────────────────────

#[tokio::test]
async fn sk2_has_skill_true() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Skill Check")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let skill_body = json!({"content": "---\nname: x\n---\nContent"});
    let req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{job_id}/skill"),
        skill_body,
        &token,
        &csrf,
    );
    app.clone().oneshot(req).await.unwrap();

    let req = get_with_token(&format!("/api/cron/jobs/{job_id}/skill"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["has_skill"], true);
}

// ── SK-3: Has skill (false) ─────────────────────────────────────────

#[tokio::test]
async fn sk3_has_skill_false() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("No Skill")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let req = get_with_token(&format!("/api/cron/jobs/{job_id}/skill"), &token);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let json = body_json(resp).await;
    assert_eq!(json["data"]["has_skill"], false);
}

// ── SK-4: Save empty skill ──────────────────────────────────────────

#[tokio::test]
async fn sk4_save_empty_skill() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Empty Skill")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let skill_body = json!({"content": ""});
    let req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{job_id}/skill"),
        skill_body,
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── SK-5: Save placeholder skill ────────────────────────────────────

#[tokio::test]
async fn sk5_save_placeholder_skill() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Placeholder Skill")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let skill_body = json!({"content": "TODO: fill in later"});
    let req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{job_id}/skill"),
        skill_body,
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── SK-6: Save skill for nonexistent job ─────────────────────────────

#[tokio::test]
async fn sk6_save_skill_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let skill_body = json!({"content": "---\nname: x\n---\nOk"});
    let req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}/skill"),
        skill_body,
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── SK-7: Delete existing skill ──────────────────────────────────────

#[tokio::test]
async fn sk7_delete_skill() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let created = create_job(&mut app, &token, &csrf, create_job_body("Delete Skill Job")).await;
    let job_id = created["cron_job_id"].as_str().unwrap().to_owned();

    let save_req = json_with_token(
        "POST",
        &format!("/api/cron/jobs/{job_id}/skill"),
        json!({"content": "---\nname: delete-me\n---\nContent"}),
        &token,
        &csrf,
    );
    let save_resp = app.clone().oneshot(save_req).await.unwrap();
    assert_eq!(save_resp.status(), StatusCode::OK);

    let delete_req = delete_with_token(&format!("/api/cron/jobs/{job_id}/skill"), &token, &csrf);
    let delete_resp = app.clone().oneshot(delete_req).await.unwrap();
    assert_eq!(delete_resp.status(), StatusCode::OK);

    let has_req = get_with_token(&format!("/api/cron/jobs/{job_id}/skill"), &token);
    let has_resp = app.oneshot(has_req).await.unwrap();
    assert_eq!(has_resp.status(), StatusCode::OK);

    let json = body_json(has_resp).await;
    assert_eq!(json["data"]["has_skill"], false);
}

// ── SK-8: Delete skill for nonexistent job ───────────────────────────

#[tokio::test]
async fn sk8_delete_skill_nonexistent() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let req = delete_with_token(
        &format!("/api/cron/jobs/{MISSING_CRON_JOB_ID}/skill"),
        &token,
        &csrf,
    );
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ── SC-5: Invalid cron expression ────────────────────────────────────

#[tokio::test]
async fn sc5_invalid_cron_expression() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = create_cron_job_body("Invalid Cron", "invalid cron");
    let req = json_with_token("POST", "/api/cron/jobs", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── SC-6: Cron with timezone ─────────────────────────────────────────

#[tokio::test]
async fn sc6_cron_with_timezone() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "Shanghai Job",
        "schedule": { "kind": "cron", "expr": "0 0 9 * * *", "tz": "Asia/Shanghai" },
        "message": "hello",
        "conversation_id": TEST_CONV_1,
        "created_by": "user"
    });

    seed_conversation(&mut app, &services, &token, &csrf, TEST_CONV_1).await;
    let data = create_job(&mut app, &token, &csrf, body).await;
    let now = nomifun_common::now_ms();
    assert!(data["state"]["next_run_at_ms"].as_i64().unwrap() > now);
}

// ── SC-7: Every zero interval ────────────────────────────────────────

#[tokio::test]
async fn sc7_every_zero_interval() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "Zero Interval",
        "schedule": { "kind": "every", "every_ms": 0 },
        "message": "x",
        "conversation_id": TEST_CONV_1,
        "created_by": "user"
    });
    let req = json_with_token("POST", "/api/cron/jobs", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── SC-8: Every negative interval ────────────────────────────────────

#[tokio::test]
async fn sc8_every_negative_interval() {
    let (mut app, services) = build_app().await;
    let (token, csrf) = setup_and_login(&mut app, &services, "admin", "StrongP@ss1").await;

    let body = json!({
        "name": "Negative Interval",
        "schedule": { "kind": "every", "every_ms": -1000 },
        "message": "x",
        "conversation_id": TEST_CONV_1,
        "created_by": "user"
    });
    let req = json_with_token("POST", "/api/cron/jobs", body, &token, &csrf);
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
