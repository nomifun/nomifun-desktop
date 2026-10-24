//! Ignored canonical Agent Runtime smoke against a real Step Plan provider.
//!
//! Run explicitly with the credential-isolating runner:
//! `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs`
//! The default runner exercises the selected canonical Session → Runtime path.
//! `NOMIFUN_LIVE_STEPFUN_MODEL` selects a confirmed model (step-3.7-flash by
//! default, or step-5-preview), never an endpoint or fallback. Only the runner
//! may read the credential env.

use std::fmt;
use std::future::Future;
use std::io::Read as _;
use std::path::Path;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode, header};
use futures_util::FutureExt;
use nomifun_app::bootstrap::{NomiCoreApplication, ServerEnvironment};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zeroize::Zeroizing;

#[path = "support/live_idmm_demo.rs"]
mod live_idmm_demo;

#[path = "support/live_reasoning_lifecycle.rs"]
mod live_reasoning_lifecycle;

#[path = "support/live_app_hosting.rs"]
mod live_app_hosting;

#[path = "support/live_user_input.rs"]
mod live_user_input;

#[path = "support/live_resource_wait.rs"]
mod live_resource_wait;

#[path = "support/live_input_queue.rs"]
mod live_input_queue;

#[path = "support/live_scheduled_tasks.rs"]
mod live_scheduled_tasks;
#[path = "support/live_subagent.rs"]
mod live_subagent;
#[path = "support/live_subagent_eight.rs"]
mod live_subagent_eight;

#[cfg(all(feature = "browser-use", feature = "computer-use"))]
#[path = "support/live_general_desktop.rs"]
mod live_general_desktop;
const STEPFUN_PLAN_BASE_URL: &str = "https://api.stepfun.com/step_plan/v1";
const STEPFUN_PLAN_MODEL: &str = "step-3.7-flash";
const STEPFUN_PLAN_MODELS: [&str; 2] = [STEPFUN_PLAN_MODEL, "step-5-preview"];
const LIVE_MODEL_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_STEPFUN_MODEL";
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
const WORKSPACE_FILE_MARKER: &str = "NOMIFUN_WORKSPACE_FILE_LIVE_OK";
const COMPONENT_MARKER: &str = "NOMIFUN_MANAGED_COMPONENT_LIVE_OK";
const COMPONENT_CONTINUATION_MARKER: &str = "NOMIFUN_COMPONENT_CONTINUATION_LIVE_OK";
const COMPANION_MARKER: &str = "NOMIFUN_COMPANION_LIVE_OK";
const CREATIVE_MARKER: &str = "NOMIFUN_CREATIVE_LIVE_OK";
const COLLABORATION_MODEL_MARKER: &str = "NOMIFUN_AGENT_COLLABORATION_LIVE_OK";
const AUTOWORK_MODEL_MARKER: &str = "NOMIFUN_AUTOWORK_LIVE_OK";
const AUTOWORK_TAG: &str = "live-commercial-model-smoke";
const CREDENTIAL_AUDIT_SETTLE_DELAY: Duration = Duration::from_millis(100);
const CREDENTIAL_AUDIT_ATTEMPTS: usize = 5;
const CREDENTIAL_AUDIT_REQUIRED_CLEAN_SCANS: usize = 2;
// A 64-step coding turn can legitimately outlast the ordinary smoke window
// when the live provider must compact context and run real process checks.
const LONG_CODING_TURN_DEADLINE: Duration = Duration::from_secs(1_500);

struct LiveFixture {
    _environment: ServerEnvironment,
    application: NomiCoreApplication,
}

#[derive(Clone, Copy)]
enum LiveCase {
    SubagentIntegration,
    SubagentEightIntegration,
    ScheduledTasks,
    SelectedModel,
    WorkspaceFile,
    ManagedComponent,
    UserInputHandoff,
    ResourceWait,
    ReasoningLifecycle,
    CodingPreset,
    SnakeGame,
    LongCoding,
    Companion,
    CreativeStudio,
    Idmm,
    AppHosting,
    AppWorkspaceUiFixture,
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
        Ok(model) if STEPFUN_PLAN_MODELS.contains(&model.as_str()) => Ok(model),
        Err(std::env::VarError::NotPresent) => Ok(STEPFUN_PLAN_MODEL.to_owned()),
        _ => Err(SmokeFailure::new("provider.model", "LIVE_MODEL_INVALID", 400)),
    }
}

