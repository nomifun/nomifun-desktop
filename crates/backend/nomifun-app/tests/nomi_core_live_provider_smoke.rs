//! Ignored canonical Agent Runtime smoke against a real Step Plan provider.
//!
//! Run explicitly with the credential-isolating runner:
//! `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs`
//! The default runner exercises the selected canonical Session → Runtime path.
//! Use `--before-tool-smoke` to exercise the live before-tool decision boundary.
//! `NOMIFUN_LIVE_STEPFUN_MODEL` selects the confirmed model (step-3.7-flash),
//! never an endpoint or fallback. Only the runner may read the credential env.

use std::fmt;
use std::future::Future;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use nomifun_app::bootstrap::{NomiCoreApplication, ServerEnvironment};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zeroize::Zeroizing;

const STEPFUN_PLAN_BASE_URL: &str = "https://api.stepfun.com/step_plan/v1";
const STEPFUN_PLAN_MODEL: &str = "step-3.7-flash";
const LIVE_MODEL_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_STEPFUN_MODEL";
const ENGINE_SMOKE_DEADLINE: Duration = Duration::from_secs(15 * 60);
const LIVE_API_KEY_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_STEPFUN_API_KEY";
const STDIN_CREDENTIAL_LIMIT_BYTES: u64 = 16 * 1024;
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SESSION_MESSAGE_PAGE_LIMIT: u32 = 100;
const SESSION_MESSAGE_MAX_PAGES: usize = 512;
const BOOT_DEADLINE: Duration = Duration::from_secs(90);
const LOCAL_API_DEADLINE: Duration = Duration::from_secs(20);
const TURN_COMMAND_DEADLINE: Duration = Duration::from_secs(180);
const TURN_RESULT_DEADLINE: Duration = Duration::from_secs(120);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const SELECTED_MODEL_MARKER: &str = "NOMIFUN_SELECTED_MODEL_LIVE_OK";
const COLLABORATION_MODEL_MARKER: &str = "NOMIFUN_AGENT_COLLABORATION_LIVE_OK";
const AUTOWORK_MODEL_MARKER: &str = "NOMIFUN_AUTOWORK_LIVE_OK";
const AUTOWORK_TAG: &str = "live-commercial-model-smoke";
const CREDENTIAL_AUDIT_SETTLE_DELAY: Duration = Duration::from_millis(100);
const CREDENTIAL_AUDIT_ATTEMPTS: usize = 5;
const CREDENTIAL_AUDIT_REQUIRED_CLEAN_SCANS: usize = 2;

struct LiveFixture {
    _environment: ServerEnvironment,
    application: NomiCoreApplication,
}

#[derive(Clone)]
struct SmokeFailure {
    phase: &'static str,
    code: String,
    status: u16,
}

impl SmokeFailure {
    fn new(phase: &'static str, code: impl Into<String>, status: u16) -> Self {
        Self {
            phase,
            code: sanitize_code(code.into()),
            status,
        }
    }

    fn http(phase: &'static str, status: StatusCode, body: &Value) -> Self {
        let code = body
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("HTTP_STATUS_FAILURE");
        let code = if code == "CONFLICT" {
            admission_conflict_code(body).unwrap_or(code)
        } else {
            code
        };
        Self::new(phase, code, status.as_u16())
    }
}

impl fmt::Debug for SmokeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmokeFailure")
            .field("phase", &self.phase)
            .field("code", &self.code)
            .field("status", &self.status)
            .finish()
    }
}

impl fmt::Display for SmokeFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "phase={} code={} status={}",
            self.phase, self.code, self.status
        )
    }
}

fn sanitize_code(code: String) -> String {
    if !code.is_empty()
        && code.len() <= 96
        && code
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        code
    } else {
        "UNTYPED_FAILURE".to_owned()
    }
}

async fn hard_deadline<T, F>(
    phase: &'static str,
    deadline_code: &'static str,
    duration: Duration,
    future: F,
) -> Result<T, SmokeFailure>
where
    F: Future<Output = Result<T, SmokeFailure>>,
{
    match tokio::time::timeout(duration, future).await {
        Ok(result) => result,
        Err(_) => Err(SmokeFailure::new(
            phase,
            deadline_code,
            StatusCode::REQUEST_TIMEOUT.as_u16(),
        )),
    }
}

fn required_secret_from_stdin() -> Result<Zeroizing<String>, SmokeFailure> {
    if std::env::var_os(LIVE_API_KEY_ENVIRONMENT_NAME).is_some() {
        return Err(SmokeFailure::new(
            "credentials",
            "LIVE_CREDENTIAL_ENVIRONMENT_PRESENT",
            StatusCode::PRECONDITION_FAILED.as_u16(),
        ));
    }

    let mut raw = Zeroizing::new(String::new());
    std::io::stdin()
        .lock()
        .take(STDIN_CREDENTIAL_LIMIT_BYTES.saturating_add(1))
        .read_to_string(&mut raw)
        .map_err(|_| {
            SmokeFailure::new(
                "credentials",
                "LIVE_CREDENTIAL_STDIN_READ_FAILED",
                StatusCode::PRECONDITION_FAILED.as_u16(),
            )
        })?;
    if raw.len() as u64 > STDIN_CREDENTIAL_LIMIT_BYTES {
        return Err(SmokeFailure::new(
            "credentials",
            "LIVE_CREDENTIAL_TOO_LARGE",
            StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
        ));
    }
    let credential = raw.trim();
    if credential.is_empty() {
        return Err(SmokeFailure::new(
            "credentials",
            "LIVE_CREDENTIAL_EMPTY",
            StatusCode::PRECONDITION_FAILED.as_u16(),
        ));
    }
    if credential.contains(['\r', '\n']) {
        return Err(SmokeFailure::new(
            "credentials",
            "LIVE_CREDENTIAL_INVALID",
            StatusCode::PRECONDITION_FAILED.as_u16(),
        ));
    }
    Ok(Zeroizing::new(credential.to_owned()))
}

fn live_model() -> Result<String, SmokeFailure> {
    match std::env::var(LIVE_MODEL_ENVIRONMENT_NAME) {
        Ok(model) if model == STEPFUN_PLAN_MODEL => Ok(model),
        Err(std::env::VarError::NotPresent) => Ok(STEPFUN_PLAN_MODEL.to_owned()),
        _ => Err(SmokeFailure::new("provider.model", "LIVE_MODEL_INVALID", 400)),
    }
}