async fn build_fixture(root: &TempDir, local_proof: Option<&str>) -> Result<LiveFixture, SmokeFailure> {
    // macOS temp roots are often spelled /var while their canonical owner is
    // /private/var. Exercise the production storage gate with a real directory,
    // never a platform alias that traverses a symlink.
    let canonical_root = nomifun_common::paths::canonicalize_simplified(root.path())
        .map_err(|_| SmokeFailure::new("bootstrap", "TEMP_ROOT_CANONICALIZATION_FAILED", 500))?;
    let data_dir = canonical_root.join("data");
    let work_dir = canonical_root.join("work");
    let log_dir = canonical_root.join("logs");
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
    let mut environment = hard_deadline(
        "bootstrap",
        "ENVIRONMENT_BOOTSTRAP_DEADLINE_EXCEEDED",
        BOOT_DEADLINE,
        async {
            nomifun_app::bootstrap::init_nomi_core_environment(&cli, "").await.map_err(|_| {
                SmokeFailure::new(
                    "bootstrap",
                    "ENVIRONMENT_BOOTSTRAP_FAILED",
                    StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                )
            })
        },
    )
    .await?;
    if let Some(proof) = local_proof {
        environment.config.auth_policy = nomifun_auth::AuthPolicy::TrustLocalToken;
        environment.config.local_trust_secret = Some(std::sync::Arc::from(proof));
    }
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
        // Match an HTTP server's request task boundary instead of polling the
        // entire route under the scenario driver's nested test stack. A timed
        // out or dropped observation still aborts this exact request task.
        let service = router.clone();
        let dispatch = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            service.oneshot(request).await
        }));
        let response = dispatch.await.map_err(|_| SmokeFailure::new(
            phase, "ROUTER_REQUEST_TASK_FAILED", StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        ))?.map_err(|_| {
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
                    "traits": [],
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
        selections.push(json!({
            "capability": {
                "id": capability_id
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
        {"resource_kind": "process_session", "resource_id": "managed-process-session"},
        {"resource_kind": "project_memory", "resource_id": "default-project-memory"}
    ])
}

fn verify_resource_selections(binding: &Value, selections: &Value) -> Result<(), SmokeFailure> {
    verify_resource_selections_in_workspace(binding, selections, None)
}

fn verify_resource_selections_in_workspace(binding: &Value, selections: &Value, workspace: Option<&Path>) -> Result<(), SmokeFailure> {
    let mut selections = selections.as_array().ok_or_else(|| {
        SmokeFailure::new("session.resources", "FIXTURE_RESOURCE_SELECTION_INVALID", 500)
    })?.clone();
    let resources = binding.get("typed_resource_bindings").and_then(Value::as_array).ok_or_else(|| {
        SmokeFailure::new("session.resources", "SESSION_RESOURCE_BINDINGS_MISSING", 502)
    })?;
    if workspace.is_some() && !selections.iter().any(|selection| selection["resource_kind"] == "workspace") {
        // An explicit cwd adds one location identity when the Agent has no file
        // Actions. It must not acquire file authority through this extra binding.
        let mut locations = resources.iter().filter(|resource| resource["resource_kind"] == "workspace");
        if !locations.next().is_some_and(|resource|
            resource["operations"].as_array().is_some_and(Vec::is_empty))
            || locations.next().is_some()
        {
            return Err(SmokeFailure::new("session.resources", "SESSION_WORKSPACE_LOCATION_AUTHORITY_INVALID", 409));
        }
        selections.push(json!({"resource_kind":"workspace","resource_id":"default-workspace"}));
    }
    if resources.len() != selections.len() {
        return Err(SmokeFailure::new("session.resources", format!("SESSION_RESOURCE_COUNT_MISMATCH_{}_{}", resources.len(), selections.len()), 409));
    }
    if let Some(selection) = selections.iter().find(|selection| {
        resources.iter().filter(|resource| {
            if resource.get("resource_kind") != selection.get("resource_kind") { return false; }
            let kind = resource.get("resource_kind").and_then(Value::as_str);
            if let Some(expected) = workspace.filter(|_| matches!(kind, Some("workspace" | "process_session" | "project_memory"))) {
                let Some(root) = resource.pointer("/typed_parameters/workspace_root").and_then(Value::as_str) else { return false; };
                if std::fs::canonicalize(root).ok().is_none_or(|actual|
                    std::fs::canonicalize(expected).ok().as_ref() != Some(&actual)) {
                    return false;
                }
                let Ok(definition) = serde_json::from_value::<nomifun_agent_contracts::TypedResourceBinding>((*resource).clone()) else { return false; };
                if nomifun_agent_contracts::resource_definition_id(&definition).ok().as_ref() != Some(&definition.binding_id) {
                    return false;
                }
                if kind == Some("workspace") {
                    // This is the public API's opaque identity for an explicitly
                    // selected root, not the managed default-workspace selector.
                    use sha2::{Digest, Sha256};
                    let id = format!("selected-workspace-{:x}", Sha256::digest(root.as_bytes()));
                    return resource.get("resource_id").and_then(Value::as_str) == Some(id.as_str());
                }
                if kind == Some("project_memory") {
                    use sha2::{Digest, Sha256};
                    let Some(owner) = resource.get("owner_id").and_then(Value::as_str) else { return false; };
                    let Ok(identity) = serde_json::to_vec(&(owner, root)) else { return false; };
                    let id = format!("project-workspace-memory-{:x}", Sha256::digest(identity));
                    return resource.get("resource_id").and_then(Value::as_str) == Some(id.as_str());
                }
            }
            resource.get("resource_id") == selection.get("resource_id")
        }).count() != 1
    }) {
        // Only fixture-owned resource kind labels appear in diagnostics, never
        // owner IDs, absolute paths, route records or provider credentials.
        let kind = match selection.get("resource_kind").and_then(Value::as_str) {
            Some("workspace") => "WORKSPACE", Some("process_session") => "PROCESS_SESSION",
            Some("project_memory") => "PROJECT_MEMORY", Some("browser") => "BROWSER",
            Some("computer") => "COMPUTER", Some("asset_library") => "ASSET_LIBRARY",
            Some("scheduler") => "SCHEDULER", _ => "OTHER",
        };
        return Err(SmokeFailure::new("session.resources", format!("SESSION_RESOURCE_SELECTION_MISMATCH_{kind}"), 409));
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
    create_session_in_workspace(router, preset_id, provider_id, model, resource_selections, None).await
}

async fn create_session_in_workspace(
    router: &Router,
    preset_id: &str,
    provider_id: &str,
    model: &str,
    resource_selections: Value,
    workspace: Option<&Path>,
) -> Result<(String, Value), SmokeFailure> {
    let mut body = json!({
        "preset_id": preset_id,
        "title": "Live Step Plan session",
        "resource_selections": resource_selections,
        "model": {"provider_id": provider_id, "model": model},
    });
    if let Some(workspace) = workspace {
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err(SmokeFailure::new("session.workspace", "LIVE_WORKSPACE_INVALID", 400));
        }
        body["workspace"] = json!(workspace.to_string_lossy());
    }
    let created = successful_json(
        router,
        "session.create",
        Method::POST,
        "/api/agent-sessions",
        Some(body),
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
    verify_resource_selections_in_workspace(&binding, &resource_selections, workspace)?;
    if let Some(workspace) = workspace {
        let projection = successful_json(router, "session.workspace", Method::GET,
            format!("/api/agent-sessions/{session_id}/projection"), None,
            LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let projection = envelope_data("session.workspace", projection)?;
        verify_fixture_workspace(&projection, workspace)?;
    }
    Ok((session_id, binding))
}

fn verify_fixture_workspace(projection: &Value, expected: &Path) -> Result<(), SmokeFailure> {
    let actual = projection.pointer("/extra/workspace").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("session.workspace", "LIVE_WORKSPACE_PROJECTION_MISSING", 502))?;
    let expected = std::fs::canonicalize(expected)
        .map_err(|_| SmokeFailure::new("session.workspace", "LIVE_WORKSPACE_INVALID", 400))?;
    let actual = std::fs::canonicalize(actual)
        .map_err(|_| SmokeFailure::new("session.workspace", "LIVE_WORKSPACE_PROJECTION_INVALID", 502))?;
    if expected != actual || projection.pointer("/extra/custom_workspace") != Some(&json!(true)) {
        return Err(SmokeFailure::new("session.workspace", "LIVE_WORKSPACE_BINDING_MISMATCH", 409));
    }
    Ok(())
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

async fn session_events(router: &Router, session_id: &str) -> Result<Vec<Value>, SmokeFailure> {
    let mut cursor = 0_u64;
    let mut events = Vec::new();
    for _ in 0..SESSION_MESSAGE_MAX_PAGES {
        let response = successful_json(router, "long_coding.events", Method::GET,
            format!("/api/agent-sessions/{session_id}/events?after_seq={cursor}&limit=500"),
            None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let page = envelope_data("long_coding.events", response)?;
        let next = page.pointer("/next_cursor/seq").and_then(Value::as_u64)
            .ok_or_else(|| SmokeFailure::new("long_coding.events", "SESSION_CURSOR_MISSING", 502))?;
        if next < cursor {
            return Err(SmokeFailure::new("long_coding.events", "SESSION_CURSOR_REGRESSED", 409));
        }
        let items = page.get("events").and_then(Value::as_array)
            .ok_or_else(|| SmokeFailure::new("long_coding.events", "SESSION_EVENTS_MISSING", 502))?;
        events.extend(items.iter().cloned());
        if next == cursor { return Ok(events); }
        cursor = next;
    }
    Err(SmokeFailure::new("long_coding.events", "SESSION_EVENT_PAGE_LIMIT_EXCEEDED", 503))
}

fn first_durable_error_code(messages: &[Value]) -> Option<String> {
    for message in messages {
        let Some(projection) = message.get("projection") else {
            continue;
        };
        let tool_summary = projection.get("tool_summary");
        let is_error = projection.get("status").and_then(Value::as_str) == Some("error")
            || projection.get("type").and_then(Value::as_str) == Some("error")
            || projection.get("error").is_some_and(|error|!error.is_null())
            || tool_summary.and_then(|summary| summary.get("error")).is_some_and(|error|!error.is_null());
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
        let output = projection.get("output").and_then(Value::as_str)
            .or_else(|| tool_summary.and_then(|summary| summary.get("error")).and_then(Value::as_str))
            .unwrap_or_default();
        let name = projection.get("name").and_then(Value::as_str)
            .or_else(|| tool_summary.and_then(|summary| summary.get("name")).and_then(Value::as_str));
        if name == Some("start_process")
            && output.contains("Capability Kernel rejected Agent Runtime Tool (INVALID_PAYLOAD)")
        {
            let args = projection.get("args").or_else(|| tool_summary.and_then(|summary| summary.get("args")));
            let code = if args.is_none() {
                "CODING_EXEC_ARGUMENTS_NOT_PROJECTED"
            } else if args.is_some_and(|args| args.get("env").is_some_and(Value::is_null)) {
                "CODING_EXEC_NULL_ENV"
            } else if args.is_some_and(|args| args.get("args").is_some_and(Value::is_null)) {
                "CODING_EXEC_NULL_ARGS"
            } else if args.is_some_and(|args| args["command"] != "git" || args["args"] != json!(["--version"])) {
                "CODING_EXEC_COMMAND_SHAPE_MISMATCH"
            } else if args.is_some_and(|args| !object_has_only_keys(args, &["operation", "command", "args", "timeout_ms"])) {
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
            match name {
                Some("update_plan") => "CODING_PLAN_REJECTED",
                Some("report_completion") => "CODING_COMPLETION_REJECTED",
                Some("write_file") => "CODING_WRITE_REJECTED",
                Some("read_file") => "CODING_READ_REJECTED",
                Some("apply_patch") => "CODING_PATCH_REJECTED",
                Some("start_process") => "CODING_EXEC_REJECTED",
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
            ("source exceeds the remaining bounded compaction budget", "CODING_COMPACTION_SOURCE_BUDGET"),
            ("per-turn compaction call budget exhausted", "CODING_COMPACTION_CALL_BUDGET"),
            ("compaction summary must retain identity", "CODING_COMPACTION_SUMMARY_INVALID"),
            ("Mandatory instructions/task state/accepted inputs and pending images exceed", "CODING_COMPACTION_MANDATORY_BUDGET"),
            ("summary request exceeds its input budget", "CODING_COMPACTION_INPUT_BUDGET"),
            ("compaction cannot fit the retained request/instructions/tools", "CODING_COMPACTION_RETAINED_BUDGET"),
            ("compaction route attempted a Tool Call", "CODING_COMPACTION_TOOL_CALL"),
            ("Agent Runtime turn exceeded the model-step limit", "CODING_MODEL_STEP_LIMIT"),
            ("Agent Runtime context is", "CODING_CONTEXT_TOO_LARGE"),
            ("Agent Runtime model stream ended without a terminal event", "CODING_MODEL_TERMINAL_MISSING"),
            ("Agent Runtime turn panicked", "CODING_TURN_PANICKED"),
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
        Value::Object(values) => {
            let provider_http_code = values.get("providerDiagnostic")
                .and_then(|diagnostic| diagnostic.get("httpStatus"))
                .and_then(Value::as_u64)
                .and_then(|status| match status {
                    400 => Some("CODING_PROVIDER_HTTP_400"),
                    401 => Some("CODING_PROVIDER_HTTP_401"),
                    402 => Some("CODING_PROVIDER_HTTP_402"),
                    403 => Some("CODING_PROVIDER_HTTP_403"),
                    404 => Some("CODING_PROVIDER_HTTP_404"),
                    408 => Some("CODING_PROVIDER_HTTP_408"),
                    409 => Some("CODING_PROVIDER_HTTP_409"),
                    422 => Some("CODING_PROVIDER_HTTP_422"),
                    429 => Some("CODING_PROVIDER_HTTP_429"),
                    500 => Some("CODING_PROVIDER_HTTP_500"),
                    502 => Some("CODING_PROVIDER_HTTP_502"),
                    503 => Some("CODING_PROVIDER_HTTP_503"),
                    504 => Some("CODING_PROVIDER_HTTP_504"),
                    _ => None,
                });
            provider_http_code.or_else(|| values.values().find_map(admission_conflict_code))
        },
        Value::Array(values) => values.iter().find_map(admission_conflict_code),
        _ => None,
    }
}

async fn emit_live_turn_failure_trace(root: &Path) {
    let db_path = root.join("data").join("nomifun-backend.db");
    let Ok(pool) = nomifun_db::sqlx::SqlitePool::connect(&format!("sqlite://{}", db_path.display())).await else {
        return;
    };
    let rows: Result<Vec<Option<String>>, _> = nomifun_db::sqlx::query_scalar(
        "SELECT inline_json FROM agent_events WHERE kind = 'turn/failed' ORDER BY seq LIMIT 8"
    ).fetch_all(&pool).await;
    if let Ok(rows) = rows {
        for (index, row) in rows.into_iter().enumerate() {
            let Some(value) = row.and_then(|row| serde_json::from_str::<Value>(&row).ok()) else { continue };
            let code = value.pointer("/error/code").and_then(Value::as_str)
                .map(str::to_owned).unwrap_or_else(|| "NONE".to_owned());
            let diagnosis = admission_conflict_code(&value).unwrap_or("NONE");
            let detail = value.pointer("/error/detail").and_then(Value::as_str).unwrap_or_default();
            let safe_detail = detail.split_whitespace().take(24).map(|word| {
                if word.len() > 20 || word.contains(['\\', '/', '@', '='])
                    || (word.starts_with("sk-") && word.len() > 6) {
                    return "REDACTED".to_owned();
                }
                word.chars().filter(|ch| ch.is_ascii_alphanumeric()
                    || matches!(ch, ':' | '.' | '(' | ')' | '-' | '_'))
                    .collect::<String>()
            }).collect::<Vec<_>>().join("_");
            eprintln!("NOMIFUN_LIVE_SMOKE_TURN_FAILURE index={index} code={} diagnosis={diagnosis} steps={} detail={safe_detail}",
                sanitize_code(code), value.get("model_steps").and_then(Value::as_u64).unwrap_or(0));
        }
    }
    pool.close().await;
}

fn collaboration_argument_shape(arguments: &Value) -> String {
    let kind = |value: Option<&Value>| match value {
        None => "missing", Some(Value::Null) => "null", Some(Value::Bool(_)) => "boolean",
        Some(Value::String(_)) => "string", Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object", Some(Value::Number(_)) => "number",
    };
    let strategy = match arguments.get("strategy").and_then(Value::as_str) {
        Some("parallel") => "parallel", Some("planned") => "planned", _ => "other",
    };
    format!("{strategy}:{}", kind(arguments.get("tasks")))
}

async fn emit_live_runtime_progress_trace(root: &Path) {
    let db_path = root.join("data").join("nomifun-backend.db");
    let Ok(pool) = nomifun_db::sqlx::sqlite::SqlitePoolOptions::new().max_connections(1)
        .connect_with(nomifun_db::sqlx::sqlite::SqliteConnectOptions::new().filename(db_path).read_only(true)).await else {
        return;
    };
    let rows: Result<Vec<(String,String)>, _> = nomifun_db::sqlx::query_as(
        "SELECT session_id,event_id FROM agent_events WHERE kind = 'runtime/progress-recorded' ORDER BY session_id,seq"
    ).fetch_all(&pool).await;
    if let Ok(rows) = rows {
        let Ok(store) = nomifun_agent_session::AgentSessionStore::from_pool(pool.clone()).await else { pool.close().await; return; };
        let mut grouped = std::collections::BTreeMap::<String,Vec<nomifun_agent_contracts::EventId>>::new();
        for (session,event) in rows { grouped.entry(session).or_default().push(event.into()); }
        let mut rows = Vec::new();
        for (session,ids) in grouped {
            let Ok(payloads) = store.resolved_event_payloads(&session.into(),&ids).await else { continue; };
            rows.extend(ids.into_iter().filter_map(|id|payloads.get(id.as_ref()).cloned()));
        }
        drop(store);
        let mut steps = 0_u32;
        let mut compact_calls = 0_u32;
        let mut compacted = 0_u32;
        let mut degraded = 0_u32;
        let mut reads = 0_u32;
        let mut execs = 0_u32;
        let mut writes = 0_u32;
        let mut reports = 0_u32;
        let mut invalid_arguments = 0_u32;
        let mut unavailable_names = 0_u32;
        let mut kernel_rejections = 0_u32;
        let mut delegations = 0_u32;
        let mut text_events = 0_u32;
        let mut event_counts = std::collections::BTreeMap::<String,u32>::new();
        let mut proposed_tools = std::collections::BTreeMap::<&'static str,u32>::new();
        let mut rejections = std::collections::BTreeMap::<String,u32>::new();
        let mut runtime_gate_prefixes = std::collections::BTreeMap::<&'static str,u32>::new();
        let mut control_calls = std::collections::BTreeMap::<String, &'static str>::new();
        let mut control_errors = Vec::<String>::new();
        let mut proposed_shapes = std::collections::BTreeMap::<String, String>::new();
        let mut rejected_shapes = Vec::new();
        for value in rows {
            let event = value.pointer("/event/event").and_then(Value::as_str).unwrap_or_default();
            if !event.is_empty() && event.len() <= 64 && event.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'_') {
                *event_counts.entry(event.to_owned()).or_default() += 1;
            }
            if event == "model_response_rejected" {
                if let Some(reason) = value.pointer("/event/rejection/kind").and_then(Value::as_str)
                    .filter(|reason| reason.len()<=64 && reason.bytes().all(|byte|byte.is_ascii_lowercase() || byte==b'_')) {
                    *rejections.entry(reason.to_owned()).or_default() += 1;
                }
            }
            match event {
                "model_step_started" => steps += 1,
                "compaction_started" => compact_calls += 1,
                "context_compacted" => {
                    compacted += 1;
                    if value.pointer("/event/summary").and_then(Value::as_str)
                        .is_some_and(|summary| summary.contains("Automatic summary incomplete")) {
                        degraded += 1;
                    }
                }
                "completion_reported" => reports += 1,
                "output_text_delta" => text_events += 1,
                "tool_call_completed" => {
                    let name = value.pointer("/event/call/name").and_then(Value::as_str);
                    let label = match name {
                        Some("ToolSearch") => "tool_search",
                        Some("read_file") => "read_file", Some("write_file") => "write_file",
                        Some("start_process") => "start_process", Some("update_plan") => "update_plan",
                        Some("report_completion") => "report_completion", Some("request_user_input") => "request_user_input",
                        Some("finish_turn") => "finish_turn", _ => "other",
                    };
                    *proposed_tools.entry(label).or_default() += 1;
                    let call_id = value.pointer("/event/call/call_id").and_then(Value::as_str);
                    if let (Some(name @ ("update_plan" | "report_completion")), Some(call_id)) = (name, call_id) {
                        control_calls.insert(call_id.to_owned(), if name == "update_plan" { "PLAN" } else { "REPORT" });
                    }
                    if let (Some(name), Some(call_id)) = (name, call_id)
                        && name.contains("agent_collaboration") {
                        proposed_shapes.insert(call_id.to_owned(), collaboration_argument_shape(&value["event"]["call"]["arguments"]));
                    }
                }
                "tool_completed" => {
                    let call_id = value.pointer("/event/result/call_id").and_then(Value::as_str);
                    if value.pointer("/event/result/is_error").and_then(Value::as_bool) == Some(true) {
                        let response = value.pointer("/event/result/output/0/text").and_then(Value::as_str).unwrap_or_default();
                        let prefix = if response.starts_with("Not executed: declare the NEXT source-bound exact_actions commitment") { Some("next_exact_action_required") }
                            else if response.starts_with("Not executed: this explicit multi-item task") { Some("numbered_task_plan_required") }
                            else { None };
                        if let Some(prefix) = prefix { *runtime_gate_prefixes.entry(prefix).or_default() += 1; }
                        if response.contains("INVALID_TOOL_ARGUMENTS") {
                            invalid_arguments += 1;
                            if let Some(shape) = call_id.and_then(|id| proposed_shapes.get(id))
                                && rejected_shapes.len() < 6 { rejected_shapes.push(shape.clone()); }
                        }
                        if response.contains("not exposed in the current model tool definitions") { unavailable_names += 1; }
                        if response.contains("Capability Kernel rejected") { kernel_rejections += 1; }
                    }
                    if let Some(name) = call_id.and_then(|id| control_calls.get(id))
                        && value.pointer("/event/result/is_error").and_then(Value::as_bool) == Some(true) {
                        let response = value.pointer("/event/result/output/0/text").and_then(Value::as_str).unwrap_or_default();
                        control_errors.push(format!("{name}:{}", control_error_code(response)));
                        if control_errors.len() > 12 { control_errors.remove(0); }
                    }
                }
                "tool_started" => match value.pointer("/event/action_id").and_then(Value::as_str) {
                    Some("workspace.files/read") => reads += 1,
                    Some("workspace.process/start") => execs += 1,
                    Some("workspace.files/write" | "workspace.files/patch") => writes += 1,
                    Some("agent/delegate" | "agent/fork") => delegations += 1,
                    _ => {}
                },
                _ => {}
            }
        }
        eprintln!("NOMIFUN_LIVE_SMOKE_RUNTIME_PROGRESS steps={steps} compact_calls={compact_calls} compacted={compacted} degraded={degraded} reads={reads} execs={execs} writes={writes} reports={reports}");
        eprintln!("NOMIFUN_LIVE_SMOKE_CONTROL_ERRORS sequence={}",
            if control_errors.is_empty() { "NONE".to_owned() } else { control_errors.join(",") });
        eprintln!("NOMIFUN_LIVE_SMOKE_TOOL_REJECTIONS invalid_arguments={invalid_arguments} unavailable_names={unavailable_names} kernel_rejections={kernel_rejections} delegations={delegations} text_events={text_events}");
        eprintln!("NOMIFUN_LIVE_SMOKE_COLLAB_ARGUMENT_SHAPES sequence={}",
            if rejected_shapes.is_empty() { "NONE".into() } else { rejected_shapes.join(",") });
        eprintln!("NOMIFUN_LIVE_SMOKE_CANONICAL_COUNTS events={} proposals={} rejections={} observed_gate_prefixes={}",
            json!(event_counts),json!(proposed_tools),json!(rejections),json!(runtime_gate_prefixes));
    }
    pool.close().await;
}

fn control_error_code(response: &str) -> &'static str {
    if response.contains("Invalid plan:") { "PLAN_SCHEMA" }
    else if response.contains("Requirement source input") { "REQUIREMENT_INPUT_INDEX" }
    else if response.contains("Source quote does not occur") { "REQUIREMENT_QUOTE_MISMATCH" }
    else if response.contains("accepted input sources") { "REQUIREMENT_SOURCE" }
    else if response.contains("Existing requirements are immutable") { "REQUIREMENT_REWRITE" }
    else if response.contains("Plan already has these step statuses") { "PLAN_NOOP" }
    else if response.contains("every accepted input") { "REQUIREMENT_COVERAGE" }
    else if response.contains("current plan steps") { "PLAN_STEPS" }
    else if response.contains("plan step") { "PLAN_OPEN" }
    else if response.contains("Evidence is failed") { "EVIDENCE_STALE" }
    else if response.starts_with("Invalid completion report:") { "REPORT_SCHEMA" }
    else { "OTHER" }
}

#[test]
fn live_control_diagnostics_use_fixed_categories_only() {
    assert_eq!(control_error_code("Source quote does not occur in that accepted input"),
        "REQUIREMENT_QUOTE_MISMATCH");
    assert_eq!(control_error_code("Invalid plan: missing field `explanation`"),
        "PLAN_SCHEMA");
    assert_eq!(control_error_code("private diagnostic SHOULD_NOT_EMIT"), "OTHER");
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

// Bounded, content-free diagnostics for live coding acceptance. The fixture
// directory and Provider credential never appear in this line.
fn emit_live_coding_trace(phase: &str, messages: &[Value], file: &Path) {
    let mut read = 0_u16;
    let mut write = 0_u16;
    let mut patch = 0_u16;
    let mut exec = 0_u16;
    let mut plan = 0_u16;
    let mut completion = 0_u16;
    let mut tool_errors = 0_u16;
    let mut flow = String::new();
    for message in messages {
        if message.get("presentation_intent").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        let projection = &message["projection"];
        let name = projection.pointer("/tool_summary/name").and_then(Value::as_str)
            .or_else(|| projection.get("name").and_then(Value::as_str));
        match name {
            Some("read_file") => read = read.saturating_add(1),
            Some("write_file") => write = write.saturating_add(1),
            Some("apply_patch") => patch = patch.saturating_add(1),
            Some("start_process") => exec = exec.saturating_add(1),
            Some("update_plan") => plan = plan.saturating_add(1),
            Some("report_completion") => completion = completion.saturating_add(1),
            _ => {}
        }
        if flow.len() < 96 {
            let marker = match name {
                Some("read_file") => 'R',
                Some("write_file") => 'W',
                Some("apply_patch") => 'P',
                Some("start_process") => 'E',
                Some("update_plan") => 'U',
                Some("report_completion") => 'C',
                _ => 'O',
            };
            flow.push(marker);
            if marker == 'E' {
                let exit_code = projection.get("output").and_then(Value::as_str)
                    .and_then(|output| serde_json::from_str::<Value>(output).ok())
                    .and_then(|output| output.get("exit_code").and_then(Value::as_i64));
                flow.push(match exit_code { Some(0) => '0', Some(_) => '1', None => 'x' });
            }
        }
        if projection.get("status").and_then(Value::as_str) == Some("error")
            || projection.get("error").is_some()
        {
            tool_errors = tool_errors.saturating_add(1);
        }
    }
    let content = std::fs::read_to_string(file).ok();
    let lower = content.as_ref().map(|text| text.to_ascii_lowercase());
    let has = |needle: &str| lower.as_ref().is_some_and(|text| text.contains(needle));
    let final_replies = messages.iter().filter_map(assistant_text_projection).count();
    eprintln!(
        "NOMIFUN_LIVE_SMOKE_CODING_TRACE phase={phase} read={read} write={write} patch={patch} exec={exec} plan={plan} completion={completion} tool_errors={tool_errors} final_replies={final_replies} file_exists={} file_bytes={} html={} script={} canvas={} keydown={}",
        content.is_some(), content.as_ref().map_or(0, String::len),
        has("<html"), has("<script"), has("canvas"), has("keydown"),
    );
    eprintln!("NOMIFUN_LIVE_SMOKE_CODING_FLOW phase={phase} flow={flow}");
}

async fn emit_live_coding_history_trace(
    router: &Router,
    session_id: &str,
    phase: &'static str,
) -> Result<(u16, u16, u16), SmokeFailure> {
    let response = successful_json(router, phase, Method::GET,
        format!("/api/agent-sessions/{session_id}/message-history?page_size=500"),
        None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let page = envelope_data(phase, response)?;
    let items = page.get("items").and_then(Value::as_array).ok_or_else(||
        SmokeFailure::new(phase, "LONG_CODING_HISTORY_MISSING", 502))?;
    let mut reads_instruction = 0_u16;
    let mut reads_source = 0_u16;
    let mut reads_tests = 0_u16;
    let mut reads_other = 0_u16;
    let mut errors = 0_u16;
    let mut invalid_payload = 0_u16;
    let mut not_found = 0_u16;
    let mut scope_rejected = 0_u16;
    let mut capability_unavailable = 0_u16;
    let mut admission = 0_u16;
    let mut process_error = 0_u16;
    let mut command_exit = 0_u16;
    let mut tool_search = 0_u16;
    let mut discovery_revealed_write = 0_u16;
    let mut other_tools = 0_u16;
    let mut exec_failed = 0_u16;
    let mut exec_succeeded = 0_u16;
    let mut exec_unknown = 0_u16;
    let mut test_launches = 0_u16;
    let mut failure_names = Vec::new();
    let mut terminal_diagnosis = "NONE";
    for item in items {
        let content = &item["content"];
        if content.get("error").is_some() {
            if let Some(code) = admission_conflict_code(content) {
                terminal_diagnosis = code;
            }
        }
        let Some(name) = content.get("name").and_then(Value::as_str) else { continue };
        let output = content.get("output").and_then(Value::as_str).unwrap_or_default();
        if content.get("status").and_then(Value::as_str) == Some("error") {
            errors = errors.saturating_add(1);
            if failure_names.len() < 12 {
                let name = match name {
                    "read_file" | "write_file" | "apply_patch" | "start_process" | "poll_process" | "git_status" | "git_diff"
                    | "search_files" | "update_plan" | "report_completion" => name,
                    _ => "other",
                };
                failure_names.push(name);
            }
        }
        if output.contains("INVALID_PAYLOAD") {
            invalid_payload = invalid_payload.saturating_add(1);
        }
        if output.contains("RESOURCE_NOT_FOUND") || output.contains("not found") {
            not_found = not_found.saturating_add(1);
        }
        if output.contains("INSTRUCTION_SCOPE") || output.contains("instruction scope") {
            scope_rejected = scope_rejected.saturating_add(1);
        }
        if output.contains("CAPABILITY_UNAVAILABLE") || output.contains("capability unavailable") {
            capability_unavailable = capability_unavailable.saturating_add(1);
        }
        if output.contains("admission") || output.contains("ADMISSION") {
            admission = admission.saturating_add(1);
        }
        if output.contains("process") || output.contains("Process") {
            process_error = process_error.saturating_add(1);
        }
        if output.contains("exit_code") || output.contains("Exit code") {
            command_exit = command_exit.saturating_add(1);
        }
        if name == "ToolSearch" {
            tool_search = tool_search.saturating_add(1);
            if output.contains("write_file") || output.contains("apply_patch") {
                discovery_revealed_write = discovery_revealed_write.saturating_add(1);
            }
        } else if !matches!(name, "read_file" | "write_file" | "apply_patch" | "start_process" | "update_plan" | "report_completion") {
            other_tools = other_tools.saturating_add(1);
        }
        if name == "read_file" {
            let args = &content["args"];
            let path = args.get("path").and_then(Value::as_str).unwrap_or_default();
            if args.get("format").and_then(Value::as_str) == Some("instruction_scope") {
                reads_instruction = reads_instruction.saturating_add(1);
            } else if path.ends_with("ledger.js") || path.ends_with("ranking.js") {
                reads_source = reads_source.saturating_add(1);
            } else if path.ends_with(".test.js") {
                reads_tests = reads_tests.saturating_add(1);
            } else {
                reads_other = reads_other.saturating_add(1);
            }
        }
        if matches!(name, "start_process") {
            let args = &content["args"];
            if matches!(args.get("command").and_then(Value::as_str), Some("bun" | "bun.exe"))
                && args.get("args").and_then(Value::as_array)
                    .and_then(|args| args.first()).and_then(Value::as_str) == Some("test") {
                test_launches = test_launches.saturating_add(1);
            }
        }
        if matches!(name, "start_process" | "poll_process") {
            let exit_code = serde_json::from_str::<Value>(output).ok()
                .and_then(|value| value.get("exit_code").and_then(Value::as_i64));
            match exit_code {
                Some(0) => exec_succeeded = exec_succeeded.saturating_add(1),
                Some(_) => exec_failed = exec_failed.saturating_add(1),
                None => exec_unknown = exec_unknown.saturating_add(1),
            }
        }
    }
    eprintln!("NOMIFUN_LIVE_SMOKE_CODING_HISTORY phase={phase} instruction={reads_instruction} source={reads_source} tests={reads_tests} other={reads_other} errors={errors} invalid_payload={invalid_payload} not_found={not_found} scope_rejected={scope_rejected} capability_unavailable={capability_unavailable} admission={admission} process_error={process_error} command_exit={command_exit} tool_search={tool_search} discovery_revealed_write={discovery_revealed_write} other_tools={other_tools} exec_failed={exec_failed} exec_succeeded={exec_succeeded} exec_unknown={exec_unknown} test_launches={test_launches}");
    eprintln!("NOMIFUN_LIVE_SMOKE_CODING_FAILURES phase={phase} names={} diagnosis={terminal_diagnosis}", failure_names.join(","));
    Ok((exec_failed, exec_succeeded, test_launches))
}

async fn wait_for_session_marker(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    after_seq: u64,
    marker: &'static str,
    duration: Duration,
) -> Result<(), SmokeFailure> {
    wait_for_session_marker_with_tools(router,phase,session_id,after_seq,marker,duration,None).await
}

fn session_marker_tools_match(messages: &[Value], expected_action: Option<&str>) -> bool {
    let tools: Vec<_> = messages.iter().filter(|message|
        message.get("presentation_intent").and_then(Value::as_str) == Some("tool")).collect();
    match expected_action {
        None => tools.is_empty(),
        Some(action) => tools.len() == 1
            && tools[0].pointer("/projection/state").and_then(Value::as_str) == Some("recorded")
            && tools[0].pointer("/projection/tool_summary/action_id").and_then(Value::as_str) == Some(action)
            && tools[0].pointer("/projection/tool_summary/result_state").and_then(Value::as_str) == Some("recorded")
            && tools[0].pointer("/projection/tool_summary/result_digest").and_then(Value::as_str)
                .is_some_and(|digest| digest.len() == 64 && digest.bytes().all(|byte|byte.is_ascii_hexdigit()))
            && tools[0].pointer("/projection/tool_summary/error").is_none_or(Value::is_null),
    }
}

async fn wait_for_session_marker_with_tools(
    router: &Router, phase: &'static str, session_id: &str, after_seq: u64,
    marker: &'static str, duration: Duration, expected_action: Option<&str>,
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
        let unexpected_tools = !session_marker_tools_match(&messages,expected_action);
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

async fn session_binding(
    router: &Router,
    phase: &'static str,
    session_id: &str,
) -> Result<Value, SmokeFailure> {
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
    observation.pointer("/session/agent_binding").filter(|binding|binding.is_object()).cloned()
        .ok_or_else(||SmokeFailure::new(phase,"SESSION_FROZEN_BINDING_MISSING",StatusCode::CONFLICT.as_u16()))
}

async fn assert_session_binding(router:&Router,phase:&'static str,session_id:&str,expected:&Value) -> Result<(),SmokeFailure> {
    if &session_binding(router,phase,session_id).await? != expected {
        return Err(SmokeFailure::new(
            phase,
            "SESSION_FROZEN_BINDING_CHANGED",
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

async fn collaboration_projection(router: &Router, session_id: &str, timeout: Duration) -> Result<Value, SmokeFailure> {
    let response = successful_json(
        router, "cluster.link", Method::GET,
        format!("/api/agent-sessions/{session_id}/projection"), None,
        timeout, &[StatusCode::OK],
    ).await?;
    envelope_data("cluster.link", response)
}

async fn wait_for_collaboration_link(
    router: &Router,
    session_id: &str,
    previous: Option<&str>,
    timeout: Duration,
) -> Result<String, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(SmokeFailure::new("cluster.link", "CLUSTER_LEAD_LINK_DEADLINE_EXCEEDED", 408));
        }
        let projection = collaboration_projection(router, session_id, LOCAL_API_DEADLINE.min(remaining)).await?;
        if let Some(id) = projection.get("linked_execution_id").and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty() && Some(*id) != previous)
        {
            return Ok(id.to_owned());
        }
        tokio::time::sleep(POLL_INTERVAL.min(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
        )).await;
    }
}

async fn run_live_agent_collaboration(
    router: &Router,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let cursor = session_message_cursor(router, "cluster.cursor_before", session_id).await?;
    let before = collaboration_projection(router, session_id, LOCAL_API_DEADLINE).await?;
    let previous = before.get("linked_execution_id").and_then(Value::as_str);
    start_session_turn(
        router,
        "cluster.trigger_turn",
        session_id,
        &uuid::Uuid::now_v7().to_string(),
        format!(
            "Use a subagent now to exercise the real persistent collaboration runtime. Invoke the Agent collaboration delegation Action exactly once with strategy=parallel, one task named marker, and this exact task prompt: Do not call tools and do not ask a question. Reply with exactly {COLLABORATION_MODEL_MARKER} and no other text. After acceptance, join the exact child with agent_wait and agent_status, then report its verified result from this parent Turn."
        ),
    )
    .await?;
    // /turns acknowledges admission, not model/tool completion. Waiting for
    // a new durable link avoids a deterministic race without weakening the
    // subsequent execution/marker assertions or delegating a second time.
    let execution_id = wait_for_collaboration_link(
        router, session_id, previous, TURN_RESULT_DEADLINE,
    ).await?;
    wait_for_execution_marker(
        router,
        &execution_id,
        COLLABORATION_MODEL_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    // The parent was explicitly required to delegate once. Keep the child
    // execution and exact parent reply assertions, and require exactly that
    // settled Action instead of applying the no-tools marker policy here.
    wait_for_session_marker_with_tools(
        router,
        "cluster.lead_reply",
        session_id,
        cursor,
        COLLABORATION_MODEL_MARKER,
        TURN_RESULT_DEADLINE,
        Some("agent/delegate"),
    )
    .await?;
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

fn managed_component_evidence(events: &[Value], operation: &str) -> Result<Value, SmokeFailure> {
    let failure = || SmokeFailure::new("component.evidence", "COMPONENT_CANONICAL_EVIDENCE_INVALID", 422);
    let turn_start = events.iter().find(|event| event["kind"] == "turn/started"
        && event["correlation_id"] == operation).ok_or_else(failure)?;
    if !events.iter().any(|event| event["kind"] == "turn/completed" && event["correlation_id"] == operation) {
        return Err(failure());
    }
    let starts = events.iter().filter(|event| event["kind"] == "tool/call-started"
        && event["causation_event_id"] == turn_start["event_id"]).collect::<Vec<_>>();
    let [start] = starts.as_slice() else { return Err(failure()); };
    if start.pointer("/payload/value/action_id").and_then(Value::as_str) != Some("workspace.process/start")
        || start.pointer("/payload/value/name").and_then(Value::as_str) != Some("start_process") {
        return Err(failure());
    }
    let call_id = start.pointer("/payload/value/call_id").and_then(Value::as_str).ok_or_else(failure)?;
    let proposal = events.iter().find(|event| event["kind"] == "runtime/progress-recorded"
        && event["correlation_id"] == operation
        && event.pointer("/payload/value/event/event").and_then(Value::as_str) == Some("tool_call_completed")
        && event.pointer("/payload/value/event/call/call_id").and_then(Value::as_str) == Some(call_id))
        .ok_or_else(failure)?;
    let arguments = proposal.pointer("/payload/value/event/call/arguments").ok_or_else(failure)?;
    if arguments["component"] != "officecli" || arguments["args"] != json!(["--version"])
        || arguments.get("command").is_some() || arguments.get("cmd").is_some() {
        return Err(failure());
    }
    let result = events.iter().find(|event| event["kind"] == "tool/result-recorded"
        && event["causation_event_id"] == start["event_id"]
        && event.pointer("/payload/value/operation_id") == start.pointer("/payload/value/operation_id"))
        .ok_or_else(failure)?;
    let payload = &result["payload"]["value"];
    let native = serde_json::from_value::<nomifun_agent_runtime::AgentToolResult>(payload["output"].clone())
        .map_err(|_| failure())?;
    if !payload["error"].is_null() || native.is_error || native.call_id.as_ref() != call_id {
        return Err(failure());
    }
    let output = serde_json::from_str::<Value>(&native.output_text()).map_err(|_| failure())?;
    let identity = &output["component_identity"];
    let version = identity["version"].as_str().ok_or_else(failure)?;
    let digest = identity["digest"].as_str().ok_or_else(failure)?;
    if output["state"] != "exited" || output["success"] != true || output["exit_code"] != 0
        || output.pointer("/cleanup/reaped") != Some(&json!(true))
        || identity["component"] != "officecli" || identity["source"] != "managed"
        || version.is_empty() || version.len() > 64
        || !version.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        || digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || output.pointer("/output/text").and_then(Value::as_str).map(str::trim) != Some(version) {
        return Err(failure());
    }
    Ok(json!({"schema_version":1,"provider":"stepfun-plan","model":"step-5-preview",
        "component":"officecli","action_id":"workspace.process/start","version":version,"digest":digest,
        "source":"managed","canonical_component_argument":true,"exit_code":0,"cleanup_reaped":true,
        "call_started_seq":start["seq"],"result_seq":result["seq"]}))
}

async fn run_live_managed_component_chain(
    router: &Router, api_key: &str, model: &str,
) -> Result<(), SmokeFailure> {
    if model != "step-5-preview" {
        return Err(SmokeFailure::new("component.model", "COMPONENT_SMOKE_MODEL_INVALID", 400));
    }
    let provider_id = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset_id, _) = create_agent_preset(router, &provider_id, model,
        &[("workspace.process", &["workspace.process/start"], "process_session")]).await?;
    let (session_id, binding) = create_session(router, &preset_id, &provider_id, model,
        json!([{"resource_kind":"process_session","resource_id":"managed-process-session"}])).await?;
    assert_session_runtime(router, &session_id).await?;
    let cursor = session_message_cursor(router, "component.cursor_before", &session_id).await?;
    let operation = start_session_turn(router, "component.turn", &session_id,
        &uuid::Uuid::now_v7().to_string(), format!(
            "Call start_process exactly once with this JSON object: {{\"component\":\"officecli\",\"args\":[\"--version\"],\"wait_ms\":30000}}. Use the host-managed component target. Do not set command or cmd, do not use a shell or PATH executable, and do not call any other tool. After its actual successful exit, reply with exactly {COMPONENT_MARKER} and no other text."
        )).await?;
    wait_for_session_marker_with_tools(router, "component.reply", &session_id, cursor,
        COMPONENT_MARKER, TURN_RESULT_DEADLINE, Some("workspace.process/start")).await?;
    let mut evidence = managed_component_evidence(&session_events(router, &session_id).await?, &operation)?;
    let cursor = session_message_cursor(router, "component.continuation_cursor", &session_id).await?;
    let continuation = start_session_turn(router, "component.continuation_turn", &session_id,
        &uuid::Uuid::now_v7().to_string(), format!(
            "This is an ordinary follow-up in the same session. Do not call tools. Reply with exactly {COMPONENT_CONTINUATION_MARKER} and no other text."
        )).await?;
    wait_for_session_marker(router, "component.continuation_reply", &session_id, cursor,
        COMPONENT_CONTINUATION_MARKER, TURN_RESULT_DEADLINE).await?;
    let events = session_events(router, &session_id).await?;
    if !events.iter().any(|event| event["kind"] == "turn/completed" && event["correlation_id"] == continuation)
        || session_binding(router, "component.binding_after", &session_id).await? != binding {
        return Err(SmokeFailure::new("component.continuation", "COMPONENT_SESSION_CONTINUATION_INVALID", 422));
    }
    assert_session_runtime(router, &session_id).await?;
    evidence["completed_turns"] = json!(2);
    evidence["ordinary_continuation_no_tools"] = json!(true);
    evidence["same_session_binding"] = json!(true);
    eprintln!("NOMIFUN_LIVE_COMPONENT_EVIDENCE {evidence}");
    Ok(())
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
    // Global Skill/MCP choices compile an immutable Session-only variant.
    // Freeze the actual admitted definition, including Snapshot/resources and
    // version, rather than guessing it has the library Preset's identity.
    let original_binding = session_binding(router,"agent_switch.capture_original_binding",&session_id).await?;
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
    let collaboration_binding = session_binding(router,"agent_switch.capture_new_binding",&collaboration_session_id).await?;
    assert_session_binding(
        router,
        "agent_switch.original_binding",
        &session_id,
        &original_binding,
    )
    .await?;
    assert_session_binding(
        router,
        "agent_switch.new_binding",
        &collaboration_session_id,
        &collaboration_binding,
    )
    .await?;
    run_live_agent_collaboration(router, &collaboration_session_id).await?;
    run_live_autowork(router, &collaboration_session_id).await?;
    Ok(())
}

async fn run_live_workspace_file_chain(
    router: &Router,
    api_key: &str,
    model: &str,
    work_dir: &Path,
    official_coding: bool,
    snake_game: bool,
) -> Result<(), SmokeFailure> {
    Box::pin(run_live_workspace_file_chain_observed(
        router, api_key, model, work_dir, official_coding, snake_game, None, true,
    )).await
}

async fn run_live_workspace_file_chain_observed(
    router: &Router,
    api_key: &str,
    model: &str,
    work_dir: &Path,
    official_coding: bool,
    snake_game: bool,
    navigation: Option<&live_reasoning_lifecycle::Navigation>,
    require_reply_marker: bool,
) -> Result<(), SmokeFailure> {
    let provider_id = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let preset_id = if official_coding {
        let created = successful_json(
            router,
            "coding.preset",
            Method::POST,
            "/api/agent-presets/from-template/coding.codex".to_owned(),
            Some(json!({
                "reuse_existing": false,
                "display_name": "Live Coding Agent smoke",
                "model": {"provider_id": provider_id, "model": model}
            })),
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        ).await?;
        let preset = envelope_data("coding.preset", created)?;
        required_string("coding.preset", &preset, "/preset/preset_id", "CODING_PRESET_ID_MISSING")?
    } else {
        create_agent_preset(
            router,
            &provider_id,
            model,
            &[("workspace.files", &["workspace.files/read", "workspace.files/write"], "workspace")],
        ).await?.0
    };
    let selections = if official_coding {
        coding_resource_selections()
    } else {
        json!([{"resource_kind":"workspace","resource_id":"default-workspace"}])
    };
    let (session_id, _) = create_session_in_workspace(
        router,
        &preset_id,
        &provider_id,
        model,
        selections,
        Some(work_dir),
    ).await?;
    if let Some(navigation) = navigation {
        navigation.before_turn(&session_id).await?;
    }
    let file_name = if snake_game { "snake_game.html" } else { "session-smoke.txt" };
    let prompt = if snake_game {
        "写一个贪吃蛇的游戏。在项目根目录用 write_file 创建独立可运行的 snake_game.html，内含 HTML、CSS 和 JavaScript，并实现键盘方向控制、计分以及开始或重新开始。完成后检查文件并简要回复。".to_owned()
    } else {
        format!(
            "Create a workspace file named session-smoke.txt containing exactly {WORKSPACE_FILE_MARKER}. Use write_file, then verify the file exists with read_file. Finish the plan and completion account required by your runtime. End your reply with {WORKSPACE_FILE_MARKER}."
        )
    };
    let cursor = session_message_cursor(router, "file.cursor_before", &session_id).await?;
    start_session_turn(
        router,
        "file.turn",
        &session_id,
        &uuid::Uuid::now_v7().to_string(),
        prompt,
    ).await?;

    let deadline = tokio::time::Instant::now() + if snake_game { Duration::from_secs(360) } else { TURN_RESULT_DEADLINE };
    loop {
        let (messages, _) = session_messages_after(router, "file.messages", &session_id, cursor).await?;
        if let Some(code) = first_durable_error_code(&messages) {
            return Err(SmokeFailure::new("file.messages", code, 422));
        }
        let observed = successful_json(
            router,
            "file.session",
            Method::GET,
            format!("/api/agent-sessions/{session_id}"),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        ).await?;
        let observed = envelope_data("file.session", observed)?;
        if observed.pointer("/head/status").and_then(Value::as_str) == Some("ready") {
            // The terminal may commit between the preceding messages read
            // and the head read. Verify the post-terminal snapshot rather
            // than rejecting a stale pre-terminal message window.
            let (messages, _) = session_messages_after(router, "file.messages", &session_id, cursor).await?;
            if let Some(code) = first_durable_error_code(&messages) {
                return Err(SmokeFailure::new("file.messages", code, 422));
            }
            let file = work_dir.join(file_name);
            emit_live_coding_trace(if snake_game { "snake_game" } else if official_coding { "coding" } else { "file" }, &messages, &file);
            let content = std::fs::read_to_string(&file).ok();
            let valid_file = if snake_game {
                content.as_ref().is_some_and(|content| {
                    let lower = content.to_ascii_lowercase();
                    content.len() >= 1_000
                        && lower.contains("<html")
                        && lower.contains("<script")
                        && lower.contains("canvas")
                        && lower.contains("keydown")
                })
            } else {
                content.as_deref() == Some(WORKSPACE_FILE_MARKER)
            };
            if !valid_file {
                return Err(SmokeFailure::new("file.result", "WORKSPACE_FILE_MISSING_OR_WRONG", 422));
            }
            if snake_game {
                verify_game_script_syntax(content.as_deref().expect("validated game content"))?;
            }
            let root_listing = successful_json(
                router,
                "file.workspace_listing",
                Method::GET,
                format!("/api/agent-sessions/{session_id}/workspace?path=."),
                None,
                LOCAL_API_DEADLINE,
                &[StatusCode::OK],
            ).await?;
            let root_listing = envelope_data("file.workspace_listing", root_listing)?;
            if !root_listing.as_array().is_some_and(|entries| entries.iter().any(|entry|
                entry.get("name").and_then(Value::as_str) == Some(file_name)
            )) {
                return Err(SmokeFailure::new("file.workspace_listing", "WORKSPACE_FILE_NOT_LISTED", 422));
            }
            let write_recorded = messages.iter().any(|message| {
                message.get("presentation_intent").and_then(Value::as_str) == Some("tool")
                    && message.pointer("/projection/tool_summary/name").and_then(Value::as_str) == Some("write_file")
                    && message.pointer("/projection/state").and_then(Value::as_str) == Some("recorded")
            });
            if !write_recorded {
                return Err(SmokeFailure::new("file.messages", "WORKSPACE_WRITE_RECEIPT_MISSING", 422));
            }
            if !snake_game && !messages.iter().any(|message|
                message.get("presentation_intent").and_then(Value::as_str) == Some("tool")
                    && message.pointer("/projection/tool_summary/name").and_then(Value::as_str) == Some("read_file")
                    && message.pointer("/projection/state").and_then(Value::as_str) == Some("recorded")) {
                return Err(SmokeFailure::new("file.messages", "WORKSPACE_READBACK_RECEIPT_MISSING", 422));
            }
            verify_live_tool_detail(router, &session_id).await?;
            // Tool-using models may add a short explanation around the final
            // answer. The selected-model smoke checks exact text separately;
            // here we require a real terminal reply and verified file effect.
            if !messages.iter().filter_map(assistant_text_projection).any(|text|
                text.get("content").and_then(Value::as_str).is_some_and(|content|
                    !content.trim().is_empty() && (snake_game || !require_reply_marker || content.trim().ends_with(WORKSPACE_FILE_MARKER)))
            ) {
                return Err(SmokeFailure::new("file.messages", "WORKSPACE_FINAL_REPLY_MISSING", 422));
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new("file.result", "WORKSPACE_TURN_DEADLINE_EXCEEDED", 408));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

/// Inspect the exact real-provider write receipt through the product read API.
/// Only fixed failure codes leave this fixture; arguments and output are never logged.
async fn verify_live_tool_detail(router: &Router, session_id: &str) -> Result<(), SmokeFailure> {
    let history = successful_json(router, "file.tool_detail_history", Method::GET,
        format!("/api/agent-sessions/{session_id}/message-history?cursor=&page_size=100"),
        None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let history = envelope_data("file.tool_detail_history", history)?;
    let tool = history.get("items").and_then(Value::as_array).and_then(|items| items.iter().find(|message|
        message.get("type").and_then(Value::as_str) == Some("tool_call")
        && message.pointer("/content/name").and_then(Value::as_str) == Some("write_file")))
        .ok_or_else(|| SmokeFailure::new("file.tool_detail_history", "TOOL_DETAIL_WRITE_MISSING", 422))?;
    let turn = tool.pointer("/content/turn_id").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("file.tool_detail_history", "TOOL_DETAIL_TURN_MISSING", 422))?;
    let call = tool.pointer("/content/call_id").and_then(Value::as_str)
        .ok_or_else(|| SmokeFailure::new("file.tool_detail_history", "TOOL_DETAIL_CALL_MISSING", 422))?;
    for section in ["arguments", "output"] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("turn_id", turn).append_pair("call_id", call).append_pair("section", section).finish();
        let page = successful_json(router, "file.tool_detail", Method::GET,
            format!("/api/agent-sessions/{session_id}/tool-details?{query}"),
            None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let page = envelope_data("file.tool_detail", page)?;
        if page.get("turn_id").and_then(Value::as_str) != Some(turn)
            || page.get("call_id").and_then(Value::as_str) != Some(call)
            || page.get("section").and_then(Value::as_str) != Some(section)
            || page.get("availability").and_then(Value::as_str) != Some("available")
            || page.get("result_seq").and_then(Value::as_u64).is_none()
            || page.get("call_event_id").and_then(Value::as_str).is_none()
            || (section == "arguments" && page.get("content").and_then(Value::as_str).is_none_or(str::is_empty)) {
            return Err(SmokeFailure::new("file.tool_detail", "TOOL_DETAIL_SOURCE_MISMATCH", 422));
        }
    }
    eprintln!("NOMIFUN_LIVE_SMOKE_STAGE phase=file.tool_details status=pass");
    Ok(())
}

/// Independent parsing, not model self-assessment or execution of generated
/// code. The verifier sees only HTML on stdin and inherits the runner's
/// credential-free environment. No browser/gameplay acceptance is implied.
fn verify_game_script_syntax(html: &str) -> Result<(), SmokeFailure> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    const CHECK: &str = r#"
const vm = require('node:vm');
let html = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => { html += chunk; if (Buffer.byteLength(html) > 8388608) process.exit(2); });
process.stdin.on('end', () => {
  try {
    let count = 0;
    for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi)) {
      if (/\bsrc\s*=/i.test(match[1])) throw new Error('external script');
      if (!match[2].trim()) continue;
      const type = match[1].match(/\btype\s*=\s*['"]?([^'"\s>]+)/i)?.[1]?.toLowerCase();
      if (type && !['module','text/javascript','application/javascript'].includes(type)) continue;
      if (++count > 32) throw new Error('script count');
      if (type === 'module') new vm.SourceTextModule(match[2]);
      else new vm.Script(match[2]);
    }
    if (count === 0) throw new Error('no script');
  } catch { process.exitCode = 1; }
});
"#;
    let mut command = Command::new("node");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW for the verifier.
    }
    let mut child = command.args(["--no-warnings", "--experimental-vm-modules", "-e", CHECK])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|_| SmokeFailure::new("game.syntax", "NODE_VERIFIER_UNAVAILABLE", 500))?;
    let written = child.stdin.take().expect("piped stdin").write_all(html.as_bytes());
    if written.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(SmokeFailure::new("game.syntax", "SCRIPT_VERIFIER_INPUT_FAILED", 500));
    }
    let status = child.wait().map_err(|_| SmokeFailure::new("game.syntax", "SCRIPT_VERIFIER_FAILED", 500))?;
    if !status.success() {
        return Err(SmokeFailure::new("game.syntax", "GAME_SCRIPT_SYNTAX_INVALID", 422));
    }
    Ok(())
}

#[test]
fn game_syntax_verifier_checks_code_without_executing_it() {
    assert!(verify_game_script_syntax("<script>throw new Error('must not execute');</script>").is_ok());
    assert!(verify_game_script_syntax("<script>function broken( {</script>").is_err());
    assert!(verify_game_script_syntax("<script src='https://example.invalid/game.js'></script>").is_err());
    assert!(verify_game_script_syntax("<script type='module'>export const size = 20;</script>").is_ok());
}

async fn wait_for_live_coding_turn(
    router: &Router,
    session_id: &str,
    after_seq: u64,
    phase: &'static str,
) -> Result<(Vec<Value>, bool), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + LONG_CODING_TURN_DEADLINE;
    loop {
        let (messages, _) = session_messages_after(router, phase, session_id, after_seq).await?;
        let terminal_messages = messages.iter()
            .filter(|message| message.get("presentation_intent").and_then(Value::as_str) != Some("tool"))
            .cloned().collect::<Vec<_>>();
        if let Some(code) = first_durable_error_code(&terminal_messages) {
            return Err(SmokeFailure::new(phase, code, 422));
        }
        let observed = successful_json(router, phase, Method::GET,
            format!("/api/agent-sessions/{session_id}"), None,
            LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
        let observed = envelope_data(phase, observed)?;
        if observed.pointer("/head/status").and_then(Value::as_str) == Some("ready") {
            let replies = messages.iter().filter_map(assistant_text_projection)
                .filter(|reply| reply.get("content").and_then(Value::as_str)
                    .is_some_and(|text| !text.trim().is_empty())).count();
            let tools = messages.iter().filter(|message|
                message.get("presentation_intent").and_then(Value::as_str) == Some("tool")).count();
            let events = session_events(router, session_id).await?;
            let started = events.iter().filter(|event|
                event.get("kind").and_then(Value::as_str) == Some("turn/started"))
                .filter_map(|event| event.get("seq").and_then(Value::as_u64)).max();
            let terminal = events.iter().filter(|event|
                event.get("seq").and_then(Value::as_u64).is_some_and(|seq| started.is_some_and(|start| seq > start))
                    && matches!(event.get("kind").and_then(Value::as_str), Some("turn/completed" | "turn/failed" | "turn/cancelled")))
                .max_by_key(|event| event.get("seq").and_then(Value::as_u64).unwrap_or(0));
            let completed = terminal.is_some_and(|event|
                event.get("kind").and_then(Value::as_str) == Some("turn/completed"));
            let failed_turn = !completed;
            let terminal_code = terminal.and_then(|event| event.pointer("/payload/value/error/code"))
                .and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| "NONE".to_owned());
            let terminal_code = sanitize_code(terminal_code);
            let diagnosis = terminal.and_then(|event| event.pointer("/payload/value/error/detail"))
                .and_then(admission_conflict_code).unwrap_or("NONE");
            eprintln!("NOMIFUN_LIVE_SMOKE_CODING_TERMINAL phase={phase} replies={replies} tools={tools} failed_turn={failed_turn} code={terminal_code} diagnosis={diagnosis}");
            return Ok((messages, completed));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new(phase, "LONG_CODING_TURN_DEADLINE_EXCEEDED", 408));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn run_live_long_coding_chain(
    router: &Router,
    api_key: &str,
    model: &str,
    work_dir: &Path,
) -> Result<(), SmokeFailure> {
    const INITIAL_SOURCE: &str = r#"export function parseCsv(text) {
  return text.trim().split(/\r?\n/).map((line) => line.split(','));
}

export function summarizePaid(csv) {
  const rows = parseCsv(csv);
  const totals = new Map();
  for (const [customer, cents, status] of rows.slice(1)) {
    if (status !== 'paid') continue;
    totals.set(customer, (totals.get(customer) ?? 0) + Number(cents));
  }
  return [...totals].map(([customer, cents]) => ({ customer, cents, count: 1 }))
    .sort((a, b) => a.customer.localeCompare(b.customer));
}
"#;
    const FIRST_TESTS: &str = r#"import { test, expect } from 'bun:test';
import { parseCsv, summarizePaid } from '../src/ledger.js';

test('quoted commas and doubled quotes', () => {
  expect(parseCsv('customer,cents,status\n"North, Inc",105,paid\n"A ""B"" Shop",200,paid'))
    .toEqual([['customer','cents','status'],['North, Inc','105','paid'],['A "B" Shop','200','paid']]);
});
test('CRLF, final newline and empty quoted field', () => {
  expect(parseCsv('customer,cents,status\r\n"",0,paid\r\n'))
    .toEqual([['customer','cents','status'],['','0','paid']]);
});
test('paid amounts aggregate as integer cents and count paid rows', () => {
  expect(summarizePaid('customer,cents,status\n"North, Inc",105,paid\n"North, Inc",-5,paid\nSouth,200,void\nSouth,50,paid'))
    .toEqual([{customer:'North, Inc',cents:100,count:2},{customer:'South',cents:50,count:1}]);
});
test('invalid money and malformed quotes are rejected', () => {
  expect(() => summarizePaid('customer,cents,status\nNorth,2.5,paid')).toThrow();
  expect(() => parseCsv('customer,cents,status\n"unterminated,10,paid')).toThrow();
});
"#;
    const SECOND_TESTS: &str = r#"import { test, expect } from 'bun:test';
import { rankCustomers } from '../src/ranking.js';

const data = 'customer,cents,status\nZed,100,paid\nAmy,200,paid\nBea,200,paid\nAmy,-50,paid\nZed,999,void';
test('ranks by total descending and breaks ties by customer name', () => {
  expect(rankCustomers(data, 3)).toEqual([
    {customer:'Bea',cents:200,count:1},
    {customer:'Amy',cents:150,count:2},
    {customer:'Zed',cents:100,count:1},
  ]);
});
test('limit zero and invalid limits', () => {
  expect(rankCustomers(data, 0)).toEqual([]);
  expect(() => rankCustomers(data, -1)).toThrow();
  expect(() => rankCustomers(data, 1.5)).toThrow();
});
"#;

    let prepare = || -> Result<(), SmokeFailure> {
        for directory in [work_dir.join("src"), work_dir.join("tests")] {
            std::fs::create_dir_all(directory).map_err(|_| SmokeFailure::new(
                "long_coding.fixture", "LONG_CODING_DIRECTORY_CREATE_FAILED", 500))?;
        }
        for (name, content) in [
            ("src/ledger.js", INITIAL_SOURCE),
            ("tests/ledger.test.js", FIRST_TESTS),
            ("AGENTS.md", "# Ledger project instructions\nEdit only src/ and README.md. Keep supplied tests unchanged. Run bun test after edits. Do not commit.\n"),
            ("package.json", "{\"type\":\"module\",\"scripts\":{\"test\":\"bun test\"}}\n"),
            ("README.md", "# Ledger fixture\n\nRepair the CSV and paid-invoice logic.\n"),
        ] {
            std::fs::write(work_dir.join(name), content).map_err(|_| SmokeFailure::new(
                "long_coding.fixture", "LONG_CODING_FILE_CREATE_FAILED", 500))?;
        }
        for args in [["init", "-q"].as_slice(), ["add", "--all"].as_slice()] {
            let result = std::process::Command::new("git").args(args)
                .current_dir(work_dir).output()
                .map_err(|_| SmokeFailure::new("long_coding.fixture", "LONG_CODING_GIT_UNAVAILABLE", 503))?;
            if !result.status.success() {
                return Err(SmokeFailure::new("long_coding.fixture", "LONG_CODING_GIT_SETUP_FAILED", 503));
            }
        }
        Ok(())
    };
    prepare()?;
    let provider_id = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let created = successful_json(router, "long_coding.preset", Method::POST,
        "/api/agent-presets/from-template/coding.codex".to_owned(),
        Some(json!({"reuse_existing":false,"display_name":"Live long coding Agent",
            "model":{"provider_id":provider_id,"model":model}})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let preset = envelope_data("long_coding.preset", created)?;
    let preset_id = required_string("long_coding.preset", &preset, "/preset/preset_id",
        "LONG_CODING_PRESET_ID_MISSING")?;
    let (session_id, _) = create_session_in_workspace(router, &preset_id, &provider_id, model,
        coding_resource_selections(), Some(work_dir)).await?;

    let cursor = session_message_cursor(router, "long_coding.first.cursor", &session_id).await?;
    start_session_turn(router, "long_coding.first.turn", &session_id,
        &uuid::Uuid::now_v7().to_string(),
        "Fix src/ledger.js so tests/ledger.test.js passes while preserving its parseCsv and summarizePaid exports. Follow AGENTS.md and work directly without delegation. Run `bun test tests/ledger.test.js` before editing to observe the failing baseline, then edit only the source and rerun the same check. Do not edit tests or search for unrelated setup files. Finish with the actual test result and any limitation.".to_owned()).await?;
    let (first_messages, first_completed) = wait_for_live_coding_turn(router, &session_id, cursor,
        "long_coding.first.result").await?;
    let mut all_turns_completed = first_completed;
    emit_live_coding_trace("long_first", &first_messages, &work_dir.join("src/ledger.js"));
    let (failed_checks, successful_checks, test_launches) =
        emit_live_coding_history_trace(router, &session_id, "long_coding.first.history").await?;
    if failed_checks == 0 || successful_checks == 0 || test_launches < 2 {
        return Err(SmokeFailure::new("long_coding.first.history", "LONG_CODING_PROCESS_EVIDENCE_MISSING", 422));
    }
    if std::fs::read_to_string(work_dir.join("tests/ledger.test.js")).ok().as_deref() != Some(FIRST_TESTS) {
        return Err(SmokeFailure::new("long_coding.first.result", "LONG_CODING_TESTS_MODIFIED", 422));
    }
    let mut first_check = std::process::Command::new("bun").arg("test")
        .arg("tests/ledger.test.js").current_dir(work_dir).output()
        .map_err(|_| SmokeFailure::new("long_coding.first.check", "LONG_CODING_CHECK_UNAVAILABLE", 503))?;
    let first_cases = [
        ("quote", "quoted commas and doubled quotes"),
        ("newline", "CRLF, final newline and empty quoted field"),
        ("summary", "paid amounts aggregate as integer cents and count paid rows"),
        ("malformed", "invalid money and malformed quotes are rejected"),
    ];
    for recovery in 0..2 {
        let output = String::from_utf8_lossy(&first_check.stderr);
        let failed = first_cases.iter().filter_map(|(label, title)|
            output.contains(&format!("(fail) {title}")).then_some(*label)).collect::<Vec<_>>();
        eprintln!("NOMIFUN_LIVE_SMOKE_CODING_CHECK phase=long_first attempt={recovery} quote={} newline={} summary={} malformed={} syntax={} import={} assertion={} zero_tests={}",
            !failed.contains(&"quote"), !failed.contains(&"newline"),
            !failed.contains(&"summary"), !failed.contains(&"malformed"),
            output.contains("SyntaxError") || output.contains("syntax error"),
            output.contains("Cannot find module") || output.contains("export named"),
            output.contains("expect(received)") || output.contains("expect(") || output.contains("toEqual"),
            output.contains("0 tests") || output.contains("No tests found"));
        if first_check.status.success() { break; }
        let mut specifics = Vec::new();
        if failed.contains(&"quote") { specifics.push("A comma inside quotes and doubled quotes must be parsed correctly."); }
        if failed.contains(&"newline") { specifics.push("CRLF and a final newline must not add records."); }
        if failed.contains(&"summary") { specifics.push("North, Inc has two paid rows totaling 100 cents; void rows do not count."); }
        if failed.contains(&"malformed") { specifics.push("Both a fractional-cent amount and an unterminated quoted CSV field must throw."); }
        let cursor = session_message_cursor(router, "long_coding.repair.cursor", &session_id).await?;
        start_session_turn(router, "long_coding.repair.turn", &session_id,
            &uuid::Uuid::now_v7().to_string(),
            format!("The independent `bun test tests/ledger.test.js` check still exits nonzero. Failing cases: {}. {} Continue this same task directly; do not delegate or search for other setup files. The relevant files are AGENTS.md, src/ledger.js and tests/ledger.test.js. Correct src/ledger.js without editing supplied tests, rerun the check, and state what actually passed. Do not stop after a failed command.",
                if failed.is_empty() { "unclassified test-runner failure".to_owned() } else { failed.join(", ") }, specifics.join(" "))).await?;
        let (messages, completed) = wait_for_live_coding_turn(router, &session_id, cursor,
            "long_coding.repair.result").await?;
        all_turns_completed &= completed;
        emit_live_coding_trace("long_repair", &messages, &work_dir.join("src/ledger.js"));
        if std::fs::read_to_string(work_dir.join("tests/ledger.test.js")).ok().as_deref() != Some(FIRST_TESTS) {
            return Err(SmokeFailure::new("long_coding.repair.result", "LONG_CODING_TESTS_MODIFIED", 422));
        }
        first_check = std::process::Command::new("bun").arg("test")
            .arg("tests/ledger.test.js").current_dir(work_dir).output()
            .map_err(|_| SmokeFailure::new("long_coding.repair.check", "LONG_CODING_CHECK_UNAVAILABLE", 503))?;
    }
    if !first_check.status.success() {
        return Err(SmokeFailure::new("long_coding.first.check", "LONG_CODING_REPAIR_FAILED", 422));
    }

    std::fs::write(work_dir.join("tests/ranking.test.js"), SECOND_TESTS).map_err(|_| SmokeFailure::new(
        "long_coding.second.fixture", "LONG_CODING_FILE_CREATE_FAILED", 500))?;
    let cursor = session_message_cursor(router, "long_coding.second.cursor", &session_id).await?;
    start_session_turn(router, "long_coding.second.turn", &session_id,
        &uuid::Uuid::now_v7().to_string(),
        "Continue in this same coding session without delegation. I added tests/ranking.test.js; implement src/ranking.js to pass it while preserving the ledger repair. Do not edit either test file. Run the complete `bun test` suite and update README.md with the API and verification command. Finish with the observed result and any limitation.".to_owned()).await?;
    let (second_messages, second_completed) = wait_for_live_coding_turn(router, &session_id, cursor,
        "long_coding.second.result").await?;
    all_turns_completed &= second_completed;
    emit_live_coding_trace("long_second", &second_messages, &work_dir.join("src/ranking.js"));
    let _ = emit_live_coding_history_trace(router, &session_id, "long_coding.second.history").await?;
    if !second_messages.iter().filter_map(assistant_text_projection).any(|reply|
        reply.get("content").and_then(Value::as_str).is_some_and(|text| !text.trim().is_empty())) {
        return Err(SmokeFailure::new("long_coding.second.result", "LONG_CODING_FINAL_REPLY_MISSING", 422));
    }
    if std::fs::read_to_string(work_dir.join("tests/ledger.test.js")).ok().as_deref() != Some(FIRST_TESTS)
        || std::fs::read_to_string(work_dir.join("tests/ranking.test.js")).ok().as_deref() != Some(SECOND_TESTS) {
        return Err(SmokeFailure::new("long_coding.second.result", "LONG_CODING_TESTS_MODIFIED", 422));
    }
    if !std::fs::read_to_string(work_dir.join("README.md")).ok().is_some_and(|text|
        text.contains("rankCustomers") && text.contains("bun test")) {
        return Err(SmokeFailure::new("long_coding.second.result", "LONG_CODING_DOCUMENTATION_MISSING", 422));
    }
    let final_check = std::process::Command::new("bun").arg("test")
        .current_dir(work_dir).output()
        .map_err(|_| SmokeFailure::new("long_coding.second.check", "LONG_CODING_CHECK_UNAVAILABLE", 503))?;
    if !final_check.status.success() {
        return Err(SmokeFailure::new("long_coding.second.check", "LONG_CODING_FEATURE_FAILED", 422));
    }
    if !all_turns_completed {
        return Err(SmokeFailure::new("long_coding.terminal", "LONG_CODING_TURN_FAILED", 422));
    }
    Ok(())
}

async fn run_live_companion_chain(
    router: &Router,
    api_key: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    let provider_id = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let companion = successful_json(router, "companion.create", Method::POST,
        "/api/companion/companions".to_owned(),
        Some(json!({"name":"Live Companion smoke","character":"mochi"})),
        LOCAL_API_DEADLINE, &[StatusCode::CREATED]).await?;
    let companion = envelope_data("companion.create", companion)?;
    let companion_id = required_string("companion.create", &companion,
        "/companion_id", "COMPANION_ID_MISSING")?;
    successful_json(router, "companion.model", Method::PATCH,
        format!("/api/companion/companions/{companion_id}"),
        Some(json!({"model":{"provider_id":provider_id,"model":model}})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    successful_json(router, "companion.agent", Method::PUT,
        format!("/api/product-agent-bindings/companion/{companion_id}"),
        Some(json!({
            "selection":{"kind":"template","template_key":"companion.default"},
            "model":{"provider_id":provider_id,"model":model},
            "resource_selections":[
                {"resource_kind":"companion","resource_id":companion_id},
                {"resource_kind":"companion_memory","resource_id":companion_id},
                {"resource_kind":"scheduler","resource_id":"installation-scheduler"}
            ]
        })), LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let thread = successful_json(router, "companion.thread", Method::POST,
        format!("/api/companion/companions/{companion_id}/companion/threads"),
        Some(json!({})), LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let thread = envelope_data("companion.thread", thread)?;
    let session_id = required_string("companion.thread", &thread,
        "/conversation_id", "COMPANION_SESSION_ID_MISSING")?;
    let projection = successful_json(router, "companion.projection", Method::GET,
        format!("/api/agent-sessions/{session_id}/projection"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let projection = envelope_data("companion.projection", projection)?;
    if projection.pointer("/agent_snapshot/preset_name").and_then(Value::as_str)
        != Some("companion.default") {
        return Err(SmokeFailure::new("companion.projection", "COMPANION_PRESET_MISMATCH", 409));
    }
    let cursor = session_message_cursor(router, "companion.cursor_before", &session_id).await?;
    start_session_turn(router, "companion.turn", &session_id,
        &uuid::Uuid::now_v7().to_string(),
        format!("This is a Companion session acceptance check. Do not call tools. Reply with exactly {COMPANION_MARKER} and no other text.")).await?;
    wait_for_session_marker(router, "companion.reply", &session_id, cursor,
        COMPANION_MARKER, TURN_RESULT_DEADLINE).await
}

async fn run_live_creative_studio_chain(
    router: &Router,
    api_key: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    let provider_id = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let canvas = successful_json(router, "creative.canvas", Method::POST,
        "/api/creative-studio/canvases".to_owned(),
        Some(json!({
            "title":"Live Creative Studio smoke",
            "agentKickoff":{
                "prompt":"test",
                "model":{"providerId":provider_id,"model":model}
            }
        })), LOCAL_API_DEADLINE, &[StatusCode::CREATED]).await?;
    let canvas = envelope_data("creative.canvas", canvas)?;
    let canvas_id = required_string("creative.canvas", &canvas,
        "/canvas/canvasId", "CREATIVE_CANVAS_ID_MISSING")?;
    let detail = successful_json(router, "creative.canvas_detail", Method::GET,
        format!("/api/creative-studio/canvases/{canvas_id}"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let detail = envelope_data("creative.canvas_detail", detail)?;
    let session_id = required_string("creative.canvas_detail", &detail,
        "/document/chatSessions/0/id", "CREATIVE_SESSION_ID_MISSING")?;
    let pending_key = required_string("creative.canvas_detail", &detail,
        "/document/chatSessions/0/pendingTurn/idempotencyKey", "CREATIVE_PENDING_KEY_MISSING")?;
    let resolved = successful_json(router, "creative.resolve", Method::POST,
        "/api/creative-studio/canvas-agent-sessions/resolve".to_owned(),
        Some(json!({
            "canvas_id":canvas_id,
            "session_id":session_id,
            "model":{"provider_id":provider_id,"model":model},
            "pending_turn_idempotency_key":pending_key
        })), LOCAL_API_DEADLINE, &[StatusCode::CREATED]).await?;
    let resolved = envelope_data("creative.resolve", resolved)?;
    let conversation_id = required_string("creative.resolve", &resolved,
        "/binding/conversation_id", "CREATIVE_CONVERSATION_ID_MISSING")?;
    let projection = successful_json(router, "creative.projection", Method::GET,
        format!("/api/agent-sessions/{conversation_id}/projection"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let projection = envelope_data("creative.projection", projection)?;
    if projection.pointer("/agent_snapshot/preset_name").and_then(Value::as_str)
        != Some("creative-studio.default") {
        return Err(SmokeFailure::new("creative.projection", "CREATIVE_PRESET_MISMATCH", 409));
    }
    let cursor = session_message_cursor(router, "creative.cursor_before", &conversation_id).await?;
    start_session_turn(router, "creative.turn", &conversation_id,
        &uuid::Uuid::now_v7().to_string(),
        format!("This is a Creative Studio Agent session acceptance check. Do not call tools. Reply with exactly {CREATIVE_MARKER} and no other text.")).await?;
    wait_for_session_marker(router, "creative.reply", &conversation_id, cursor,
        CREATIVE_MARKER, TURN_RESULT_DEADLINE).await
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
    eprintln!("NOMIFUN_LIVE_SMOKE_BUILD digest={}",descriptor.build_digest);
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

struct UiFixtureShutdownFile {
    path: std::path::PathBuf,
    bytes: Vec<u8>,
}

struct UiFixtureShutdownFiles {
    app_count: usize,
    files: Vec<UiFixtureShutdownFile>,
}

fn shutdown_probe_has_link(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    { metadata.is_symlink() }
}

/// Probe only ordinary known application source files, never configuration or
/// credential stores. This separates production close from the outer test's
/// later TempDir destructor, which can partially reclaim a Windows directory.
async fn snapshot_ui_fixture_shutdown_files(router: &Router, root: &Path) -> Result<UiFixtureShutdownFiles, SmokeFailure> {
    let response = successful_json(router, "ui_fixture.shutdown_files", Method::GET, "/api/apps", None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let apps = envelope_data("ui_fixture.shutdown_files", response)?;
    let apps = apps.as_array().ok_or_else(|| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_APPS_INVALID", 500))?;
    let owned_root = std::fs::canonicalize(root.join("data/app-hosting/apps"))
        .map_err(|_| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_APP_ROOT_UNAVAILABLE", 500))?;
    let mut files = Vec::new();
    for app in apps {
        let id = required_string("ui_fixture.shutdown_files", app, "/id", "UI_FIXTURE_APP_ID_MISSING")?;
        uuid::Uuid::parse_str(&id).map_err(|_| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_APP_ID_INVALID", 500))?;
        let directory = std::path::PathBuf::from(required_string("ui_fixture.shutdown_files", app, "/directory", "UI_FIXTURE_APP_DIRECTORY_MISSING")?);
        let metadata = std::fs::symlink_metadata(&directory)
            .map_err(|_| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_APP_DIRECTORY_UNAVAILABLE", 500))?;
        if shutdown_probe_has_link(&metadata) || std::fs::canonicalize(&directory).ok().as_deref() != Some(owned_root.join(&id).as_path()) {
            return Err(SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_APP_DIRECTORY_OUTSIDE_OWNER", 500));
        }
        for relative in ["index.html", "snake-game/index.html", "server.cjs"] {
            let path = directory.join(relative);
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_SOURCE_METADATA_FAILED", 500)),
            };
            let mut checked = directory.clone();
            for component in Path::new(relative).components() {
                checked.push(component);
                if shutdown_probe_has_link(&std::fs::symlink_metadata(&checked)
                    .map_err(|_| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_SOURCE_METADATA_FAILED", 500))?) {
                    return Err(SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_SOURCE_LINK_REJECTED", 500));
                }
            }
            if !metadata.is_file() || !std::fs::canonicalize(&path).ok().is_some_and(|canonical| canonical.starts_with(owned_root.join(&id))) {
                return Err(SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_SOURCE_OUTSIDE_OWNER", 500));
            }
            let bytes = std::fs::read(&path)
                .map_err(|_| SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_SOURCE_READ_FAILED", 500))?;
            files.push(UiFixtureShutdownFile { path, bytes });
        }
    }
    Ok(UiFixtureShutdownFiles { app_count: apps.len(), files })
}

fn assert_ui_fixture_shutdown_files(snapshot: UiFixtureShutdownFiles) -> Result<(), SmokeFailure> {
    use sha2::{Digest, Sha256};
    let mut digests = Vec::new();
    for file in &snapshot.files {
        if std::fs::read(&file.path).ok().as_deref() != Some(file.bytes.as_slice()) {
            return Err(SmokeFailure::new("ui_fixture.shutdown_files", "UI_FIXTURE_PRODUCTION_CLOSE_CHANGED_SOURCE", 500));
        }
        digests.push(format!("{:x}", Sha256::digest(&file.bytes)));
    }
    eprintln!("NOMIFUN_LIVE_APP_WORKSPACE_SHUTDOWN_FILES_PRESERVED app_count={} file_count={} sha256={}",
        snapshot.app_count, snapshot.files.len(), digests.join(","));
    Ok(())
}

/// Each scenario owns its future on the heap. Keeping all scenario chains in
/// one async match inflates the driver frame and can overflow the test stack
/// before its first request; larger worker stacks would hide that coupling.
fn live_case_future<'a>(case: LiveCase, router: &'a Router, api_key: &'a str,
    model: &'a str, root: &'a Path, work_dir: &'a Path,
    application: &'a NomiCoreApplication, local_proof: Option<&'a str>,
) -> futures_util::future::LocalBoxFuture<'a, Result<(), SmokeFailure>> {
    match case {
        LiveCase::SubagentIntegration => live_subagent::run(router, api_key, model, root, work_dir).boxed_local(),
        LiveCase::SubagentEightIntegration => live_subagent_eight::run(router, api_key, model, root, work_dir).boxed_local(),
        LiveCase::ScheduledTasks => live_scheduled_tasks::run(router, api_key, model, root, work_dir).boxed_local(),
        LiveCase::WorkspaceFile | LiveCase::CodingPreset | LiveCase::SnakeGame => run_live_workspace_file_chain(
            router, api_key, model, work_dir,
            matches!(case, LiveCase::CodingPreset | LiveCase::SnakeGame), matches!(case, LiveCase::SnakeGame),
        ).boxed_local(),
        LiveCase::LongCoding => run_live_long_coding_chain(router, api_key, model, work_dir).boxed_local(),
        LiveCase::SelectedModel => run_selected_model_chain(router, api_key, model).boxed_local(),
        LiveCase::ManagedComponent => run_live_managed_component_chain(router, api_key, model).boxed_local(),
        LiveCase::UserInputHandoff => live_user_input::run(router, api_key, model, root).boxed_local(),
        LiveCase::ResourceWait => live_resource_wait::run(router, api_key, model, root).boxed_local(),
        LiveCase::ReasoningLifecycle => live_reasoning_lifecycle::run(router, api_key, model, work_dir).boxed_local(),
        LiveCase::Companion => run_live_companion_chain(router, api_key, model).boxed_local(),
        LiveCase::CreativeStudio => run_live_creative_studio_chain(router, api_key, model).boxed_local(),
        LiveCase::Idmm => live_idmm_demo::run(router, api_key, model).boxed_local(),
        LiveCase::AppHosting => live_app_hosting::run(router, application, api_key,
            local_proof.expect("application smoke has its isolated boot proof"), model).boxed_local(),
        LiveCase::AppWorkspaceUiFixture => live_app_hosting::run_ui_fixture(router, application, api_key, model).boxed_local(),
    }
}

fn retained_subagent_parent(path:&Path)->Result<std::path::PathBuf,SmokeFailure> {
    let invalid=||SmokeFailure::new("bootstrap","NATIVE_FIXTURE_PARENT_INVALID",400);
    let git=nomifun_common::paths::canonicalize_simplified(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.git")).map_err(|_|invalid())?;
    let parent=nomifun_common::paths::canonicalize_simplified(path).map_err(|_|invalid())?;
    if !path.is_absolute()||!git.is_dir()||!parent.is_dir()||parent==git||!parent.starts_with(&git) {return Err(invalid());}
    Ok(parent)
}

fn live_fixture_root(case:LiveCase)->Result<(TempDir,bool),SmokeFailure> {
    let retain=std::env::var_os("NOMIFUN_LIVE_RETAIN_NATIVE_FIXTURE");
    let result=if let Some(retain)=retain {
        if retain!="1"||!matches!(case,LiveCase::SubagentIntegration|LiveCase::SubagentEightIntegration) {
            return Err(SmokeFailure::new("bootstrap","NATIVE_FIXTURE_MODE_INVALID",400));
        }
        let parent=std::env::var_os("NOMIFUN_LIVE_FIXTURE_PARENT")
            .ok_or_else(||SmokeFailure::new("bootstrap","NATIVE_FIXTURE_PARENT_INVALID",400))?;
        let parent=retained_subagent_parent(Path::new(&parent))?;
        tempfile::Builder::new().prefix("subagent-native-").tempdir_in(parent).map(|root|(root,true))
    } else {tempfile::tempdir().map(|root|(root,false))};
    result.map_err(|_|SmokeFailure::new("bootstrap","TEMP_ROOT_CREATE_FAILED",500))
}

async fn run_live_provider_smoke(case: LiveCase) -> Result<(), SmokeFailure> {
    let model = live_model()?;
    let api_key = required_secret_from_stdin()?;
    let (root,retain_native)=live_fixture_root(case)?;
    let local_proof = matches!(case, LiveCase::AppHosting | LiveCase::AppWorkspaceUiFixture).then(|| uuid::Uuid::new_v4().to_string());
    let fixture = build_fixture(&root, local_proof.as_deref()).await?;
    // Only the internal test driver receives a proof automatically. The actual
    // HTTP listener in the app smoke uses the untouched authenticated router.
    let router = if let Some(proof) = &local_proof {
        let proof = axum::http::HeaderValue::from_str(proof)
            .map_err(|_| SmokeFailure::new("bootstrap", "TEST_LOCAL_PROOF_INVALID", 500))?;
        fixture.application.router().layer(axum::middleware::from_fn(move |mut request: Request<Body>, next: axum::middleware::Next| {
            let proof = proof.clone();
            async move {
                request.headers_mut().insert("x-nomi-local-trust", proof);
                next.run(request).await
            }
        }))
    } else { fixture.application.router() };
    let work_dir = nomifun_common::paths::canonicalize_simplified(&root.path().join("work"))
        .map_err(|_| SmokeFailure::new("bootstrap", "TEMP_WORKSPACE_CANONICALIZATION_FAILED", 500))?;
    let result = AssertUnwindSafe(async {
        official_runtime_build(&router).await?;
        live_case_future(case, &router, api_key.as_str(), &model, root.path(), &work_dir,
            &fixture.application, local_proof.as_deref()).await
    }).catch_unwind().await.unwrap_or_else(|_| Err(SmokeFailure::new(
        "live.case", "LIVE_CASE_PANICKED", 500)));
    if let Err(error) = &result {
        eprintln!("NOMIFUN_LIVE_SMOKE_CASE_FAILURE {error}");
    }
    let shutdown_files = if matches!(case, LiveCase::AppWorkspaceUiFixture) {
        Some(snapshot_ui_fixture_shutdown_files(&router, root.path()).await)
    } else { None };
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
            application.close().await.map_err(|error| {
                // A bounded category is enough to identify the shutdown owner;
                // never print arbitrary service/provider error text here.
                let text = error.to_string().to_ascii_lowercase();
                let owner = if text.contains("background task") { "background_tasks" }
                    else if text.contains("session background process") { "session_processes" }
                    else if text.contains("storage") { "storage" }
                    else if text.contains("mcp") { "mcp_auth" }
                    else if text.contains("terminal") { "terminal" }
                    else if text.contains("background") { "background" }
                    else if text.contains("runtime") { "agent_runtime" }
                    else { "unknown" };
                eprintln!("NOMIFUN_LIVE_SMOKE_SHUTDOWN_OWNER owner={owner}");
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
    if let Err(error) = &close_result {
        eprintln!("NOMIFUN_LIVE_SMOKE_CLOSE_FAILURE {error}");
    }
    if let Some(snapshot) = shutdown_files { assert_ui_fixture_shutdown_files(snapshot?)?; }
    if result.is_err() || matches!(case, LiveCase::LongCoding) {
        emit_live_runtime_progress_trace(root.path()).await;
        emit_live_turn_failure_trace(root.path()).await;
    }
    let audit_result = hard_deadline(
        "credential.audit",
        "CREDENTIAL_AUDIT_DEADLINE_EXCEEDED",
        LOCAL_API_DEADLINE,
        async { ensure_secret_not_persisted(root.path(), api_key.as_bytes()).await },
    )
    .await;

    audit_result?;
    if let Some(proof) = local_proof {
        ensure_secret_not_persisted(root.path(), proof.as_bytes()).await?;
    }
    // Retention is diagnostic only, after the real Host has closed and the
    // existing plaintext credential audit has passed. The original failure
    // remains a failure; its canonical facts and encrypted provider storage
    // stay together inside this one disposable fixture.
    if retain_native&&close_result.is_ok() {
        let retained=root.keep();
        eprintln!("NOMIFUN_LIVE_SMOKE_NATIVE_FIXTURE {}",json!({"fixture_root":retained,
            "data_root":retained.join("data"),"work_root":retained.join("work")}));
    }
    result?;
    close_result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --cron-smoke"]
async fn nomi_core_scheduled_tasks_reach_live_stepfun() {
    if let Err(error)=run_live_provider_smoke(LiveCase::ScheduledTasks).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an authorized Step Plan credential on stdin; use the runner --subagent-smoke"]
async fn nomi_core_subagent_integration_reaches_live_stepfun() {
    if let Err(error)=run_live_provider_smoke(LiveCase::SubagentIntegration).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires an authorized Step Plan credential on stdin; use the runner --subagent-eight-smoke"]
async fn nomi_core_eight_subagent_integration_reaches_live_stepfun() {
    if let Err(error)=run_live_provider_smoke(LiveCase::SubagentEightIntegration).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --app-hosting-smoke"]
async fn nomi_core_app_generation_and_hosting_reaches_live_stepfun() {
    if let Err(error) = run_live_provider_smoke(LiveCase::AppHosting).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "isolated production UI Host; requires credential stdin and explicit finish"]
async fn nomi_core_app_workspace_ui_fixture() {
    if let Err(error) = run_live_provider_smoke(LiveCase::AppWorkspaceUiFixture).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --idmm-smoke"]
async fn nomi_core_idmm_reaches_live_stepfun() {
    if let Err(error) = run_live_provider_smoke(LiveCase::Idmm).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {error}");
        panic!("live IDMM demo did not pass");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --reasoning-smoke"]
async fn nomi_core_reasoning_lifecycle_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::ReasoningLifecycle).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --model-smoke"]
async fn nomi_core_selected_model_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::SelectedModel).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --component-smoke"]
async fn nomi_core_managed_component_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::ManagedComponent).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --input-handoff-smoke"]
async fn nomi_core_user_input_handoff_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::UserInputHandoff).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --resource-wait-smoke"]
async fn nomi_core_resource_wait_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::ResourceWait).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --input-queue-smoke"]
async fn nomi_core_input_queue_reaches_live_stepfun() {
    if let Err(failure) = live_input_queue::run().boxed_local().await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --file-smoke"]
async fn nomi_core_workspace_file_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::WorkspaceFile).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --coding-smoke"]
async fn nomi_core_official_coding_agent_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::CodingPreset).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --game-smoke"]
async fn nomi_core_snake_game_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::SnakeGame).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --long-coding-smoke"]
async fn nomi_core_long_coding_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::LongCoding).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --companion-smoke"]
async fn nomi_core_official_companion_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::Companion).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --creative-smoke"]
async fn nomi_core_official_creative_studio_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveCase::CreativeStudio).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[cfg(all(feature = "browser-use", feature = "computer-use"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --general-desktop-smoke"]
async fn nomi_core_general_desktop_reaches_live_stepfun() {
    if let Err(failure) = live_general_desktop::run().await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

    #[test]
    fn retained_subagent_fixture_parent_is_an_exact_isolated_repository_child() {
        let git=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../.git");
        let parent=tempfile::Builder::new().prefix("subagent-retention-self-test-").tempdir_in(&git).unwrap();
        assert_eq!(retained_subagent_parent(parent.path()).unwrap(),nomifun_common::paths::canonicalize_simplified(parent.path()).unwrap());
        assert!(retained_subagent_parent(&git).is_err());
        assert!(retained_subagent_parent(Path::new(".")).is_err());
        let outside=tempfile::tempdir().unwrap();assert!(retained_subagent_parent(outside.path()).is_err());
    }

    #[test]
    fn live_driver_future_frames_are_bounded_without_larger_thread_stacks() {
        let driver = run_live_provider_smoke(LiveCase::CodingPreset);
        let router = Router::new();
        let file_chain = run_live_workspace_file_chain(&router, "", "step-5-preview", Path::new("."), true, false);
        assert!(std::mem::size_of_val(&driver) < 128 * 1024, "scenario driver frame must stay below 128 KiB");
        assert!(std::mem::size_of_val(&file_chain) < 64 * 1024, "file scenario wrapper must stay below 64 KiB");
    }

    #[tokio::test]
    async fn file_scenario_polls_local_http_without_a_credential_or_provider_request() {
        let router = Router::new().fallback(|| async {
            (StatusCode::BAD_REQUEST, axum::Json(json!({"code":"FIXTURE_HTTP_REJECTED"})))
        });
        let failure = run_live_workspace_file_chain(&router, "LOCAL_FIXTURE_ONLY", "step-5-preview", Path::new("."), true, false).await.unwrap_err();
        assert_eq!(failure.phase, "provider.create");
        assert_eq!(failure.code, "FIXTURE_HTTP_REJECTED");
    }

    #[tokio::test]
    async fn timed_out_http_observation_aborts_its_exact_request_task() {
        struct NotifyOnDrop(std::sync::Arc<tokio::sync::Notify>);
        impl Drop for NotifyOnDrop { fn drop(&mut self) { self.0.notify_one(); } }
        let dropped = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = dropped.clone();
        let router = Router::new().fallback(move || {
            let guard = NotifyOnDrop(signal.clone());
            async move { let _guard = guard; std::future::pending::<StatusCode>().await }
        });
        let failure = dispatch_json(&router, "fixture.deadline", Method::GET, "/".into(), None, Duration::from_millis(25)).await.unwrap_err();
        assert_eq!(failure.code, "HTTP_DEADLINE_EXCEEDED");
        tokio::time::timeout(Duration::from_secs(1), dropped.notified()).await.expect("timed out request must not become an orphaned handler");
    }

    #[test]
    fn provider_http_diagnostics_require_typed_public_metadata() {
        for (status, expected) in [
            (400, "CODING_PROVIDER_HTTP_400"), (401, "CODING_PROVIDER_HTTP_401"),
            (402, "CODING_PROVIDER_HTTP_402"), (403, "CODING_PROVIDER_HTTP_403"),
            (404, "CODING_PROVIDER_HTTP_404"), (408, "CODING_PROVIDER_HTTP_408"),
            (409, "CODING_PROVIDER_HTTP_409"), (422, "CODING_PROVIDER_HTTP_422"),
            (429, "CODING_PROVIDER_HTTP_429"), (500, "CODING_PROVIDER_HTTP_500"),
            (502, "CODING_PROVIDER_HTTP_502"), (503, "CODING_PROVIDER_HTTP_503"),
            (504, "CODING_PROVIDER_HTTP_504"),
        ] {
            let error = json!({"projection":{"error":{
                "providerDiagnostic":{"reason":"provider_unavailable","httpStatus":status},
                "detail":"private provider prose NEVER_PRINT_ME",
            }}});
            assert_eq!(admission_conflict_code(&error), Some(expected));
        }
        for error in [
            json!({"detail":"provider_http_status=503; NEVER_PRINT_ME"}),
            json!({"providerDiagnostic":{"httpStatus":"503"}}),
            json!({"providerDiagnostic":{"http_status":503}}),
            json!({"httpStatus":503}),
        ] {
            assert_eq!(admission_conflict_code(&error), None);
        }
    }

    #[test]
    fn marker_tool_policy_requires_the_exact_requested_settled_delegation() {
        let delegation = json!({"presentation_intent":"tool","projection":{"state":"recorded",
            "tool_summary":{"action_id":"agent/delegate","result_state":"recorded","result_digest":"a".repeat(64)}}});
        assert!(session_marker_tools_match(&[],None));
        assert!(!session_marker_tools_match(&[delegation.clone()],None));
        assert!(session_marker_tools_match(&[delegation.clone()],Some("agent/delegate")));
        assert!(!session_marker_tools_match(&[],Some("agent/delegate")));
        assert!(!session_marker_tools_match(&[delegation.clone(),delegation.clone()],Some("agent/delegate")));
        let mut wrong=delegation.clone(); wrong["projection"]["tool_summary"]["action_id"]=json!("agent/fork");
        assert!(!session_marker_tools_match(&[wrong],Some("agent/delegate")));
        let mut unsettled=delegation.clone(); unsettled["projection"]["state"]=json!("started");
        assert!(!session_marker_tools_match(&[unsettled],Some("agent/delegate")));
        let mut failed=delegation; failed["projection"]["tool_summary"]["error"]=json!("failure");
        assert!(!session_marker_tools_match(&[failed],Some("agent/delegate")));
    }

    #[test]
    fn normal_derived_turn_summaries_with_null_error_are_not_failed_tools() {
        for (state,outcome) in [("running",Value::Null),("running",json!({"kind":"waiting","request_id":"authorization"})),
            ("completed",json!({"kind":"input_required","request_id":"next-decision"})),
            ("completed",json!({"kind":"answered","message_ids":["answer"]}))] {
            let summary=json!({"presentation_intent":"turn_summary","projection":{
                "state":state,"error":null,"turn_outcome":outcome}});
            assert_eq!(first_durable_error_code(&[summary]),None);
        }
        let settled=json!({"presentation_intent":"tool","projection":{"state":"recorded",
            "tool_summary":{"name":"start_process","error":null}}});
        assert_eq!(first_durable_error_code(&[settled]),None);
        for projection in [json!({"state":"failed","error":{"code":"TURN_FAILED","message":"real failure"}}),
            json!({"status":"error","error":null}),
            json!({"tool_summary":{"name":"read_file","error":"real tool failure"}})] {
            assert!(first_durable_error_code(&[json!({"projection":projection})]).is_some(),
                "nullable normal metadata must not hide a real error");
        }
    }

    #[test]
    fn admission_diagnostics_emit_only_static_codes_and_preserve_failure() {
        let projected_tool_failure = json!({"projection":{"state":"recorded",
            "tool_summary":{"name":"read_file","error":"RESOURCE_NOT_FOUND: omitted path"}}});
        assert_eq!(first_durable_error_code(&[projected_tool_failure]).as_deref(), Some("CODING_READ_REJECTED"));
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
    fn coding_product_selections_require_all_infrastructure_resources_and_minimal_requires_none() {
        let selections = coding_resource_selections();
        let typed: Vec<nomifun_api_types::AgentResourceSelectionDto> =
            serde_json::from_value(selections.clone()).unwrap();
        assert_eq!(typed.len(), 3);
        assert_eq!(typed[0].resource_kind, "workspace");
        assert_eq!(typed[0].resource_id, "default-workspace");
        assert_eq!(typed[1].resource_kind, "process_session");
        assert_eq!(typed[1].resource_id, "managed-process-session");
        assert_eq!(typed[2].resource_kind, "project_memory");
        assert_eq!(typed[2].resource_id, "default-project-memory");
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
    fn collaboration_diagnostics_expose_only_fixed_shape_categories() {
        let shape = collaboration_argument_shape(&json!({
            "strategy":"SENSITIVE_VALUE", "tasks":"SENSITIVE_VALUE",
            "extra":"SENSITIVE_VALUE"
        }));
        assert_eq!(shape, "other:string");
        assert_eq!(collaboration_argument_shape(&json!({"strategy":"parallel","tasks":[]})), "parallel:array");
    }

    #[test]
    fn selected_workspace_resources_freeze_both_file_and_process_roots() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().unwrap();
        let path = root.path().to_string_lossy().into_owned();
        let id = format!("selected-workspace-{:x}", Sha256::digest(path.as_bytes()));
        let memory_identity = serde_json::to_vec(&("live-smoke-owner", path.as_str())).unwrap();
        let memory_id = format!("project-workspace-memory-{:x}", Sha256::digest(memory_identity));
        let mut binding = json!({"typed_resource_bindings":[
            {"binding_id":"pending","resource_kind":"workspace","resource_id":id,"owner_id":"live-smoke-owner","operations":["read"],"typed_parameters":{"workspace_root":path}},
            {"binding_id":"pending","resource_kind":"process_session","resource_id":"managed-process-session","owner_id":"live-smoke-owner","operations":["execute","observe"],"typed_parameters":{"workspace_root":path}},
            {"binding_id":"pending","resource_kind":"project_memory","resource_id":memory_id,"owner_id":"live-smoke-owner","operations":["read","write"],"typed_parameters":{"workspace_root":path}}
        ]});
        for index in 0..3 {
            let definition = serde_json::from_value::<nomifun_agent_contracts::TypedResourceBinding>(binding["typed_resource_bindings"][index].clone()).unwrap();
            binding["typed_resource_bindings"][index]["binding_id"] = json!(nomifun_agent_contracts::resource_definition_id(&definition).unwrap());
        }
        let selections = coding_resource_selections();
        assert!(verify_resource_selections(&binding, &selections).is_err(), "a selected root is not the managed default identity");
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_ok());
        let canonical_id = binding["typed_resource_bindings"][0]["binding_id"].clone();
        binding["typed_resource_bindings"][0]["binding_id"] = json!(format!("workspace:{id}"));
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err(), "retired resource IDs must not be accepted by the live fixture");
        binding["typed_resource_bindings"][0]["binding_id"] = canonical_id;
        binding["typed_resource_bindings"][1]["typed_parameters"]["workspace_root"] = json!("missing-root");
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err());
    }

    #[test]
    fn selected_process_workspace_adds_only_zero_operation_location_identity() {
        use sha2::{Digest, Sha256};
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = root.path().to_string_lossy().into_owned();
        let id = format!("selected-workspace-{:x}", Sha256::digest(path.as_bytes()));
        let mut binding = json!({"typed_resource_bindings":[
            {"binding_id":"pending","resource_kind":"workspace","resource_id":id,"owner_id":"live-smoke-owner","operations":[],"typed_parameters":{"workspace_root":path}},
            {"binding_id":"pending","resource_kind":"process_session","resource_id":"managed-process-session","owner_id":"live-smoke-owner","operations":["execute"],"typed_parameters":{"workspace_root":path}}
        ]});
        let freeze = |resource: &mut Value| {
            let definition = serde_json::from_value::<nomifun_agent_contracts::TypedResourceBinding>(resource.clone()).unwrap();
            resource["binding_id"] = json!(nomifun_agent_contracts::resource_definition_id(&definition).unwrap());
        };
        for resource in binding["typed_resource_bindings"].as_array_mut().unwrap() { freeze(resource); }
        let selections = json!([{"resource_kind":"process_session","resource_id":"managed-process-session"}]);
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_ok());
        assert!(verify_resource_selections(&binding, &selections).is_err());
        let valid = binding.clone();
        binding["typed_resource_bindings"][0]["operations"] = json!(["read"]);
        freeze(&mut binding["typed_resource_bindings"][0]);
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err(), "an automatic location must grant no file authority");
        binding = valid.clone();
        binding["typed_resource_bindings"][0]["typed_parameters"]["workspace_root"] = json!(other.path());
        freeze(&mut binding["typed_resource_bindings"][0]);
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err());
        binding = valid.clone();
        binding["typed_resource_bindings"][0]["binding_id"] = json!("invalid-digest");
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err());
        binding = valid.clone();
        binding["typed_resource_bindings"][0]["resource_id"] = json!("selected-workspace-wrong-id");
        freeze(&mut binding["typed_resource_bindings"][0]);
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err());
        binding = valid.clone();
        binding["typed_resource_bindings"].as_array_mut().unwrap().push(valid["typed_resource_bindings"][0].clone());
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err(), "exactly one location identity is allowed");
        binding = valid;
        binding["typed_resource_bindings"].as_array_mut().unwrap().push(json!({"resource_kind":"browser","resource_id":"unrequested-browser"}));
        assert!(verify_resource_selections_in_workspace(&binding, &selections, Some(root.path())).is_err(), "unselected resources remain forbidden");
    }

    #[test]
    fn coding_fixture_must_bind_the_project_not_an_empty_managed_session_directory() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("project");
        let managed = root.path().join("managed-session");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(&managed).unwrap();
        let correct = json!({"extra":{"workspace":project,"custom_workspace":true}});
        assert!(verify_fixture_workspace(&correct, &project).is_ok());
        let wrong = json!({"extra":{"workspace":managed,"custom_workspace":true}});
        assert_eq!(verify_fixture_workspace(&wrong, &project).unwrap_err().code, "LIVE_WORKSPACE_BINDING_MISMATCH");
        let wrong_kind = json!({"extra":{"workspace":project,"custom_workspace":false}});
        assert!(verify_fixture_workspace(&wrong_kind, &project).is_err());
        assert!(verify_fixture_workspace(&json!({}), &project).is_err());
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
    async fn collaboration_link_waits_for_admitted_work_and_rejects_a_stale_link() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let router = Router::new().route("/api/agent-sessions/{session_id}/projection",
            axum::routing::get(move || {
                let calls = calls.clone();
                async move {
                    let link = match calls.fetch_add(1, Ordering::SeqCst) {
                        0 => None,
                        1 => Some("previous-execution"),
                        _ => Some("new-execution"),
                    };
                    axum::Json(json!({"success":true,"data":{"linked_execution_id":link}}))
                }
            }));
        let link = wait_for_collaboration_link(&router, "session", Some("previous-execution"), Duration::from_secs(2)).await.unwrap();
        assert_eq!(link, "new-execution");
        assert_eq!(observed.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn missing_collaboration_link_remains_a_failure_at_the_deadline() {
        let router = Router::new().route("/api/agent-sessions/{session_id}/projection",
            axum::routing::get(|| async { axum::Json(json!({"success":true,"data":{"linked_execution_id":null}})) }));
        let failure = wait_for_collaboration_link(&router, "session", None, Duration::from_millis(10)).await.unwrap_err();
        assert_eq!(failure.code, "CLUSTER_LEAD_LINK_DEADLINE_EXCEEDED");
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