async fn build_fixture(root: &TempDir) -> Result<LiveFixture, SmokeFailure> {
    let data_dir = root.path().join("data");
    let work_dir = root.path().join("work");
    let log_dir = root.path().join("logs");
    for directory in [&data_dir, &work_dir, &log_dir] {
        std::fs::create_dir_all(directory).map_err(|_| {
            SmokeFailure::new(
                "bootstrap",
                "TEMP_DIRECTORY_CREATE_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            )
        })?;
    }
    let cli = nomifun_app::cli::Cli {
        host: "127.0.0.1".to_owned(),
        port: 0,
        data_dir,
        work_dir: Some(work_dir),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        local: true,
        log_dir: Some(log_dir),
        log_level: Some("off".to_owned()),
        command: None,
    };
    let environment = hard_deadline(
        "bootstrap",
        "ENVIRONMENT_BOOTSTRAP_DEADLINE_EXCEEDED",
        BOOT_DEADLINE,
        async {
            nomifun_app::bootstrap::init_nomi_core_environment(&cli, "").map_err(|_| {
                SmokeFailure::new(
                    "bootstrap",
                    "ENVIRONMENT_BOOTSTRAP_FAILED",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                )
            })
        },
    )
    .await?;
    let application = hard_deadline(
        "bootstrap",
        "APPLICATION_COMPOSE_DEADLINE_EXCEEDED",
        BOOT_DEADLINE,
        async {
            NomiCoreApplication::compose(&environment)
                .await
                .map_err(|_| {
                    SmokeFailure::new(
                        "bootstrap",
                        "APPLICATION_COMPOSE_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })
        },
    )
    .await?;
    Ok(LiveFixture {
        _environment: environment,
        application,
    })
}

async fn dispatch_json(
    router: &Router,
    phase: &'static str,
    method: Method,
    uri: String,
    body: Option<Value>,
    duration: Duration,
) -> Result<(StatusCode, Value), SmokeFailure> {
    dispatch_json_with_headers(router, phase, method, uri, body, duration, &[]).await
}

async fn dispatch_json_with_headers(
    router: &Router,
    phase: &'static str,
    method: Method,
    uri: String,
    body: Option<Value>,
    duration: Duration,
    extra_headers: &[(&str, &str)],
) -> Result<(StatusCode, Value), SmokeFailure> {
    hard_deadline(phase, "HTTP_DEADLINE_EXCEEDED", duration, async {
        let (body, has_body) = match body {
            Some(value) => {
                let bytes = serde_json::to_vec(&value).map_err(|_| {
                    SmokeFailure::new(
                        phase,
                        "REQUEST_SERIALIZATION_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })?;
                (Body::from(bytes), true)
            }
            None => (Body::empty(), false),
        };
        let mut builder = Request::builder().method(method).uri(uri);
        if has_body {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let request = builder.body(body).map_err(|_| {
            SmokeFailure::new(
                phase,
                "REQUEST_BUILD_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            )
        })?;
        let response = router.clone().oneshot(request).await.map_err(|_| {
            SmokeFailure::new(
                phase,
                "ROUTER_DISPATCH_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            )
        })?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), BODY_LIMIT)
            .await
            .map_err(|_| {
                SmokeFailure::new(
                    phase,
                    "RESPONSE_BODY_READ_FAILED",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
        let value = serde_json::from_slice(&bytes).map_err(|_| {
            let body_kind = if bytes.is_empty() { "EMPTY" } else { "NON_JSON" };
            SmokeFailure::new(
                phase,
                format!("RESPONSE_{body_kind}_HTTP_{}", status.as_u16()),
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
        Ok((status, value))
    })
    .await
}

async fn successful_json(
    router: &Router,
    phase: &'static str,
    method: Method,
    uri: impl Into<String>,
    body: Option<Value>,
    duration: Duration,
    expected: &[StatusCode],
) -> Result<Value, SmokeFailure> {
    let (status, value) =
        dispatch_json(router, phase, method, uri.into(), body, duration).await?;
    if expected.contains(&status) {
        Ok(value)
    } else {
        Err(SmokeFailure::http(phase, status, &value))
    }
}

fn envelope_data(phase: &'static str, value: Value) -> Result<Value, SmokeFailure> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(SmokeFailure::new(
            phase,
            value
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("API_ENVELOPE_FAILURE"),
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }
    value.get("data").cloned().ok_or_else(|| {
        SmokeFailure::new(
            phase,
            "API_DATA_MISSING",
            StatusCode::BAD_GATEWAY.as_u16(),
        )
    })
}

fn required_string(
    phase: &'static str,
    value: &Value,
    pointer: &str,
    code: &'static str,
) -> Result<String, SmokeFailure> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| SmokeFailure::new(phase, code, StatusCode::BAD_GATEWAY.as_u16()))
}

fn require_value(
    phase: &'static str,
    value: &Value,
    pointer: &str,
    code: &'static str,
) -> Result<Value, SmokeFailure> {
    value
        .pointer(pointer)
        .filter(|value| !value.is_null())
        .cloned()
        .ok_or_else(|| SmokeFailure::new(phase, code, StatusCode::BAD_GATEWAY.as_u16()))
}

async fn configure_stepfun(
    router: &Router,
    api_key: &str,
    base_url: &str,
    model: &str,
) -> Result<String, SmokeFailure> {
    let disabled = successful_json(
        router,
        "provider.disable_managed",
        Method::POST,
        "/api/model-services/free/activate",
        Some(json!({ "enabled": false })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let _ = envelope_data("provider.disable_managed", disabled)?;

    let (status, created) = dispatch_json(
        router,
        "provider.create",
        Method::POST,
        "/api/providers".to_owned(),
        Some(json!({
            "platform": "stepfun-plan",
            "name": "Live Step Plan integration smoke",
            "base_url": base_url,
            "auth_scheme": "bearer",
            "credentials": {"api_keys": [api_key]},
            "enabled": true,
            "sort_order": 0,
            "initial_model": {
                "model": model,
                "enabled": true,
                "sort_order": 0,
                "capabilities": [{
                    "task": "chat",
                    "traits": ["function_calling", "reasoning", "streaming"],
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {"temperature": 0.0},
                    "output_limit": 4096
                }]
            },
            "connections": []
        })),
        LOCAL_API_DEADLINE,
    )
    .await?;
    if value_contains(&created, api_key) {
        return Err(SmokeFailure::new(
            "provider.create",
            "PROVIDER_RESPONSE_EXPOSED_CREDENTIAL",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }
    if status != StatusCode::CREATED {
        return Err(SmokeFailure::http("provider.create", status, &created));
    }
    let provider = envelope_data("provider.create", created)?;
    if provider.get("has_credentials").and_then(Value::as_bool) != Some(true)
        || provider.get("credentials").is_some()
        || provider.get("api_key").is_some()
    {
        return Err(SmokeFailure::new(
            "provider.create",
            "PROVIDER_SECRET_PROJECTION_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }
    let provider_id = required_string(
        "provider.create",
        &provider,
        "/provider_id",
        "PROVIDER_ID_MISSING",
    )?;
    Ok(provider_id)
}

async fn create_agent_preset(
    router: &Router,
    provider_id: &str,
    model: &str,
    selected_capabilities: &[(&str, &[&str], &str)],
) -> Result<(String, Value), SmokeFailure> {
    let created = successful_json(
        router,
        "agent_settings.create",
        Method::POST,
        "/api/agent-presets/from-template/chat.minimal",
        Some(json!({
            "reuse_existing": false,
            "display_name": "Live Step Plan product smoke",
            "model_route_refs": {},
            "chat_route_records": {},
            "model": {
                "provider_id": provider_id,
                "model": model
            }
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let editor = envelope_data("agent_settings.create", created)?;
    if editor.pointer("/revision/document/chat_route_records/agent_chat/primary/provider_id")
        != Some(&Value::String(provider_id.to_owned()))
        || editor.pointer("/revision/document/chat_route_records/agent_chat/primary/model")
            != Some(&Value::String(model.to_owned()))
    {
        return Err(SmokeFailure::new(
            "agent_settings.create",
            "STEPFUN_ROUTE_NOT_PRIMARY",
            StatusCode::CONFLICT.as_u16(),
        ));
    }

    let preset_id = required_string(
        "agent_settings.create",
        &editor,
        "/preset/preset_id",
        "PRESET_ID_MISSING",
    )?;
    let revision = require_value(
        "agent_settings.create",
        &editor,
        "/revision/reference",
        "PRESET_REVISION_MISSING",
    )?;
    let mut draft = require_value(
        "agent_settings.create",
        &editor,
        "/draft",
        "PRESET_DRAFT_MISSING",
    )?;
    if draft.get("source_template_key").and_then(Value::as_str) != Some("chat.minimal") {
        return Err(SmokeFailure::new(
            "agent_settings.create",
            "CHAT_MINIMAL_DRAFT_PROVENANCE_MISSING",
            StatusCode::CONFLICT.as_u16(),
        ));
    }

    let capabilities = successful_json(
        router,
        "agent_settings.capabilities",
        Method::GET,
        "/api/capabilities",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let capabilities = envelope_data("agent_settings.capabilities", capabilities)?;
    let capabilities = capabilities.as_array().ok_or_else(|| {
        SmokeFailure::new(
            "agent_settings.capabilities",
            "CAPABILITY_CATALOG_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        )
    })?;
    let mut selections = Vec::with_capacity(selected_capabilities.len());
    for &(capability_id, action_allowlist, required_resource_kind) in selected_capabilities {
        let item = capabilities
            .iter()
            .find(|item| {
                item.pointer("/capability/id").and_then(Value::as_str) == Some(capability_id)
            })
            .ok_or_else(|| {
                SmokeFailure::new(
                    "agent_settings.capabilities",
                    "CODING_CAPABILITY_MISSING",
                    StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                )
            })?;
        if item.get("materialization_state").and_then(Value::as_str) != Some("materialized")
            || !item
                .get("supported_surfaces")
                .and_then(Value::as_array)
                .is_some_and(|surfaces| surfaces.iter().any(|surface| surface == "desktop"))
            || !item
                .get("required_resource_kinds")
                .and_then(Value::as_array)
                .is_some_and(|kinds| {
                    if required_resource_kind.is_empty() {
                        kinds.is_empty()
                    } else {
                        kinds.len() == 1 && kinds[0] == required_resource_kind
                    }
                })
        {
            return Err(SmokeFailure::new(
                "agent_settings.capabilities",
                "CODING_CAPABILITY_NOT_NOMI_MAPPABLE",
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        let version = required_string(
            "agent_settings.capabilities",
            item,
            "/capability/version",
            "CODING_CAPABILITY_VERSION_MISSING",
        )?;
        selections.push(json!({
            "capability": {
                "id": capability_id,
                "version": version
            },
            "action_allowlist": action_allowlist
        }));
    }
    let document = draft
        .pointer_mut("/document")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| {
            SmokeFailure::new(
                "agent_settings.draft",
                "PRESET_DOCUMENT_INVALID",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
    document.insert(
        "enabled_capabilities".to_owned(),
        Value::Array(selections),
    );
    document.insert("skill_bindings".to_owned(), json!([]));
    document.insert(
        "persona".to_owned(),
        Value::String("You are a precise coding agent operating only in the bound workspace.".to_owned()),
    );
    document.insert(
        "instructions".to_owned(),
        Value::String(
            "Use the exact requested native tools. Never replace a dedicated filesystem or VCS tool with shell commands. Correct rejected planning or pre-execution validation proposals using the engine's feedback. Stop on external execution failures or unknown outcomes; never replay an operation whose effects may already have happened."
                .to_owned(),
        ),
    );
    let saved = successful_json(
        router,
        "agent_settings.save",
        Method::POST,
        format!("/api/agent-presets/{preset_id}/revisions"),
        Some(json!({
            "expected_current_revision": revision,
            "draft": draft,
            "reason": "live StepFun Nomi-core coding smoke"
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let saved = envelope_data("agent_settings.save", saved)?;
    if saved.pointer("/revision/document/chat_route_records/agent_chat/primary/provider_id")
        != Some(&Value::String(provider_id.to_owned()))
        || saved.pointer("/revision/document/chat_route_records/agent_chat/primary/model")
            != Some(&Value::String(model.to_owned()))
        || saved
            .pointer("/revision/document/enabled_capabilities")
            .and_then(Value::as_array)
            .is_none()
    {
        return Err(SmokeFailure::new(
            "agent_settings.save",
            "CODING_REVISION_INVALID",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let saved_capabilities = saved
        .pointer("/revision/document/enabled_capabilities")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            SmokeFailure::new(
                "agent_settings.save",
                "CODING_CAPABILITIES_MISSING",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
    let mut saved_ids = saved_capabilities
        .iter()
        .filter_map(|selection| {
            selection
                .pointer("/capability/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    saved_ids.sort();
    let mut expected_ids = selected_capabilities
        .iter()
        .map(|(id, _, _)| (*id).to_owned())
        .collect::<Vec<_>>();
    expected_ids.sort();
    if saved_ids != expected_ids {
        return Err(SmokeFailure::new(
            "agent_settings.save",
            "CODING_CAPABILITY_SET_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    ensure_single_model_route(&saved["revision"]["document"], provider_id, model)?;
    let saved_revision = require_value(
        "agent_settings.save",
        &saved,
        "/revision/reference",
        "SAVED_PRESET_REVISION_MISSING",
    )?;
    let snapshot = require_value(
        "agent_settings.save",
        &saved,
        "/resolved_snapshot_ref",
        "SAVED_SNAPSHOT_REF_MISSING",
    )?;
    Ok((
        preset_id,
        json!({
            "preset_revision_ref": saved_revision,
            "resolved_snapshot_ref": snapshot,
            "typed_resource_bindings": [],
            "binding_version": 1
        }),
    ))
}

// Product selections, not client-authored TypedResourceBinding/operations.
// Both the bounded engine Agent and the legacy coding Agent require these;
// chat.minimal requires neither and must not receive unused selections.
fn coding_resource_selections() -> Value {
    json!([
        {"resource_kind": "workspace", "resource_id": "default-workspace"},
        {"resource_kind": "process_session", "resource_id": "managed-process-session"}
    ])
}

fn verify_resource_selections(binding: &Value, selections: &Value) -> Result<(), SmokeFailure> {
    let selections = selections.as_array().ok_or_else(|| {
        SmokeFailure::new("session.resources", "FIXTURE_RESOURCE_SELECTION_INVALID", 500)
    })?;
    let resources = binding.get("typed_resource_bindings").and_then(Value::as_array).ok_or_else(|| {
        SmokeFailure::new("session.resources", "SESSION_RESOURCE_BINDINGS_MISSING", 502)
    })?;
    if resources.len() != selections.len() || selections.iter().any(|selection| {
        resources.iter().filter(|resource| {
            resource.get("resource_kind") == selection.get("resource_kind")
                && resource.get("resource_id") == selection.get("resource_id")
        }).count() != 1
    }) {
        return Err(SmokeFailure::new("session.resources", "SESSION_RESOURCE_SELECTION_MISMATCH", 409));
    }
    Ok(())
}

async fn create_session(
    router: &Router,
    preset_id: &str,
    provider_id: &str,
    model: &str,
    resource_selections: Value,
) -> Result<(String, Value), SmokeFailure> {
    let created = successful_json(
        router,
        "session.create",
        Method::POST,
        "/api/agent-sessions",
        Some(json!({
            "preset_id": preset_id,
            "title": "Live Step Plan session",
            "resource_selections": resource_selections,
            "model": {
                "provider_id": provider_id,
                "model": model
            }
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let session = envelope_data("session.create", created)?;
    let session_id = required_string(
        "session.create",
        &session,
        "/agent_session_id",
        "SESSION_ID_MISSING",
    )?;
    let observation = successful_json(
        router,
        "session.model_projection",
        Method::GET,
        format!("/api/agent-sessions/{session_id}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let observation = envelope_data("session.model_projection", observation)?;
    let binding = require_value(
        "session.create",
        &session,
        "/agent_binding",
        "SESSION_BINDING_MISSING",
    )?;
    if observation.pointer("/session/agent_session_id")
        != Some(&Value::String(session_id.clone()))
        || observation.pointer("/session/agent_binding") != Some(&binding)
    {
        return Err(SmokeFailure::new(
            "session.model_projection",
            "SESSION_BINDING_PROJECTION_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    verify_resource_selections(&binding, &resource_selections)?;
    Ok((session_id, binding))
}

async fn start_session_turn(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    idempotency_key: &str,
    prompt: String,
) -> Result<String, SmokeFailure> {
    let response = successful_json(
        router,
        phase,
        Method::POST,
        format!("/api/agent-sessions/{session_id}/turns"),
        Some(json!({
            "input": {
                "content": prompt
            },
            "idempotency_key": idempotency_key
        })),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let response = envelope_data(phase, response)?;
    if response.get("agent_session_id").and_then(Value::as_str) != Some(session_id) {
        return Err(SmokeFailure::new(
            phase,
            "SESSION_TURN_IDENTITY_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    required_string(phase, &response, "/operation_id", "SESSION_TURN_OPERATION_ID_MISSING")
}

async fn session_message_cursor(
    router: &Router,
    phase: &'static str,
    session_id: &str,
) -> Result<u64, SmokeFailure> {
    session_messages_after(router, phase, session_id, 0)
        .await
        .map(|(_, cursor)| cursor)
}

async fn session_messages_after(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    after_seq: u64,
) -> Result<(Vec<Value>, u64), SmokeFailure> {
    let mut cursor = after_seq;
    let mut messages = Vec::new();
    for _ in 0..SESSION_MESSAGE_MAX_PAGES {
        let response = successful_json(
            router,
            phase,
            Method::GET,
            format!(
                "/api/agent-sessions/{session_id}/messages?after_seq={cursor}&limit={SESSION_MESSAGE_PAGE_LIMIT}"
            ),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        )
        .await?;
        let page = envelope_data(phase, response)?;
        if page.get("agent_session_id").and_then(Value::as_str) != Some(session_id)
            || page
                .pointer("/next_cursor/agent_session_id")
                .and_then(Value::as_str)
                != Some(session_id)
        {
            return Err(SmokeFailure::new(
                phase,
                "SESSION_PAGE_IDENTITY_MISMATCH",
                StatusCode::CONFLICT.as_u16(),
            ));
        }
        let page_messages = page
            .get("messages")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                SmokeFailure::new(
                    phase,
                    "SESSION_MESSAGES_MISSING",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
        let next_cursor = page
            .pointer("/next_cursor/seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                SmokeFailure::new(
                    phase,
                    "SESSION_CURSOR_MISSING",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
        if next_cursor < cursor {
            return Err(SmokeFailure::new(
                phase,
                "SESSION_CURSOR_REGRESSED",
                StatusCode::CONFLICT.as_u16(),
            ));
        }
        for message in page_messages {
            let message_seq = message.get("last_seq").and_then(Value::as_u64).ok_or_else(|| {
                SmokeFailure::new(
                    phase,
                    "SESSION_MESSAGE_CURSOR_MISSING",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
            if message.get("session_id").and_then(Value::as_str) != Some(session_id)
                || message_seq <= cursor
                || message_seq > next_cursor
            {
                return Err(SmokeFailure::new(
                    phase,
                    "SESSION_MESSAGE_PAGE_INVALID",
                    StatusCode::CONFLICT.as_u16(),
                ));
            }
            messages.push(message.clone());
        }
        if next_cursor == cursor {
            if page_messages.is_empty() {
                return Ok((messages, cursor));
            }
            return Err(SmokeFailure::new(
                phase,
                "SESSION_CURSOR_DID_NOT_ADVANCE",
                StatusCode::CONFLICT.as_u16(),
            ));
        }
        cursor = next_cursor;
    }
    Err(SmokeFailure::new(
        phase,
        "SESSION_MESSAGE_PAGE_LIMIT_EXCEEDED",
        StatusCode::SERVICE_UNAVAILABLE.as_u16(),
    ))
}

fn first_durable_error_code(messages: &[Value]) -> Option<String> {
    for message in messages {
        let Some(projection) = message.get("projection") else {
            continue;
        };
        let is_error = projection.get("status").and_then(Value::as_str) == Some("error")
            || projection.get("type").and_then(Value::as_str) == Some("error")
            || projection.get("error").is_some();
        if !is_error {
            continue;
        }
        if let Some(code) = find_typed_code(projection) {
            if code == "CONFLICT" || code == "UNKNOWN_UPSTREAM_ERROR" {
                if let Some(diagnostic) = admission_conflict_code(projection) {
                    return Some(diagnostic.to_owned());
                }
                if let Some(location) = trusted_error_source_location(projection) {
                    return Some(location);
                }
            }
            return Some(code);
        }
        let output = projection.get("output").and_then(Value::as_str).unwrap_or_default();
        if projection["name"] == "exec_command"
            && output.contains("Capability Kernel rejected Agent Runtime Tool (INVALID_PAYLOAD)")
        {
            let args = &projection["args"];
            let code = if args.get("env").is_some_and(Value::is_null) {
                "CODING_EXEC_NULL_ENV"
            } else if args.get("args").is_some_and(Value::is_null) {
                "CODING_EXEC_NULL_ARGS"
            } else if args["command"] != "git" || args["args"] != json!(["--version"]) {
                "CODING_EXEC_COMMAND_SHAPE_MISMATCH"
            } else if !object_has_only_keys(args, &["operation", "command", "args", "timeout_ms"]) {
                "CODING_EXEC_EXTRA_FIELDS_REJECTED"
            } else {
                "CODING_EXEC_CANONICAL_ARGS_REJECTED"
            };
            return Some(code.to_owned());
        }
        let diagnostic = if output.contains("Capability Kernel rejected Agent Runtime Tool (INVALID_PAYLOAD)") {
            "CODING_KERNEL_INVALID_PAYLOAD"
        } else if output.contains("Capability Kernel rejected Agent Runtime Tool (PRESET_RESOURCE_NOT_BOUND)") {
            "CODING_KERNEL_RESOURCE_NOT_BOUND"
        } else if output.contains("Capability Kernel rejected Agent Runtime Tool (CAPABILITY_UNAVAILABLE)") {
            "CODING_KERNEL_UNAVAILABLE"
        } else if output.contains("No tools executed: submit update_plan alone") {
            "CODING_PLAN_BATCH_REJECTED"
        } else if output.contains("Operation not executed:") || output.contains("Requested calls deferred:") {
            "CODING_INSTRUCTION_SCOPE_DEFERRED"
        } else if output.contains("Source quote does not occur") {
            "CODING_REQUIREMENT_SOURCE_MISMATCH"
        } else if output.contains("source quote exceeds") {
            "CODING_REQUIREMENT_QUOTE_TOO_LONG"
        } else if output.contains("The plan needs reconsideration after") {
            "CODING_REPLAN_REQUIRED"
        } else if output.contains("Call update_plan with an in_progress step") {
            "CODING_PLAN_REQUIRED"
        } else if output.contains("Evidence call is unknown") {
            "CODING_COMPLETION_EVIDENCE_UNKNOWN"
        } else if output.contains("Evidence is failed, unsettled, overlapping or stale") {
            "CODING_COMPLETION_EVIDENCE_UNUSABLE"
        } else if output.contains("Close or explicitly block all plan steps") {
            "CODING_COMPLETION_PLAN_OPEN"
        } else if output.contains("Completion omits recorded requirements") {
            "CODING_COMPLETION_REQUIREMENTS_MISSING"
        } else if output.starts_with("Invalid plan:") {
            "CODING_PLAN_ARGUMENTS_INVALID"
        } else {
            match projection.get("name").and_then(Value::as_str) {
                Some("update_plan") => "CODING_PLAN_REJECTED",
                Some("report_completion") => "CODING_COMPLETION_REJECTED",
                Some("write_file") => "CODING_WRITE_REJECTED",
                Some("read_file") => "CODING_READ_REJECTED",
                Some("apply_patch") => "CODING_PATCH_REJECTED",
                Some("exec_command") => "CODING_EXEC_REJECTED",
                Some("resume_task") => "CODING_RESUME_REJECTED",
                _ => "TOOL_OR_TURN_FAILED",
            }
        };
        return Some(diagnostic.to_owned());
    }
    None
}

// Emit only a source location for a matching, compiled-in diagnostic prefix.
// No provider text, paths, arguments or credentials leave this helper.
fn trusted_error_source_location(value: &Value) -> Option<String> {
    const SOURCES: &[(&str, &str)] = &[
        ("CODING_DRIVER", include_str!("../../nomifun-ai-agent/src/unified_runtime.rs")),
        ("CODING_ERROR", include_str!("../../nomifun-agent-runtime/src/error.rs")),
        ("COMPACTION", include_str!("../../nomifun-agent-runtime/src/compaction.rs")),
        ("CONTEXT_LIFECYCLE", include_str!("../../nomifun-agent-runtime/src/context_lifecycle.rs")),
        ("COMPACTION_SOURCE", include_str!("../../nomifun-agent-runtime/src/compaction_source.rs")),
        ("CODING_HOST", include_str!("../src/router/unified_runtime_host.rs")),
        ("CODING_HISTORY", include_str!("../src/router/unified_runtime_history.rs")),
        ("SESSION_HOST", include_str!("../src/router/engine_session_host.rs")),
        ("KERNEL_SESSION", include_str!("../src/router/engine_kernel_session.rs")),
        ("TOOL_HOST", include_str!("../src/router/engine_tool_host.rs")),
        ("PROCESS_HOST", include_str!("../src/router/engine_process_host.rs")),
        ("WAVE2_HOST", include_str!("../src/router/agent_wave2_host.rs")),
        ("CODING_TURN", include_str!("../../nomifun-agent-runtime/src/turn.rs")),
        ("CODING_REPLAY", include_str!("../../nomifun-agent-runtime/src/history.rs")),
        ("BROKER", include_str!("../../nomifun-chat-model-broker/src/broker.rs")),
        ("MODEL_ADAPTER", include_str!("../../nomifun-chat-model-broker/src/adapter.rs")),
    ];
    fn strings<'a>(value: &'a Value, output: &mut Vec<&'a str>) {
        match value {
            Value::String(text) => output.push(text),
            Value::Object(items) => for item in items.values() { strings(item, output); },
            Value::Array(items) => for item in items { strings(item, output); },
            _ => {}
        }
    }
    let mut messages = Vec::new(); strings(value, &mut messages);
    let mut best = None;
    for (name, source) in SOURCES {
        for (line, text) in source.lines().enumerate() {
            let Some(quoted) = text.split('"').nth(1) else { continue; };
            let prefix = quoted.split(['{', '\\']).next().unwrap_or_default();
            if prefix.len() >= 16 && messages.iter().any(|message| message.contains(prefix))
                && best.as_ref().is_none_or(|(length, _)| prefix.len() > *length)
            {
                best = Some((prefix.len(), format!("DIAG_{name}_L{}", line + 1)));
            }
        }
    }
    best.map(|(_, location)| location)
}

// Classify locally generated admission failures without emitting arbitrary
// error messages, provider responses, workspace paths or credential material.
// This is diagnostic evidence only: every classified conflict still fails.
fn admission_conflict_code(value: &Value) -> Option<&'static str> {
    match value {
        Value::String(message) => [
            ("compaction ended with MaxOutputTokens", "CODING_COMPACTION_OUTPUT_LIMIT"),
            ("Mandatory instructions/task state/accepted inputs and pending images exceed", "CODING_COMPACTION_MANDATORY_BUDGET"),
            ("summary request exceeds its input budget", "CODING_COMPACTION_INPUT_BUDGET"),
            ("compaction cannot fit the retained request/instructions/tools", "CODING_COMPACTION_RETAINED_BUDGET"),
            ("compaction route attempted a Tool Call", "CODING_COMPACTION_TOOL_CALL"),
            ("Agent Runtime turn exceeded the model-step limit", "CODING_MODEL_STEP_LIMIT"),
            ("Agent Runtime context is", "CODING_CONTEXT_TOO_LARGE"),
            ("Agent Runtime model stream ended without a terminal event", "CODING_MODEL_TERMINAL_MISSING"),
            ("Agent Runtime turn panicked", "CODING_TURN_PANICKED"),
            ("provider_http_status=400;", "CODING_PROVIDER_HTTP_400"),
            ("provider_http_status=401;", "CODING_PROVIDER_HTTP_401"),
            ("provider_http_status=402;", "CODING_PROVIDER_HTTP_402"),
            ("provider_http_status=403;", "CODING_PROVIDER_HTTP_403"),
            ("provider_http_status=404;", "CODING_PROVIDER_HTTP_404"),
            ("provider_http_status=408;", "CODING_PROVIDER_HTTP_408"),
            ("provider_http_status=409;", "CODING_PROVIDER_HTTP_409"),
            ("provider_http_status=422;", "CODING_PROVIDER_HTTP_422"),
            ("provider_http_status=429;", "CODING_PROVIDER_HTTP_429"),
            ("provider_http_status=500;", "CODING_PROVIDER_HTTP_500"),
            ("provider_http_status=502;", "CODING_PROVIDER_HTTP_502"),
            ("provider_http_status=503;", "CODING_PROVIDER_HTTP_503"),
            ("provider_http_status=504;", "CODING_PROVIDER_HTTP_504"),
            ("model reused a prior tool-call identity in history", "CODING_HISTORY_CALL_REUSED"),
            ("model stream event follows tool admission or results in the same step", "CODING_HISTORY_STREAM_AFTER_TOOL"),
            ("tool event differs from the active replay model step", "CODING_HISTORY_TOOL_STEP_MISMATCH"),
            ("model event differs from the active replay model step", "CODING_HISTORY_MODEL_STEP_MISMATCH"),
            ("unfinished tool batch before a continuation boundary", "CODING_HISTORY_UNFINISHED_BATCH"),
            ("persisted tool result has no call", "CODING_HISTORY_RESULT_WITHOUT_CALL"),
            ("duplicate persisted tool call", "CODING_HISTORY_DUPLICATE_CALL"),
            ("Agent Runtime history:", "CODING_HISTORY_REJECTED"),
            ("Agent Runtime model stream failed (InvalidRequest)", "CODING_MODEL_INVALID_REQUEST"),
            ("Agent Runtime model stream failed (ProtocolViolation)", "CODING_MODEL_PROTOCOL_VIOLATION"),
            ("Agent Runtime model stream failed (CausalityRejected)", "CODING_MODEL_CAUSALITY_REJECTED"),
            ("Agent Runtime model stream failed (PromptTooLong)", "CODING_MODEL_PROMPT_TOO_LONG"),
            ("Agent Runtime model stream failed (RateLimited)", "CODING_MODEL_RATE_LIMITED"),
            ("Agent Runtime model stream failed (AuthenticationFailed)", "CODING_MODEL_AUTHENTICATION_FAILED"),
            ("Agent Runtime model stream failed (DuplicateOperation)", "CODING_MODEL_DUPLICATE_OPERATION"),
            ("Agent Runtime model stream failed (ShadowNotPrimary)", "CODING_MODEL_SHADOW_NOT_PRIMARY"),
            ("Agent Runtime model stream failed (SessionTerminal)", "CODING_MODEL_SESSION_TERMINAL"),
            ("Agent Runtime model stream failed (RouteNotFound)", "CODING_MODEL_ROUTE_NOT_FOUND"),
            ("Agent Runtime model stream failed (RouteRevisionMismatch)", "CODING_MODEL_ROUTE_REVISION_MISMATCH"),
            ("Agent Runtime model stream failed (AdapterUnavailable)", "CODING_MODEL_ADAPTER_UNAVAILABLE"),
            ("Agent Runtime model stream failed (CredentialReferenceMissing)", "CODING_MODEL_CREDENTIAL_REFERENCE_MISSING"),
            ("Agent Runtime model stream failed (CredentialTargetMismatch)", "CODING_MODEL_CREDENTIAL_TARGET_MISMATCH"),
            ("Agent Runtime model stream failed (UnsupportedFeature)", "CODING_MODEL_UNSUPPORTED_FEATURE"),
            ("Agent Runtime model stream failed (ProviderUnavailable)", "CODING_MODEL_PROVIDER_UNAVAILABLE"),
            ("Agent Runtime model stream failed (StreamInterrupted)", "CODING_MODEL_STREAM_INTERRUPTED"),
            ("Agent Runtime model stream failed (Cancelled)", "CODING_MODEL_CANCELLED"),
            ("Agent Runtime model stream failed (Internal)", "CODING_MODEL_INTERNAL"),
            ("Agent Runtime model stream failed", "CODING_MODEL_FAILED"),
            ("Agent Runtime checkpoint is invalid:", "CODING_CHECKPOINT_REJECTED"),
            ("Agent Runtime context assembly failed:", "CODING_CONTEXT_REJECTED"),
            ("Agent Runtime model emitted an invalid event:", "CODING_MODEL_EVENT_INVALID"),
            ("Agent Skills: requested Skill is not in the Agent's immutable selected Skill locks", "CONFLICT_SKILL_LOCK_SELECTION"),
            ("current Kernel compilation differs from the persisted Nomi resolved Snapshot", "CONFLICT_KERNEL_SNAPSHOT_MISMATCH"),
            ("Nomi Plugin Tool Kernel admission failed:", "CONFLICT_KERNEL_ADMISSION"),
            ("Nomi Plugin Tool session materialization failed:", "CONFLICT_TOOL_MATERIALIZATION"),
            ("Engine Session admission:", "CONFLICT_ENGINE_SESSION_ADMISSION"),
            ("Engine turn receipt:", "CONFLICT_ENGINE_TURN_RECEIPT"),
            ("Engine resources:", "CONFLICT_ENGINE_RESOURCES"),
            ("Nomi Wave 2 workspace", "CONFLICT_WORKSPACE_ADMISSION"),
            ("Nomi Wave 2 AgentSession", "CONFLICT_WORKSPACE_ADMISSION"),
            ("Agent Runtime requires an owned process execute grant", "CONFLICT_PROCESS_GRANT"),
        ].into_iter().find_map(|(known, code)| message.contains(known).then_some(code)),
        Value::Object(values) => values.values().find_map(admission_conflict_code),
        Value::Array(values) => values.iter().find_map(admission_conflict_code),
        _ => None,
    }
}

fn find_typed_code(value: &Value) -> Option<String> {
    match value {
        Value::Object(values) => {
            if let Some(code) = values
                .get("code")
                .and_then(Value::as_str)
                .map(str::to_owned)
            {
                let code = sanitize_code(code);
                if code != "UNTYPED_FAILURE" {
                    return Some(code);
                }
            }
            values.values().find_map(find_typed_code)
        }
        Value::Array(values) => values.iter().find_map(find_typed_code),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn object_has_only_keys(value: &Value, allowed: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object
            .keys()
            .all(|key| allowed.contains(&key.as_str()))
    })
}

fn canonical_path_eq(left: &Path, right: &Path) -> bool {
    std::fs::canonicalize(left)
        .ok()
        .zip(std::fs::canonicalize(right).ok())
        .is_some_and(|(left, right)| left == right)
}

fn assistant_text_projection(message: &Value) -> Option<&Value> {
    if message.get("presentation_intent").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let projection = message.get("projection")?;
    let object = projection.as_object()?;
    (object.get("state").and_then(Value::as_str) == Some("completed")
        && object.get("content").and_then(Value::as_str).is_some())
    .then_some(projection)
}

fn exact_assistant_marker_count(messages: &[Value], marker: &str) -> usize {
    messages
        .iter()
        .filter_map(assistant_text_projection)
        .filter(|projection| {
            projection
                .get("content")
                .and_then(Value::as_str)
                .is_some_and(|content| content.trim() == marker)
        })
        .count()
}

async fn wait_for_session_marker(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    after_seq: u64,
    marker: &'static str,
    duration: Duration,
) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let (messages, _) =
            session_messages_after(router, phase, session_id, after_seq).await?;
        if let Some(code) = first_durable_error_code(&messages) {
            return Err(SmokeFailure::new(
                phase,
                code,
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        let unexpected_tools = messages.iter().any(|message| {
            message.get("presentation_intent").and_then(Value::as_str) == Some("tool")
        });
        let assistant_text = messages
            .iter()
            .filter_map(assistant_text_projection)
            .collect::<Vec<_>>();
        let latest_marker_count = exact_assistant_marker_count(&messages, marker);
        let exact_marker = assistant_text.len() == 1
            && latest_marker_count == 1
            && assistant_text[0]
                .get("correlation_id")
                .and_then(Value::as_str)
                .is_some_and(|message_id| uuid::Uuid::parse_str(message_id)
                    .is_ok_and(|value| value.get_version_num() == 7));
        let response = successful_json(
            router,
            phase,
            Method::GET,
            format!("/api/agent-sessions/{session_id}"),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        )
        .await?;
        let observation = envelope_data(phase, response)?;
        let ready =
            observation.pointer("/head/status").and_then(Value::as_str) == Some("ready");
        if ready && unexpected_tools {
            return Err(SmokeFailure::new(
                phase,
                "SESSION_UNEXPECTED_TOOL_EVIDENCE",
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        if ready && exact_marker {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            let (code, status) = if !ready {
                (
                    "SESSION_RESULT_DEADLINE_EXCEEDED",
                    StatusCode::REQUEST_TIMEOUT,
                )
            } else if latest_marker_count > 1 || assistant_text.len() > 1 {
                (
                    "SESSION_MARKER_MULTIPLIED",
                    StatusCode::UNPROCESSABLE_ENTITY,
                )
            } else {
                (
                    "SESSION_EXACT_MARKER_MISSING",
                    StatusCode::UNPROCESSABLE_ENTITY,
                )
            };
            return Err(SmokeFailure::new(phase, code, status.as_u16()));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn assert_distinct_turn_message_identities(
    router: &Router,
    session_id: &str,
    after_seq: u64,
) -> Result<(), SmokeFailure> {
    let (messages, _) = session_messages_after(
        router,
        "model.message_identity",
        session_id,
        after_seq,
    )
    .await?;
    let user_id = messages.iter().find_map(|message| {
        let projection = message.get("projection")?;
        (message.get("presentation_intent").and_then(Value::as_str) == Some("message")
            && projection.get("state").and_then(Value::as_str) == Some("accepted"))
        .then(|| projection.get("correlation_id").and_then(Value::as_str))
        .flatten()
    });
    let assistant_id = messages.iter().find_map(|message| {
        assistant_text_projection(message)?
            .get("correlation_id")
            .and_then(Value::as_str)
    });
    let valid = user_id.zip(assistant_id).is_some_and(|(user, assistant)| {
        user != assistant
            && [user, assistant].iter().all(|value| {
                uuid::Uuid::parse_str(value)
                    .is_ok_and(|parsed| parsed.get_version_num() == 7)
            })
    });
    if !valid {
        return Err(SmokeFailure::new(
            "model.message_identity",
            "USER_ASSISTANT_MESSAGE_IDENTITY_COLLISION",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn assert_session_preset(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    preset_id: &str,
) -> Result<(), SmokeFailure> {
    let response = successful_json(
        router,
        phase,
        Method::GET,
        format!("/api/agent-sessions/{session_id}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let observation = envelope_data(phase, response)?;
    if observation
        .pointer("/session/agent_binding/preset_revision_ref/preset_id")
        .and_then(Value::as_str)
        != Some(preset_id)
    {
        return Err(SmokeFailure::new(
            phase,
            "SESSION_FROZEN_PRESET_CHANGED",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn wait_for_execution_marker(
    router: &Router,
    execution_id: &str,
    marker: &str,
    duration: Duration,
) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let response = successful_json(
            router,
            "cluster.execution",
            Method::GET,
            format!("/api/agent-executions/{execution_id}"),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        )
        .await?;
        let detail = envelope_data("cluster.execution", response)?;
        let status = detail
            .pointer("/execution/status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status == "completed" {
            let exact = detail
                .get("attempts")
                .and_then(Value::as_array)
                .is_some_and(|attempts| {
                    attempts.iter().any(|attempt| {
                        attempt.get("output_summary").and_then(Value::as_str)
                            .is_some_and(|output| output.trim() == marker)
                    })
                });
            if exact {
                return Ok(());
            }
            return Err(SmokeFailure::new(
                "cluster.execution",
                "CLUSTER_EXACT_MARKER_MISSING",
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        if matches!(status, "completed_with_failures" | "failed" | "cancelled") {
            return Err(SmokeFailure::new(
                "cluster.execution",
                format!("CLUSTER_TERMINAL_{status}"),
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new(
                "cluster.execution",
                "CLUSTER_RESULT_DEADLINE_EXCEEDED",
                StatusCode::REQUEST_TIMEOUT.as_u16(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn run_live_agent_collaboration(
    router: &Router,
    session_id: &str,
    provider_id: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    let cursor = session_message_cursor(router, "cluster.cursor_before", session_id).await?;
    let created = successful_json(
        router,
        "cluster.create",
        Method::POST,
        "/api/agent-executions",
        Some(json!({
            "goal": "Run the commercial-model Agent collaboration acceptance step",
            "model_pool": {
                "mode": "single",
                "model": {"provider_id": provider_id, "model": model}
            },
            "lead_model": {"provider_id": provider_id, "model": model},
            "lead_conversation_id": session_id,
            "delegation_policy": "automatic",
            "adaptation_policy": "fixed",
            "decision_policy": "ask_user",
            "max_parallel": 1,
            "steps": [{
                "title": "Commercial model cluster step",
                "spec": format!(
                    "Do not call tools and do not ask a question. Reply with exactly {COLLABORATION_MODEL_MARKER} and no other text."
                )
            }]
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::CREATED],
    )
    .await?;
    let execution = envelope_data("cluster.create", created)?;
    let execution_id = required_string(
        "cluster.create",
        &execution,
        "/execution_id",
        "CLUSTER_EXECUTION_ID_MISSING",
    )?;
    wait_for_execution_marker(
        router,
        &execution_id,
        COLLABORATION_MODEL_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    wait_for_session_marker(
        router,
        "cluster.lead_reply",
        session_id,
        cursor,
        COLLABORATION_MODEL_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    let projection = successful_json(
        router,
        "cluster.link",
        Method::GET,
        format!("/api/agent-sessions/{session_id}/projection"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let projection = envelope_data("cluster.link", projection)?;
    if projection.get("linked_execution_id").and_then(Value::as_str)
        != Some(execution_id.as_str())
    {
        return Err(SmokeFailure::new(
            "cluster.link",
            "CLUSTER_LEAD_LINK_MISSING",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn run_live_autowork(
    router: &Router,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let created = successful_json(
        router,
        "autowork.requirement",
        Method::POST,
        "/api/requirements",
        Some(json!({
            "title": "Commercial model AutoWork acceptance",
            "content": format!(
                "Do not call tools and do not ask a question. Reply with exactly {AUTOWORK_MODEL_MARKER} and no other text."
            ),
            "tag": AUTOWORK_TAG,
            "created_by": "user"
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::CREATED],
    )
    .await?;
    let requirement = envelope_data("autowork.requirement", created)?;
    let requirement_id = required_string(
        "autowork.requirement",
        &requirement,
        "/requirement_id",
        "AUTOWORK_REQUIREMENT_ID_MISSING",
    )?;
    let enabled = successful_json(
        router,
        "autowork.enable",
        Method::POST,
        "/api/requirements/autowork",
        Some(json!({
            "kind": "conversation",
            "target_id": session_id,
            "enabled": true,
            "tag": AUTOWORK_TAG,
            "max_requirements": 1
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let state = envelope_data("autowork.enable", enabled)?;
    if state.get("enabled").and_then(Value::as_bool) != Some(true)
        || state.get("running").and_then(Value::as_bool) != Some(true)
    {
        return Err(SmokeFailure::new(
            "autowork.enable",
            "AUTOWORK_LOOP_NOT_RUNNING",
            StatusCode::CONFLICT.as_u16(),
        ));
    }

    let deadline = tokio::time::Instant::now() + TURN_RESULT_DEADLINE;
    loop {
        let response = successful_json(
            router,
            "autowork.requirement_status",
            Method::GET,
            format!("/api/requirements/{requirement_id}"),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        )
        .await?;
        let requirement = envelope_data("autowork.requirement_status", response)?;
        match requirement.get("status").and_then(Value::as_str) {
            Some("done") => {
                if !requirement
                    .get("completion_note")
                    .and_then(Value::as_str)
                    .is_some_and(|note| note.contains(AUTOWORK_MODEL_MARKER))
                {
                    return Err(SmokeFailure::new(
                        "autowork.requirement_status",
                        "AUTOWORK_COMPLETION_MARKER_MISSING",
                        StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                    ));
                }
                return Ok(());
            }
            Some("failed" | "cancelled" | "needs_review") => {
                return Err(SmokeFailure::new(
                    "autowork.requirement_status",
                    "AUTOWORK_REQUIREMENT_NOT_DONE",
                    StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                ));
            }
            _ => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new(
                "autowork.requirement_status",
                "AUTOWORK_RESULT_DEADLINE_EXCEEDED",
                StatusCode::REQUEST_TIMEOUT.as_u16(),
            ));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

fn value_contains(value: &Value, marker: &str) -> bool {
    match value {
        Value::String(text) => text.contains(marker),
        Value::Array(values) => values.iter().any(|value| value_contains(value, marker)),
        Value::Object(values) => values.values().any(|value| value_contains(value, marker)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn ensure_secret_not_persisted_once(
    root: &Path,
    secret: &[u8],
) -> Result<(), SmokeFailure> {
    if secret.is_empty() {
        return Err(SmokeFailure::new(
            "credential.audit",
            "CREDENTIAL_AUDIT_SECRET_EMPTY",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            // Application shutdown may remove an ephemeral lock/WAL file
            // between directory enumeration and this audit. Its absence
            // cannot preserve the tested credential.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(SmokeFailure::new(
                    "credential.audit",
                    "CREDENTIAL_AUDIT_METADATA_FAILED",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                ));
            }
        };
        if metadata.is_dir() {
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err(SmokeFailure::new(
                        "credential.audit",
                        "CREDENTIAL_AUDIT_DIRECTORY_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    ));
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(_) => {
                        return Err(SmokeFailure::new(
                            "credential.audit",
                            "CREDENTIAL_AUDIT_ENTRY_FAILED",
                            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                        ));
                    }
                };
                pending.push(entry.path());
            }
        } else if metadata.is_file() {
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    return Err(SmokeFailure::new(
                        "credential.audit",
                        "CREDENTIAL_AUDIT_READ_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    ));
                }
            };
            if bytes
                .windows(secret.len())
                .any(|candidate| candidate == secret)
            {
                return Err(SmokeFailure::new(
                    "credential.audit",
                    "PLAINTEXT_CREDENTIAL_PERSISTED",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                ));
            }
        }
    }
    Ok(())
}

async fn ensure_secret_not_persisted(
    root: &Path,
    secret: &[u8],
) -> Result<(), SmokeFailure> {
    // Application shutdown may unlink SQLite/WAL/lock files while the
    // directory walk is in progress. Require consecutive clean scans
    // after a short settle interval so the audit does not turn a harmless
    // disappearing file into a false negative or race with final cleanup.
    let mut last_error = None;
    let mut consecutive_clean_scans = 0;
    for _ in 0..CREDENTIAL_AUDIT_ATTEMPTS {
        match ensure_secret_not_persisted_once(root, secret) {
            Ok(()) => {
                consecutive_clean_scans += 1;
                if consecutive_clean_scans >= CREDENTIAL_AUDIT_REQUIRED_CLEAN_SCANS {
                    return Ok(());
                }
                tokio::time::sleep(CREDENTIAL_AUDIT_SETTLE_DELAY).await;
            }
            Err(error) if error.code == "PLAINTEXT_CREDENTIAL_PERSISTED" => {
                return Err(error);
            }
            Err(error) => {
                consecutive_clean_scans = 0;
                last_error = Some(error);
                tokio::time::sleep(CREDENTIAL_AUDIT_SETTLE_DELAY).await;
            }
        }
    }
    Err(last_error.unwrap_or_else(|| {
        SmokeFailure::new(
            "credential.audit",
            "CREDENTIAL_AUDIT_UNSTABLE",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    }))
}

async fn run_selected_model_chain(
    router: &Router,
    api_key: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    let provider_id =
        configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset_id, _) =
        create_agent_preset(router, &provider_id, model, &[]).await?;
    let (session_id, _) =
        create_session(router, &preset_id, &provider_id, model, json!([])).await?;
    assert_session_runtime(router, &session_id).await?;
    let cursor =
        session_message_cursor(router, "model.cursor_before", &session_id).await?;
    start_session_turn(
        router,
        "model.turn",
        &session_id,
        &uuid::Uuid::now_v7().to_string(),
        format!(
            "This is a model-selection acceptance check. Do not call tools. Reply with exactly {SELECTED_MODEL_MARKER} and no other text."
        ),
    )
    .await?;
    wait_for_session_marker(
        router,
        "model.reply",
        &session_id,
        cursor,
        SELECTED_MODEL_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    assert_distinct_turn_message_identities(router, &session_id, cursor).await?;
    assert_session_runtime(router, &session_id).await?;

    // Switching Agent configuration creates another frozen Session; the first
    // Session must retain its exact original Preset binding.
    let collaboration_actions: &[&str] = &[
        "agent/delegate",
        "agent/request_user_decision",
    ];
    let (collaboration_preset_id, _) = create_agent_preset(
        router,
        &provider_id,
        model,
        &[("agent.collaboration", collaboration_actions, "")],
    )
    .await?;
    let (collaboration_session_id, _) = create_session(
        router,
        &collaboration_preset_id,
        &provider_id,
        model,
        json!([]),
    )
    .await?;
    if collaboration_session_id == session_id || collaboration_preset_id == preset_id {
        return Err(SmokeFailure::new(
            "agent_switch.new_session",
            "AGENT_SWITCH_REUSED_FROZEN_IDENTITY",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    assert_session_preset(
        router,
        "agent_switch.original_binding",
        &session_id,
        &preset_id,
    )
    .await?;
    assert_session_preset(
        router,
        "agent_switch.new_binding",
        &collaboration_session_id,
        &collaboration_preset_id,
    )
    .await?;
    run_live_agent_collaboration(
        router,
        &collaboration_session_id,
        &provider_id,
        model,
    )
    .await?;
    run_live_autowork(router, &collaboration_session_id).await?;
    Ok(())
}

fn ensure_single_model_route(document: &Value, provider_id: &str, model: &str) -> Result<(), SmokeFailure> {
    let route = &document["chat_route_records"]["agent_chat"];
    if route.pointer("/primary/provider_id").and_then(Value::as_str) != Some(provider_id)
        || route.pointer("/primary/model").and_then(Value::as_str) != Some(model)
        || route.get("failovers").is_some_and(|value| value.as_array().is_none_or(|items| !items.is_empty()))
    {
        return Err(SmokeFailure::new("provider.route", "LIVE_MODEL_ROUTE_OR_FAILOVER_INVALID", 409));
    }
    Ok(())
}

async fn official_runtime_build(
    router: &Router,
) -> Result<nomifun_api_types::RuntimeBuildDescriptor, SmokeFailure> {
    let response = successful_json(
        router,
        "runtime.build",
        Method::GET,
        "/api/agent-runtime",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let descriptor = envelope_data("runtime.build", response)?;
    let descriptor: nomifun_api_types::RuntimeBuildDescriptor =
        serde_json::from_value(descriptor)
            .map_err(|_| SmokeFailure::new("runtime.build", "RUNTIME_BUILD_INVALID", 502))?;
    descriptor
        .validate()
        .map_err(|_| SmokeFailure::new("runtime.build", "RUNTIME_BUILD_INVALID", 502))?;
    if descriptor.family_id != "nomifun.nomi" || descriptor.supported_profiles != ["default"] {
        return Err(SmokeFailure::new(
            "runtime.build",
            "OFFICIAL_RUNTIME_IDENTITY_INVALID",
            409,
        ));
    }
    Ok(descriptor)
}

async fn assert_session_runtime(router: &Router, session: &str) -> Result<(), SmokeFailure> {
    let descriptor = official_runtime_build(router).await?;
    let response = successful_json(router, "runtime.session", Method::GET,
        format!("/api/agent-sessions/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let observation = envelope_data("runtime.session", response)?;
    let projection = successful_json(router, "runtime.projection", Method::GET,
        format!("/api/agent-sessions/{session}/projection"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let projection = envelope_data("runtime.projection", projection)?;
    let binding = projection.pointer("/extra/runtime_build_binding")
        .cloned()
        .ok_or_else(|| SmokeFailure::new("runtime.build", "SESSION_RUNTIME_BUILD_MISSING", 409))?;
    let binding: nomifun_api_types::RuntimeBuildBinding = serde_json::from_value(binding)
        .map_err(|_| SmokeFailure::new("runtime.build", "SESSION_RUNTIME_BUILD_INVALID", 409))?;
    binding.validate().map_err(|_| SmokeFailure::new(
        "runtime.build", "SESSION_RUNTIME_BUILD_INVALID", 409))?;
    if observation.pointer("/session/agent_session_id").and_then(Value::as_str) != Some(session)
        || observation.pointer("/session/agent_binding").is_none()
        || observation.pointer("/session/runtime_build_binding").is_some()
        || binding.family_id != descriptor.family_id
        || binding.build_id != descriptor.build_id
        || binding.build_digest != descriptor.build_digest
        || binding.profile != "default"
    {
        return Err(SmokeFailure::new("engine.binding", "CANONICAL_SESSION_BINDING_INVALID", 409));
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum LiveSmokeMode { Model, BeforeTool }

const RETAIN_NATIVE_FIXTURE_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_RETAIN_NATIVE_FIXTURE";
const NATIVE_FIXTURE_PARENT_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_FIXTURE_PARENT";

fn native_fixture_parent(mode: LiveSmokeMode) -> Result<Option<std::path::PathBuf>, SmokeFailure> {
    let Some(enabled) = std::env::var_os(RETAIN_NATIVE_FIXTURE_ENVIRONMENT_NAME) else {
        return Ok(None);
    };
    let failed = || SmokeFailure::new("native.fixture", "NATIVE_FIXTURE_PARENT_INVALID", 400);
    if enabled != "1" || !matches!(mode, LiveSmokeMode::BeforeTool) {
        return Err(failed());
    }
    let parent = std::env::var_os(NATIVE_FIXTURE_PARENT_ENVIRONMENT_NAME)
        .map(std::path::PathBuf::from)
        .ok_or_else(failed)?;
    if !parent.is_absolute() || !parent.is_dir() {
        return Err(failed());
    }
    let parent = parent.canonicalize().map_err(|_| failed())?;
    let git = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.git")
        .canonicalize()
        .map_err(|_| failed())?;
    if !git.is_dir()
        || parent == git
        || !parent.starts_with(&git)
        || parent
            .to_str()
            .is_none_or(|path| path.chars().any(char::is_control))
    {
        return Err(failed());
    }
    Ok(Some(parent))
}

fn retain_native_fixture(root: TempDir, parent: &Path) -> Result<(), SmokeFailure> {
    let failed = || SmokeFailure::new("native.fixture", "NATIVE_FIXTURE_PATHS_INVALID", 422);
    let fixture = root.path().canonicalize().map_err(|_| failed())?;
    let data = fixture.join("data").canonicalize().map_err(|_| failed())?;
    let work = fixture.join("work").canonicalize().map_err(|_| failed())?;
    if fixture.parent() != Some(parent)
        || !fixture
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("before-tool-native-"))
        || data != fixture.join("data")
        || work != fixture.join("work")
        || !data.is_dir()
        || !work.is_dir()
    {
        return Err(failed());
    }
    let paths = json!({
        "fixture_root": fixture.to_str().ok_or_else(failed)?,
        "data_root": data.to_str().ok_or_else(failed)?,
        "work_root": work.to_str().ok_or_else(failed)?,
    });
    let marker = serde_json::to_string(&paths).map_err(|_| failed())?;
    // All fallible verification precedes keep. This root is only for the
    // current native acceptance and contains the ordinary encrypted test
    // Provider record; no service or application process remains running.
    let _kept = root.keep();
    eprintln!("NOMIFUN_LIVE_SMOKE_NATIVE_FIXTURE {marker}");
    Ok(())
}

async fn run_live_provider_smoke(mode: LiveSmokeMode) -> Result<(), SmokeFailure> {
    let retained_parent = native_fixture_parent(mode)?;
    let model = live_model()?;
    let api_key = required_secret_from_stdin()?;
    let root = match retained_parent.as_ref() {
        Some(parent) => tempfile::Builder::new().prefix("before-tool-native-").tempdir_in(parent),
        None => tempfile::tempdir(),
    }.map_err(|_| {
        SmokeFailure::new(
            "bootstrap",
            "TEMP_ROOT_CREATE_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let fixture = build_fixture(&root).await?;
    let router = fixture.application.router();
    let mut stages_passed = Vec::new();

    let result = match mode {
        LiveSmokeMode::Model => run_selected_model_chain(&router, api_key.as_str(), &model).await,
        LiveSmokeMode::BeforeTool => hard_deadline(
            "before_tool.smoke",
            "BEFORE_TOOL_SMOKE_DEADLINE_EXCEEDED",
            ENGINE_SMOKE_DEADLINE,
            before_tool_smoke::run(
                &router,
                api_key.as_str(),
                &model,
                root.path(),
                &mut stages_passed,
            ),
        )
        .await,
    };
    drop(router);
    let LiveFixture {
        _environment: environment,
        application,
    } = fixture;
    let close_result = hard_deadline(
        "shutdown",
        "APPLICATION_SHUTDOWN_DEADLINE_EXCEEDED",
        SHUTDOWN_DEADLINE,
        async {
            application.close().await.map_err(|_| {
                SmokeFailure::new(
                    "shutdown",
                    "APPLICATION_SHUTDOWN_FAILED",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                )
            })
        },
    )
    .await;
    // `ServerEnvironment` owns the tracing flush guard. Drop it before the
    // credential audit so no buffered log write can occur after a clean scan.
    drop(environment);
    close_result?;
    let audit_result = hard_deadline(
        "credential.audit",
        "CREDENTIAL_AUDIT_DEADLINE_EXCEEDED",
        LOCAL_API_DEADLINE,
        async { ensure_secret_not_persisted(root.path(), api_key.as_bytes()).await },
    )
    .await;

    audit_result?;
    for phase in stages_passed {
        before_tool_smoke::emit_stage_pass(phase)?;
    }
    // Native UI acceptance is independent from model instruction fidelity.
    // Explicit retention is safe only here, after shutdown and credential audit;
    // a failed smoke keeps its original failure and never becomes a pass.
    if let Some(parent) = retained_parent { retain_native_fixture(root, &parent)?; }
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --model-smoke"]
async fn nomi_core_selected_model_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::Model).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[path = "nomi_core_live_provider_smoke/before_tool.rs"]
mod before_tool_smoke;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --before-tool-smoke"]
async fn nomi_core_product_before_tool_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::BeforeTool).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn admission_diagnostics_emit_only_static_codes_and_preserve_failure() {
        let location = trusted_error_source_location(&json!({"detail":"process controls cannot change launch parameters; NEVER_PRINT_ME"})).unwrap();
        assert!(location.starts_with("DIAG_PROCESS_HOST_L"));
        assert!(!location.contains("NEVER_PRINT_ME"));
        assert!(trusted_error_source_location(&json!("NEVER_PRINT_ME")).is_none());
        let unknown_upstream = json!({"projection":{"type":"error","code":"UNKNOWN_UPSTREAM_ERROR",
            "error":{"detail":"Agent Runtime history: Agent Runtime checkpoint is invalid: model reused a prior tool-call identity in history; NEVER_PRINT_ME"}}});
        assert_eq!(first_durable_error_code(&[unknown_upstream]).as_deref(), Some("CODING_HISTORY_CALL_REUSED"));
        let message = json!({"projection": {"status":"error", "code":"CONFLICT",
            "error":{"message":"current Kernel compilation differs from the persisted Nomi resolved Snapshot; secret=NEVER_PRINT_ME"}}});
        assert_eq!(first_durable_error_code(&[message]).as_deref(), Some("CONFLICT_KERNEL_SNAPSHOT_MISMATCH"));
        let http = SmokeFailure::http("session.create", StatusCode::CONFLICT,
            &json!({"code":"CONFLICT", "message":"Engine resources: /private/user/path NEVER_PRINT_ME"}));
        assert_eq!(http.code, "CONFLICT_ENGINE_RESOURCES");
        assert_eq!(http.status, 409);
        assert!(!http.to_string().contains("NEVER_PRINT_ME"));
        for raw in ["NEVER_PRINT_ME", "unknown resource conflict", "CONFLICT_KERNEL_SNAPSHOT_MISMATCH"] {
            let unknown = json!({"projection":{"status":"error", "code":"CONFLICT", "message":raw}});
            assert_eq!(first_durable_error_code(&[unknown]).as_deref(), Some("CONFLICT"));
        }
        let non_conflict = json!({"projection":{"status":"error", "code":"FORBIDDEN",
            "message":"Engine resources: NEVER_PRINT_ME"}});
        assert_eq!(first_durable_error_code(&[non_conflict]).as_deref(), Some("FORBIDDEN"));
    }

    #[test]
    fn coding_product_selections_require_both_resources_and_minimal_requires_none() {
        let selections = coding_resource_selections();
        let typed: Vec<nomifun_api_types::AgentResourceSelectionDto> =
            serde_json::from_value(selections.clone()).unwrap();
        assert_eq!(typed.len(), 2);
        assert_eq!(typed[0].resource_kind, "workspace");
        assert_eq!(typed[0].resource_id, "default-workspace");
        assert_eq!(typed[1].resource_kind, "process_session");
        assert_eq!(typed[1].resource_id, "managed-process-session");
        for selection in selections.as_array().unwrap() {
            assert_eq!(selection.as_object().unwrap().len(), 2);
        }
        let binding = json!({"typed_resource_bindings": selections});
        assert!(verify_resource_selections(&binding, &selections).is_ok());
        assert!(verify_resource_selections(&binding, &json!([])).is_err());
        let empty = json!({"typed_resource_bindings": []});
        assert!(verify_resource_selections(&empty, &json!([])).is_ok());
        assert!(verify_resource_selections(&empty, &selections).is_err());
        let missing_process = json!({"typed_resource_bindings": [selections[0]]});
        assert!(verify_resource_selections(&missing_process, &selections).is_err());
        let duplicate = json!({"typed_resource_bindings": [selections[0], selections[0]]});
        assert!(verify_resource_selections(&duplicate, &selections).is_err());
    }

    #[test]
    fn live_route_rejects_fallback_and_wrong_exact_model() {
        let mut document = json!({"chat_route_records":{"agent_chat":{
            "primary":{"provider_id":"provider","model":"step-3.77-flash"},"failovers":[]}}});
        assert!(ensure_single_model_route(&document, "provider", "step-3.77-flash").is_ok());
        assert!(ensure_single_model_route(&document, "provider", "step-3.7-flash").is_err());
        document["chat_route_records"]["agent_chat"]["failovers"] = json!([{"model":"step-3.7-flash"}]);
        assert!(ensure_single_model_route(&document, "provider", "step-3.77-flash").is_err());
    }

    #[test]
    fn nested_plaintext_secret_and_failure_sentinel_are_stable() {
        let secret = "test-secret-value";
        assert!(value_contains(
            &json!({"outer": [{"auth": {"header": format!("Bearer {secret}")}}]}),
            secret,
        ));
        let failure = SmokeFailure::new("coding.exec", "COMMAND_FAILED", 422);
        assert_eq!(
            format!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}"),
            "NOMIFUN_LIVE_SMOKE_FAILURE phase=coding.exec code=COMMAND_FAILED status=422"
        );
    }

    #[tokio::test]
    async fn message_tail_drain_follows_every_cursor_page() {
        let router = Router::new().route(
            "/api/agent-sessions/{session_id}/messages",
            axum::routing::get(
                |axum::extract::Path(session_id): axum::extract::Path<String>,
                 axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| async move {
                    let after_seq = query
                        .get("after_seq")
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or_default();
                    let (messages, next_seq) = match after_seq {
                        0 => (
                            vec![json!({
                                "session_id": session_id,
                                "projection_id": "first",
                                "first_seq": 1,
                                "last_seq": 1,
                                "presentation_intent": "left",
                                "projection": {"content": "first", "turn_id": "turn"},
                                "semantic_digest": "first"
                            })],
                            1,
                        ),
                        1 => (
                            vec![json!({
                                "session_id": session_id,
                                "projection_id": "second",
                                "first_seq": 2,
                                "last_seq": 2,
                                "presentation_intent": "left",
                                "projection": {"content": "second", "turn_id": "turn"},
                                "semantic_digest": "second"
                            })],
                            2,
                        ),
                        _ => (Vec::new(), after_seq),
                    };
                    axum::Json(json!({
                        "success": true,
                        "data": {
                            "agent_session_id": session_id,
                            "messages": messages,
                            "next_cursor": {
                                "agent_session_id": session_id,
                                "seq": next_seq
                            }
                        }
                    }))
                },
            ),
        );

        let (messages, cursor) =
            session_messages_after(&router, "pagination.test", "session", 0)
                .await
                .expect("drain every message page");
        assert_eq!(cursor, 2);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["projection"]["content"], "first");
        assert_eq!(messages[1]["projection"]["content"], "second");
    }
}
