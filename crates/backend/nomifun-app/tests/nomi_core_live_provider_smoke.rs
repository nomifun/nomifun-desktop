//! Ignored product-chain smoke against a real Step Plan provider.
//!
//! Run explicitly with the credential-isolating runner:
//! `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs`
//! Use `--engine-smoke` for bounded official Nomi + Coding engine turns only.
//! `NOMIFUN_LIVE_STEPFUN_MODEL` selects the confirmed model (step-3.7-flash),
//! never an endpoint or fallback. Only the runner may read the credential env.

use std::fmt;
use std::future::Future;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use git2::{Repository, StatusOptions};
use nomifun_app::bootstrap::{NomiCoreApplication, ServerEnvironment};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;
use zeroize::Zeroizing;

const STEPFUN_PLAN_BASE_URL: &str = "https://api.stepfun.com/step_plan/v1";
const STEPFUN_PLAN_MODEL: &str = "step-3.7-flash";
const LIVE_MODEL_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_STEPFUN_MODEL";
const ENGINE_SMOKE_DEADLINE: Duration = Duration::from_secs(15 * 60);
const ENGINE_CAPABILITIES: &[(&str, &[&str], &str)] = &[
    (
        "workspace.files",
        &[
            "workspace.files/read",
            "workspace.files/write",
            "workspace.files/patch",
        ],
        "workspace",
    ),
    (
        "workspace.process",
        &["workspace.process/exec"],
        "process_session",
    ),
];
const LIVE_API_KEY_ENVIRONMENT_NAME: &str = "NOMIFUN_LIVE_STEPFUN_API_KEY";
const STDIN_CREDENTIAL_LIMIT_BYTES: u64 = 16 * 1024;
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const SESSION_MESSAGE_PAGE_LIMIT: u32 = 100;
const SESSION_MESSAGE_MAX_PAGES: usize = 512;
const BOOT_DEADLINE: Duration = Duration::from_secs(90);
const LOCAL_API_DEADLINE: Duration = Duration::from_secs(20);
const TURN_COMMAND_DEADLINE: Duration = Duration::from_secs(180);
const TURN_RESULT_DEADLINE: Duration = Duration::from_secs(120);
const CODING_STAGE_DEADLINE: Duration = Duration::from_secs(180);
const REMOTE_CANCEL_DEADLINE: Duration = Duration::from_secs(70);
const REMOTE_MCP_DEADLINE: Duration = Duration::from_secs(180);
const CRON_RESULT_DEADLINE: Duration = Duration::from_secs(180);
const CRON_REPLAY_SETTLE_DEADLINE: Duration = Duration::from_secs(10);
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const REMOTE_MARKER: &str = "NOMIFUN_REMOTE_LIVE_OK";
const SELECTED_MODEL_MARKER: &str = "NOMIFUN_SELECTED_MODEL_LIVE_OK";
const GUID_INITIAL_MARKER: &str = "NOMIFUN_GUID_INITIAL_LIVE_OK";
const CRON_MARKER: &str = "NOMIFUN_CRON_LIVE_OK";
const CODING_CREATE_MARKER: &str = "NOMIFUN_CODING_CREATE_OK";
const CODING_PATCH_MARKER: &str = "NOMIFUN_CODING_PATCH_OK";
const CODING_EXEC_MARKER: &str = "NOMIFUN_CODING_EXEC_OK";
const CODING_INSPECT_MARKER: &str = "NOMIFUN_CODING_INSPECT_OK";
const CODING_COMMIT_MARKER: &str = "NOMIFUN_CODING_COMMIT_OK";
const CODING_FILE: &str = "live-coding.txt";
const CODING_FILE_CONTENT: &str = "alpha\nbeta\n";
const CODING_COMMIT_MESSAGE: &str = "live coding smoke commit";
const CODING_CAPABILITIES: &[(&str, &[&str], &str)] = &[
    (
        "workspace.files",
        &[
            "workspace.files/read",
            "workspace.files/search",
            "workspace.files/write",
            "workspace.files/patch",
        ],
        "workspace",
    ),
    (
        "workspace.process",
        &["workspace.process/exec"],
        "process_session",
    ),
    (
        "workspace.vcs",
        &[
            "workspace.vcs/status",
            "workspace.vcs/diff",
            "workspace.vcs/stage",
            "workspace.vcs/commit",
        ],
        "workspace",
    ),
];
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

async fn successful_json_with_headers(
    router: &Router,
    phase: &'static str,
    method: Method,
    uri: impl Into<String>,
    body: Option<Value>,
    duration: Duration,
    expected: &[StatusCode],
    extra_headers: &[(&str, &str)],
) -> Result<Value, SmokeFailure> {
    let (status, value) = dispatch_json_with_headers(
        router,
        phase,
        method,
        uri.into(),
        body,
        duration,
        extra_headers,
    )
    .await?;
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
    _engine: Option<&Value>,
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
                    kinds.len() == 1 && kinds[0] == required_resource_kind
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
    expected_engine: Option<&Value>,
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
            "capability_selection": {
                "enabled_skills": [],
                "excluded_auto_skills": [],
                "mcp_server_ids": []
            },
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
    let _ = expected_engine;
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

async fn bind_session_workspace(
    router: &Router,
    session_id: &str,
    workspace: &Path,
) -> Result<(), SmokeFailure> {
    let workspace = std::fs::canonicalize(workspace)
        .map_err(|_| {
            SmokeFailure::new(
                "session.workspace",
                "WORKSPACE_CANONICALIZE_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            )
        })?
        .to_string_lossy()
        .into_owned();
    let response = successful_json(
        router,
        "session.workspace",
        Method::PATCH,
        format!("/api/conversations/{session_id}"),
        Some(json!({
            "extra": {
                "workspace": workspace
            }
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let projection = envelope_data("session.workspace", response)?;
    if !projection
        .pointer("/extra/workspace")
        .and_then(Value::as_str)
        .is_some_and(|actual| canonical_path_eq(Path::new(actual), Path::new(&workspace)))
    {
        return Err(SmokeFailure::new(
            "session.workspace",
            "WORKSPACE_BINDING_NOT_PERSISTED",
            StatusCode::CONFLICT.as_u16(),
        ));
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

async fn start_guid_initial_turn(
    router: &Router,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let idempotency_key = uuid::Uuid::now_v7().to_string();
    let response = successful_json_with_headers(
        router,
        "guid.initial_turn",
        Method::POST,
        format!("/api/conversations/{session_id}/messages"),
        Some(json!({
            "content": format!(
                "Reply with exactly {GUID_INITIAL_MARKER}. Do not call any tool and do not add other text."
            ),
            "files": []
        })),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::ACCEPTED],
        &[
            ("idempotency-key", idempotency_key.as_str()),
            ("x-nomifun-initial-delivery", "1"),
        ],
    )
    .await?;
    let response = envelope_data("guid.initial_turn", response)?;
    if response.get("msg_id").and_then(Value::as_str).is_none() {
        return Err(SmokeFailure::new(
            "guid.initial_turn",
            "GUID_INITIAL_MESSAGE_ID_MISSING",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }
    Ok(())
}

async fn warm_guid_session(router: &Router, session_id: &str) -> Result<(), SmokeFailure> {
    successful_json(
        router,
        "guid.warmup",
        Method::POST,
        format!("/api/conversations/{session_id}/warmup"),
        Some(json!({})),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    Ok(())
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
            && output.contains("Capability Kernel rejected Coding Tool (INVALID_PAYLOAD)")
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
        let diagnostic = if output.contains("Capability Kernel rejected Coding Tool (INVALID_PAYLOAD)") {
            "CODING_KERNEL_INVALID_PAYLOAD"
        } else if output.contains("Capability Kernel rejected Coding Tool (PRESET_RESOURCE_NOT_BOUND)") {
            "CODING_KERNEL_RESOURCE_NOT_BOUND"
        } else if output.contains("Capability Kernel rejected Coding Tool (CAPABILITY_UNAVAILABLE)") {
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
        ("CODING_DRIVER", include_str!("../../nomifun-ai-agent/src/coding_runtime.rs")),
        ("CODING_ERROR", include_str!("../../nomifun-coding-engine/src/error.rs")),
        ("COMPACTION", include_str!("../../nomifun-coding-engine/src/compaction.rs")),
        ("CONTEXT_LIFECYCLE", include_str!("../../nomifun-coding-engine/src/context_lifecycle.rs")),
        ("COMPACTION_SOURCE", include_str!("../../nomifun-coding-engine/src/compaction_source.rs")),
        ("CODING_HOST", include_str!("../src/router/coding_runtime_host.rs")),
        ("CODING_HISTORY", include_str!("../src/router/coding_runtime_history.rs")),
        ("SESSION_HOST", include_str!("../src/router/engine_session_host.rs")),
        ("KERNEL_SESSION", include_str!("../src/router/engine_kernel_session.rs")),
        ("TOOL_HOST", include_str!("../src/router/engine_tool_host.rs")),
        ("PROCESS_HOST", include_str!("../src/router/engine_process_host.rs")),
        ("WAVE2_HOST", include_str!("../src/router/agent_wave2_host.rs")),
        ("CODING_TURN", include_str!("../../nomifun-coding-engine/src/turn.rs")),
        ("CODING_REPLAY", include_str!("../../nomifun-coding-engine/src/history.rs")),
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

// These failures have no external effect: Coding controls execute locally,
// and the exact canonical process schema is checked before owner dispatch.
// Unknown owner failures or failed filesystem effects are never recoverable
// evidence. A corrected proposal still needs a later successful observation.
fn coding_pre_execution_rejection(message: &Value) -> bool {
    let projection = &message["projection"];
    if projection["status"] != "error" { return false; }
    if matches!(projection["name"].as_str(), Some("write_file" | "apply_patch" | "exec_command"))
        && matches!(projection["output"].as_str(),
            Some("The plan needs reconsideration after a failed tool, newly accepted user input, or changed repository instructions. Call update_plan with an explanation of the changed approach before further effects."
                | "Call update_plan with an in_progress step before executing workspace mutations or commands."))
    {
        // Coding's effect_gate returns these before invoking any owner.
        return true;
    }
    match projection["name"].as_str() {
        Some("update_plan" | "report_completion") => true,
        Some("exec_command") => projection["output"].as_str().is_some_and(|output|
            output.contains("Capability Kernel rejected Coding Tool (INVALID_PAYLOAD)"))
            && nomifun_agent_domain_wave2::validate_action_input("process.exec",
                &nomifun_agent_contracts::StrictJsonValue(projection["args"].clone())).is_err(),
        _ => false,
    }
}

fn coding_rejection_corrected(messages: &[Value], index: usize) -> bool {
    coding_pre_execution_rejection(&messages[index]) && messages[index + 1..].iter().any(|later|
        later["projection"]["name"] == messages[index]["projection"]["name"]
            && later["projection"]["status"] == "completed")
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
            ("coding turn exceeded the model-step limit", "CODING_MODEL_STEP_LIMIT"),
            ("Coding context is", "CODING_CONTEXT_TOO_LARGE"),
            ("coding model stream ended without a terminal event", "CODING_MODEL_TERMINAL_MISSING"),
            ("coding turn panicked", "CODING_TURN_PANICKED"),
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
            ("Coding history:", "CODING_HISTORY_REJECTED"),
            ("coding model stream failed (InvalidRequest)", "CODING_MODEL_INVALID_REQUEST"),
            ("coding model stream failed (ProtocolViolation)", "CODING_MODEL_PROTOCOL_VIOLATION"),
            ("coding model stream failed (CausalityRejected)", "CODING_MODEL_CAUSALITY_REJECTED"),
            ("coding model stream failed (PromptTooLong)", "CODING_MODEL_PROMPT_TOO_LONG"),
            ("coding model stream failed (RateLimited)", "CODING_MODEL_RATE_LIMITED"),
            ("coding model stream failed (AuthenticationFailed)", "CODING_MODEL_AUTHENTICATION_FAILED"),
            ("coding model stream failed (DuplicateOperation)", "CODING_MODEL_DUPLICATE_OPERATION"),
            ("coding model stream failed (ShadowNotPrimary)", "CODING_MODEL_SHADOW_NOT_PRIMARY"),
            ("coding model stream failed (SessionTerminal)", "CODING_MODEL_SESSION_TERMINAL"),
            ("coding model stream failed (RouteNotFound)", "CODING_MODEL_ROUTE_NOT_FOUND"),
            ("coding model stream failed (RouteRevisionMismatch)", "CODING_MODEL_ROUTE_REVISION_MISMATCH"),
            ("coding model stream failed (AdapterUnavailable)", "CODING_MODEL_ADAPTER_UNAVAILABLE"),
            ("coding model stream failed (CredentialReferenceMissing)", "CODING_MODEL_CREDENTIAL_REFERENCE_MISSING"),
            ("coding model stream failed (CredentialTargetMismatch)", "CODING_MODEL_CREDENTIAL_TARGET_MISMATCH"),
            ("coding model stream failed (UnsupportedFeature)", "CODING_MODEL_UNSUPPORTED_FEATURE"),
            ("coding model stream failed (ProviderUnavailable)", "CODING_MODEL_PROVIDER_UNAVAILABLE"),
            ("coding model stream failed (StreamInterrupted)", "CODING_MODEL_STREAM_INTERRUPTED"),
            ("coding model stream failed (Cancelled)", "CODING_MODEL_CANCELLED"),
            ("coding model stream failed (Internal)", "CODING_MODEL_INTERNAL"),
            ("coding model stream failed", "CODING_MODEL_FAILED"),
            ("Coding checkpoint is invalid:", "CODING_CHECKPOINT_REJECTED"),
            ("Coding context assembly failed:", "CODING_CONTEXT_REJECTED"),
            ("coding model emitted an invalid event:", "CODING_MODEL_EVENT_INVALID"),
            ("Agent Skills: requested Skill is not in the Agent's immutable selected Skill locks", "CONFLICT_SKILL_LOCK_SELECTION"),
            ("current Kernel compilation differs from the persisted Nomi resolved Snapshot", "CONFLICT_KERNEL_SNAPSHOT_MISMATCH"),
            ("Nomi Plugin Tool Kernel admission failed:", "CONFLICT_KERNEL_ADMISSION"),
            ("Nomi Plugin Tool session materialization failed:", "CONFLICT_TOOL_MATERIALIZATION"),
            ("Engine Session admission:", "CONFLICT_ENGINE_SESSION_ADMISSION"),
            ("Engine turn receipt:", "CONFLICT_ENGINE_TURN_RECEIPT"),
            ("Engine resources:", "CONFLICT_ENGINE_RESOURCES"),
            ("Nomi Wave 2 workspace", "CONFLICT_WORKSPACE_ADMISSION"),
            ("Nomi Wave 2 AgentSession", "CONFLICT_WORKSPACE_ADMISSION"),
            ("Coding requires an owned process execute grant", "CONFLICT_PROCESS_GRANT"),
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

#[derive(Clone, Copy)]
struct CodingToolExpectation {
    name: &'static str,
    validate_args: fn(&Value, &Path) -> bool,
    output_contains: Option<&'static str>,
}

const CODING_CREATE_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "Write",
    validate_args: validate_write_args,
    output_contains: None,
}];
const CODING_PATCH_TOOLS: &[CodingToolExpectation] = &[
    CodingToolExpectation {
        name: "Read",
        validate_args: validate_read_args,
        output_contains: None,
    },
    CodingToolExpectation {
        name: "ApplyPatch",
        validate_args: validate_apply_patch_args,
        output_contains: None,
    },
];
const CODING_EXEC_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "exec_command",
    validate_args: validate_exec_command_args,
    output_contains: Some("git version"),
}];
const CODING_INSPECT_TOOLS: &[CodingToolExpectation] = &[
    CodingToolExpectation {
        name: "Grep",
        validate_args: validate_grep_args,
        output_contains: Some("beta"),
    },
    CodingToolExpectation {
        name: "vcs.status",
        validate_args: validate_vcs_status_args,
        output_contains: None,
    },
    CodingToolExpectation {
        name: "vcs.diff",
        validate_args: validate_vcs_diff_args,
        output_contains: Some("beta"),
    },
];
const CODING_COMMIT_TOOLS: &[CodingToolExpectation] = &[
    CodingToolExpectation {
        name: "vcs.stage",
        validate_args: validate_vcs_stage_args,
        output_contains: None,
    },
    CodingToolExpectation {
        name: "vcs.commit",
        validate_args: validate_vcs_commit_args,
        output_contains: None,
    },
];

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

fn argument_targets_workspace_file(raw: &str, workspace: &Path) -> bool {
    let supplied = Path::new(raw);
    let supplied = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        workspace.join(supplied)
    };
    canonical_path_eq(&supplied, &workspace.join(CODING_FILE))
}

fn argument_targets_workspace(raw: &str, workspace: &Path) -> bool {
    let supplied = Path::new(raw);
    let supplied = if supplied.is_absolute() {
        supplied.to_path_buf()
    } else {
        workspace.join(supplied)
    };
    canonical_path_eq(&supplied, workspace)
}

fn validate_write_args(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["file_path", "content"])
        && args
            .get("file_path")
            .and_then(Value::as_str)
            .is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && args.get("content").and_then(Value::as_str) == Some("alpha\n")
}

fn validate_read_args(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["file_path", "offset", "limit"])
        && args
            .get("file_path")
            .and_then(Value::as_str)
            .is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && args
            .get("offset")
            .is_none_or(|offset| offset.as_u64().is_some())
        && args
            .get("limit")
            .is_none_or(|limit| limit.as_u64().is_some_and(|limit| limit > 0))
}

fn validate_apply_patch_args(args: &Value, workspace: &Path) -> bool {
    if !object_has_only_keys(args, &["files"]) {
        return false;
    }
    let Some(files) = args.get("files").and_then(Value::as_array) else {
        return false;
    };
    let [file] = files.as_slice() else {
        return false;
    };
    if !object_has_only_keys(file, &["file_path", "edits"])
        || !file
            .get("file_path")
            .and_then(Value::as_str)
            .is_some_and(|path| argument_targets_workspace_file(path, workspace))
    {
        return false;
    }
    let Some(edits) = file.get("edits").and_then(Value::as_array) else {
        return false;
    };
    let [edit] = edits.as_slice() else {
        return false;
    };
    if !object_has_only_keys(edit, &["old_string", "new_string", "replace_all"])
        || edit
            .get("replace_all")
            .is_some_and(|replace_all| replace_all.as_bool() != Some(false))
    {
        return false;
    }
    let old = edit.get("old_string").and_then(Value::as_str);
    let new = edit.get("new_string").and_then(Value::as_str);
    let exact_replacement = old == Some("alpha\n") && new == Some(CODING_FILE_CONTENT);
    // A model may preserve the existing terminal newline by replacing only
    // the first line. Require the actual workspace contents to be exact so
    // this remains equivalent to the full replacement rather than accepting
    // an arbitrary partial patch.
    let preserved_terminal_newline =
        old == Some("alpha")
            && new == Some("alpha\nbeta")
            && std::fs::read_to_string(workspace.join(CODING_FILE)).ok().as_deref()
                == Some(CODING_FILE_CONTENT);
    exact_replacement || preserved_terminal_newline
}

fn validate_exec_command_args(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["cmd", "workdir", "tty", "yield_time_ms"])
        && args.get("cmd").and_then(Value::as_str) == Some("git --version")
        && args
            .get("workdir")
            .is_none_or(|workdir| {
                workdir
                    .as_str()
                    .is_some_and(|path| argument_targets_workspace(path, workspace))
            })
        && args
            .get("tty")
            .is_none_or(|tty| tty.as_bool() == Some(false))
        && args.get("yield_time_ms").is_none_or(|yield_time| {
            yield_time
                .as_u64()
                .is_some_and(|yield_time| (250..=30_000).contains(&yield_time))
        })
}

fn validate_grep_args(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(
        args,
        &[
            "pattern",
            "path",
            "glob",
            "context_lines",
            "case_insensitive",
        ],
    ) && args.get("pattern").and_then(Value::as_str) == Some("beta")
        && args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && args
            .get("case_insensitive")
            .is_none_or(|value| value.as_bool() == Some(false))
        && args
            .get("context_lines")
            .is_none_or(|value| value.as_u64().is_some())
        && args
            .get("glob")
            .is_none_or(|value| value.as_str() == Some(CODING_FILE))
}

fn validate_vcs_status_args(args: &Value, _workspace: &Path) -> bool {
    args.as_object().is_some_and(serde_json::Map::is_empty)
}

fn validate_vcs_diff_args(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["path"])
        && args
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|path| argument_targets_workspace_file(path, workspace))
}

fn validate_vcs_stage_args(args: &Value, workspace: &Path) -> bool {
    validate_vcs_diff_args(args, workspace)
}

fn validate_vcs_commit_args(args: &Value, _workspace: &Path) -> bool {
    object_has_only_keys(args, &["message"])
        && args.get("message").and_then(Value::as_str) == Some(CODING_COMMIT_MESSAGE)
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

enum CodingEvidence {
    Complete,
    Incomplete(&'static str),
}

fn inspect_coding_evidence(
    messages: &[Value],
    marker: &str,
    expected_tools: &[CodingToolExpectation],
    workspace: &Path,
) -> CodingEvidence {
    let tool_messages = messages
        .iter()
        .filter(|message| {
            message
                .pointer("/projection/name")
                .and_then(Value::as_str)
                .is_some()
        })
        .collect::<Vec<_>>();
    if tool_messages.len() != expected_tools.len() {
        return CodingEvidence::Incomplete(if tool_messages.len() < expected_tools.len() {
            "CODING_TOOL_EVIDENCE_MISSING"
        } else {
            "CODING_TOOL_SET_MISMATCH"
        });
    }

    let mut turn_id = None;
    for (message, expected) in tool_messages.iter().zip(expected_tools) {
        if message.get("presentation_intent").and_then(Value::as_str) != Some("left") {
            return CodingEvidence::Incomplete("CODING_TOOL_PROJECTION_INVALID");
        }
        let Some(projection) = message.get("projection") else {
            return CodingEvidence::Incomplete("CODING_TOOL_PROJECTION_INVALID");
        };
        if projection.get("name").and_then(Value::as_str) != Some(expected.name) {
            return CodingEvidence::Incomplete("CODING_TOOL_SET_MISMATCH");
        }
        if projection.get("status").and_then(Value::as_str) != Some("completed") {
            return CodingEvidence::Incomplete("CODING_TOOL_NOT_COMPLETED");
        }
        if projection
            .get("call_id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return CodingEvidence::Incomplete("CODING_TOOL_CALL_ID_INVALID");
        }
        let Some(args) = projection.get("args") else {
            return CodingEvidence::Incomplete("CODING_TOOL_ARGUMENTS_INVALID");
        };
        if projection.get("input") != Some(args) {
            return CodingEvidence::Incomplete("CODING_TOOL_INPUT_MISMATCH");
        }
        if !(expected.validate_args)(args, workspace) {
            return CodingEvidence::Incomplete("CODING_TOOL_ARGUMENTS_INVALID");
        }
        if expected.output_contains.is_some_and(|needle| {
            projection
                .get("output")
                .and_then(Value::as_str)
                .is_none_or(|output| !output.contains(needle))
        }) {
            return CodingEvidence::Incomplete("CODING_TOOL_OUTPUT_INVALID");
        }
        let Some(projection_turn_id) = projection
            .get("turn_id")
            .and_then(Value::as_str)
            .filter(|turn_id| !turn_id.is_empty())
        else {
            return CodingEvidence::Incomplete("CODING_TURN_ID_MISSING");
        };
        if turn_id
            .replace(projection_turn_id)
            .is_some_and(|expected_turn_id| expected_turn_id != projection_turn_id)
        {
            return CodingEvidence::Incomplete("CODING_TURN_ID_MISMATCH");
        }
    }

    let assistant_text = messages
        .iter()
        .filter_map(assistant_text_projection)
        .collect::<Vec<_>>();
    let [assistant_text] = assistant_text.as_slice() else {
        return CodingEvidence::Incomplete(if assistant_text.is_empty() {
            "CODING_MARKER_MISSING"
        } else {
            "CODING_ASSISTANT_TEXT_INVALID"
        });
    };
    if assistant_text.get("content").and_then(Value::as_str).map(str::trim) != Some(marker) {
        return CodingEvidence::Incomplete("CODING_ASSISTANT_TEXT_INVALID");
    }
    let Some(text_turn_id) = assistant_text
        .get("turn_id")
        .and_then(Value::as_str)
        .filter(|turn_id| !turn_id.is_empty())
    else {
        return CodingEvidence::Incomplete("CODING_TURN_ID_MISSING");
    };
    if turn_id.is_none_or(|turn_id| turn_id != text_turn_id) {
        return CodingEvidence::Incomplete("CODING_TURN_ID_MISMATCH");
    }
    CodingEvidence::Complete
}

async fn wait_for_coding_stage(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    after_seq: u64,
    marker: &'static str,
    expected_tools: &[CodingToolExpectation],
    workspace: &Path,
    coding_engine: bool,
) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + CODING_STAGE_DEADLINE;
    let mut ready_incomplete_since = None;
    loop {
        // Re-read the complete stage window on every poll. Tool projections are
        // updated in place from running to completed, so advancing the cursor
        // permanently would miss the terminal update even though pagination is
        // otherwise exclusive.
        let (messages, _) = session_messages_after(router, phase, session_id, after_seq).await?;
        let fatal_messages = if coding_engine {
            messages.iter().filter(|message| !coding_pre_execution_rejection(message)).cloned().collect::<Vec<_>>()
        } else { messages.clone() };
        if let Some(code) = first_durable_error_code(&fatal_messages) {
            return Err(SmokeFailure::new(
                phase,
                code,
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        let evidence_messages = if coding_engine {
            coding_engine_evidence_messages(&messages)?
        } else {
            messages
        };
        let latest_evidence =
            inspect_coding_evidence(&evidence_messages, marker, expected_tools, workspace);
        let observation = successful_json(
            router,
            phase,
            Method::GET,
            format!("/api/agent-sessions/{session_id}"),
            None,
            LOCAL_API_DEADLINE,
            &[StatusCode::OK],
        )
        .await?;
        let observation = envelope_data(phase, observation)?;
        let ready = observation.pointer("/head/status").and_then(Value::as_str) == Some("ready");
        if ready && matches!(latest_evidence, CodingEvidence::Complete) {
            if coding_engine {
                let (raw, _) = session_messages_after(router, phase, session_id, after_seq).await?;
                if !raw.iter().any(|message| message["projection"]["name"] == "report_completion"
                    && message["projection"]["status"] == "completed")
                {
                    return Err(SmokeFailure::new(phase, "ENGINE_COMPLETION_EVIDENCE_MISSING", 422));
                }
                let controls = raw.iter().enumerate().filter(|(index, message)|
                    coding_rejection_corrected(&raw, *index) && matches!(message["projection"]["name"].as_str(), Some("update_plan" | "report_completion"))).count();
                let schemas = raw.iter().enumerate().filter(|(index, message)|
                    coding_rejection_corrected(&raw, *index) && !matches!(message["projection"]["name"].as_str(), Some("update_plan" | "report_completion"))).count();
                if controls + schemas > 0 {
                    eprintln!("NOMIFUN_LIVE_SMOKE_RECOVERY phase={phase} controls={controls} pre_execution={schemas}");
                }
            }
            return Ok(());
        }
        // A stable ready head cannot produce more tool results. Allow a short
        // cross-query projection race, then report the actual missing evidence
        // instead of letting the outer stage timeout hide the diagnosis.
        if ready {
            let since = ready_incomplete_since.get_or_insert_with(tokio::time::Instant::now);
            if since.elapsed() >= Duration::from_secs(2) {
                if let CodingEvidence::Incomplete(code) = latest_evidence {
                    return Err(SmokeFailure::new(phase, code, 422));
                }
            }
        } else {
            ready_incomplete_since = None;
        }
        if tokio::time::Instant::now() >= deadline {
            let (code, status) = if ready {
                (
                    match latest_evidence {
                        CodingEvidence::Complete => "CODING_SESSION_NOT_READY",
                        CodingEvidence::Incomplete(code) => code,
                    },
                    StatusCode::UNPROCESSABLE_ENTITY,
                )
            } else {
                (
                    "CODING_STAGE_DEADLINE_EXCEEDED",
                    StatusCode::REQUEST_TIMEOUT,
                )
            };
            return Err(SmokeFailure::new(phase, code, status.as_u16()));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn run_coding_stage(
    router: &Router,
    phase: &'static str,
    session_id: &str,
    idempotency_key: &str,
    prompt: String,
    marker: &'static str,
    expected_tools: &[CodingToolExpectation],
    workspace: &Path,
) -> Result<(), SmokeFailure> {
    let after_seq = session_message_cursor(router, phase, session_id).await?;
    start_session_turn(router, phase, session_id, idempotency_key, prompt).await?;
    wait_for_coding_stage(
        router,
        phase,
        session_id,
        after_seq,
        marker,
        expected_tools,
        workspace,
        false,
    )
    .await
}

fn initialize_git_workspace_blocking(workspace: &Path) -> Result<(), SmokeFailure> {
    std::fs::create_dir_all(workspace).map_err(|_| {
        SmokeFailure::new(
            "coding.workspace",
            "CODING_WORKSPACE_CREATE_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let repository = Repository::init(workspace).map_err(|_| {
        SmokeFailure::new(
            "coding.workspace",
            "GIT_INIT_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let mut config = repository.config().map_err(|_| {
        SmokeFailure::new(
            "coding.workspace",
            "GIT_CONFIG_OPEN_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    config
        .set_str("user.name", "NomiFun Live Smoke")
        .and_then(|_| config.set_str("user.email", "live-smoke@nomifun.invalid"))
        .map_err(|_| {
            SmokeFailure::new(
                "coding.workspace",
                "GIT_IDENTITY_CONFIG_FAILED",
                StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            )
        })
}

async fn initialize_git_workspace(workspace: &Path) -> Result<(), SmokeFailure> {
    let workspace = workspace.to_path_buf();
    hard_deadline(
        "coding.workspace",
        "CODING_WORKSPACE_INIT_DEADLINE_EXCEEDED",
        LOCAL_API_DEADLINE,
        async move {
            tokio::task::spawn_blocking(move || initialize_git_workspace_blocking(&workspace))
                .await
                .map_err(|_| {
                    SmokeFailure::new(
                        "coding.workspace",
                        "CODING_WORKSPACE_INIT_TASK_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })?
        },
    )
    .await
}

async fn run_coding_chain(
    router: &Router,
    session_id: &str,
    workspace: &Path,
) -> Result<(), SmokeFailure> {
    run_coding_stage(
        router,
        "coding.create",
        session_id,
        "live-stepfun-coding-create",
        format!(
            "Use the Write tool, not a shell, to create {CODING_FILE} with exactly `alpha` followed by one newline. After the tool completes, reply with exactly {CODING_CREATE_MARKER}."
        ),
        CODING_CREATE_MARKER,
        CODING_CREATE_TOOLS,
        workspace,
    )
    .await?;
    run_coding_stage(
        router,
        "coding.patch",
        session_id,
        "live-stepfun-coding-patch",
        format!(
            "Use Read on {CODING_FILE}. Then use ApplyPatch, not Write, Edit, Bash, or exec_command, to change its exact content from `alpha\\n` to `alpha\\nbeta\\n`. You may either replace `alpha\\n` with `alpha\\nbeta\\n`, or replace `alpha` with `alpha\\nbeta` while preserving the existing final newline. After the tool completes, reply with exactly {CODING_PATCH_MARKER}."
        ),
        CODING_PATCH_MARKER,
        CODING_PATCH_TOOLS,
        workspace,
    )
    .await?;
    run_coding_stage(
        router,
        "coding.exec",
        session_id,
        "live-stepfun-coding-exec",
        format!(
            "Use exec_command in command mode with `git --version`. Do not use Bash or any other tool. After it succeeds, reply with exactly {CODING_EXEC_MARKER}."
        ),
        CODING_EXEC_MARKER,
        CODING_EXEC_TOOLS,
        workspace,
    )
    .await?;
    run_coding_stage(
        router,
        "coding.inspect",
        session_id,
        "live-stepfun-coding-inspect",
        format!(
            "First use Grep with pattern `beta` and path `{CODING_FILE}`. Then use vcs.status with no arguments and vcs.diff with path `{CODING_FILE}`. Do not use shell tools. After all three complete, reply with exactly {CODING_INSPECT_MARKER}."
        ),
        CODING_INSPECT_MARKER,
        CODING_INSPECT_TOOLS,
        workspace,
    )
    .await?;
    run_coding_stage(
        router,
        "coding.commit",
        session_id,
        "live-stepfun-coding-commit",
        format!(
            "Use vcs.stage with path `{CODING_FILE}`, then use vcs.commit with message `{CODING_COMMIT_MESSAGE}`. Do not use shell tools. After both complete, reply with exactly {CODING_COMMIT_MARKER}."
        ),
        CODING_COMMIT_MARKER,
        CODING_COMMIT_TOOLS,
        workspace,
    )
    .await?;
    let workspace = workspace.to_path_buf();
    hard_deadline(
        "coding.verify",
        "CODING_VERIFY_DEADLINE_EXCEEDED",
        LOCAL_API_DEADLINE,
        async move {
            tokio::task::spawn_blocking(move || verify_coding_workspace(&workspace))
                .await
                .map_err(|_| {
                    SmokeFailure::new(
                        "coding.verify",
                        "CODING_VERIFY_TASK_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })?
        },
    )
    .await
}

fn verify_coding_workspace(workspace: &Path) -> Result<(), SmokeFailure> {
    let content = std::fs::read_to_string(workspace.join(CODING_FILE)).map_err(|_| {
        SmokeFailure::new(
            "coding.verify",
            "CODING_FILE_READ_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    if content != CODING_FILE_CONTENT {
        return Err(SmokeFailure::new(
            "coding.verify",
            "CODING_FILE_CONTENT_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let repository = Repository::open(workspace).map_err(|_| {
        SmokeFailure::new(
            "coding.verify",
            "GIT_REPOSITORY_OPEN_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let commit = repository
        .head()
        .and_then(|head| head.peel_to_commit())
        .map_err(|_| {
            SmokeFailure::new(
                "coding.verify",
                "GIT_COMMIT_MISSING",
                StatusCode::CONFLICT.as_u16(),
            )
        })?;
    if commit.message() != Some(CODING_COMMIT_MESSAGE) || commit.parent_count() != 0 {
        return Err(SmokeFailure::new(
            "coding.verify",
            "GIT_COMMIT_INVALID",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let tree = commit.tree().map_err(|_| {
        SmokeFailure::new(
            "coding.verify",
            "GIT_TREE_READ_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let committed = tree
        .get_path(Path::new(CODING_FILE))
        .and_then(|entry| entry.to_object(&repository))
        .and_then(|object| {
            object.as_blob().map(|blob| blob.content().to_vec()).ok_or_else(|| {
                git2::Error::from_str("committed coding file is not a blob")
            })
        })
        .map_err(|_| {
            SmokeFailure::new(
                "coding.verify",
                "GIT_COMMITTED_FILE_MISSING",
                StatusCode::CONFLICT.as_u16(),
            )
        })?;
    if committed.as_slice() != CODING_FILE_CONTENT.as_bytes() {
        return Err(SmokeFailure::new(
            "coding.verify",
            "GIT_COMMITTED_CONTENT_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let mut options = StatusOptions::new();
    options.include_untracked(true).recurse_untracked_dirs(true);
    let statuses = repository.statuses(Some(&mut options)).map_err(|_| {
        SmokeFailure::new(
            "coding.verify",
            "GIT_STATUS_READ_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    if !statuses.is_empty() {
        return Err(SmokeFailure::new(
            "coding.verify",
            "GIT_WORKSPACE_NOT_CLEAN",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

struct SessionMarkerEvidence {
    tail_seq: u64,
    marker_count: usize,
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
) -> Result<SessionMarkerEvidence, SmokeFailure> {
    let deadline = tokio::time::Instant::now() + duration;
    loop {
        let (messages, tail_seq) =
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
            return Ok(SessionMarkerEvidence {
                tail_seq,
                marker_count: latest_marker_count,
            });
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

async fn create_cron_job(
    router: &Router,
    session_id: &str,
) -> Result<String, SmokeFailure> {
    let response = successful_json(
        router,
        "cron.create",
        Method::POST,
        "/api/cron/jobs",
        Some(json!({
            "name": "Live Step Plan existing-session smoke",
            "schedule": {
                "kind": "every",
                "every_ms": 86_400_000,
                "description": "manual live smoke only"
            },
            "message": format!(
                "Reply with exactly {CRON_MARKER} and no other text. Do not use tools."
            ),
            "conversation_id": session_id,
            "conversation_title": "Live Step Plan session",
            "agent_type": "nomi",
            "created_by": "user",
            "execution_mode": "existing"
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::CREATED],
    )
    .await?;
    let job = envelope_data("cron.create", response)?;
    if job.pointer("/metadata/conversation_id").and_then(Value::as_str) != Some(session_id)
        || job.get("execution_mode").and_then(Value::as_str) != Some("existing")
    {
        return Err(SmokeFailure::new(
            "cron.create",
            "CRON_SESSION_IDENTITY_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    required_string(
        "cron.create",
        &job,
        "/cron_job_id",
        "CRON_JOB_ID_MISSING",
    )
}

async fn run_cron_now(
    router: &Router,
    cron_job_id: &str,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let response = successful_json_with_headers(
        router,
        "cron.run_now",
        Method::POST,
        format!("/api/cron/jobs/{cron_job_id}/run"),
        Some(json!({})),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
        &[("idempotency-key", "live-stepfun-cron-run")],
    )
    .await?;
    let response = envelope_data("cron.run_now", response)?;
    if response.get("conversation_id").and_then(Value::as_str) != Some(session_id) {
        return Err(SmokeFailure::new(
            "cron.run_now",
            "CRON_RUN_SESSION_IDENTITY_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn wait_for_cron_run(
    router: &Router,
    cron_job_id: &str,
) -> Result<String, SmokeFailure> {
    hard_deadline(
        "cron.runs",
        "CRON_RESULT_DEADLINE_EXCEEDED",
        CRON_RESULT_DEADLINE,
        async {
            loop {
                let runs = list_cron_runs(router, "cron.runs", cron_job_id).await?;
                if runs.len() > 1 {
                    return Err(SmokeFailure::new(
                        "cron.runs",
                        "CRON_RUN_COUNT_INVALID",
                        StatusCode::CONFLICT.as_u16(),
                    ));
                }
                if let Some(run) = runs.first() {
                    match run.get("status").and_then(Value::as_str) {
                        Some("ok") => {
                            return required_string(
                                "cron.runs",
                                run,
                                "/cron_job_run_id",
                                "CRON_JOB_RUN_ID_MISSING",
                            );
                        }
                        Some("error" | "missed" | "skipped") => {
                            return Err(SmokeFailure::new(
                                "cron.runs",
                                "CRON_RUN_FAILED",
                                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                            ));
                        }
                        _ => {}
                    }
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        },
    )
    .await
}

async fn list_cron_runs(
    router: &Router,
    phase: &'static str,
    cron_job_id: &str,
) -> Result<Vec<Value>, SmokeFailure> {
    let response = successful_json(
        router,
        phase,
        Method::GET,
        format!("/api/cron/jobs/{cron_job_id}/runs"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    envelope_data(phase, response)?
        .as_array()
        .cloned()
        .ok_or_else(|| {
            SmokeFailure::new(
                phase,
                "CRON_RUNS_INVALID",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })
}

async fn list_cron_session_ids(
    router: &Router,
    phase: &'static str,
    cron_job_id: &str,
) -> Result<Vec<String>, SmokeFailure> {
    let response = successful_json(
        router,
        phase,
        Method::GET,
        format!("/api/cron/jobs/{cron_job_id}/conversations"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let conversations = envelope_data(phase, response)?;
    let conversations = conversations.as_array().ok_or_else(|| {
        SmokeFailure::new(
            phase,
            "CRON_SESSIONS_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        )
    })?;
    conversations
        .iter()
        .map(|conversation| {
            required_string(
                phase,
                conversation,
                "/conversation_id",
                "CRON_SESSION_ID_MISSING",
            )
        })
        .collect()
}

async fn replay_cron_run_and_verify_history(
    router: &Router,
    cron_job_id: &str,
    session_id: &str,
    cron_job_run_id: &str,
    cron_start_cursor: u64,
    baseline_tail_cursor: u64,
    baseline_marker_count: usize,
) -> Result<(), SmokeFailure> {
    run_cron_now(router, cron_job_id, session_id).await?;
    let settle_deadline = tokio::time::Instant::now() + CRON_REPLAY_SETTLE_DEADLINE;
    loop {
        let (messages, tail_cursor) = session_messages_after(
            router,
            "cron.replay_messages",
            session_id,
            cron_start_cursor,
        )
        .await?;
        if tail_cursor != baseline_tail_cursor {
            return Err(SmokeFailure::new(
                "cron.replay_messages",
                "CRON_REPLAY_ADVANCED_SESSION_CURSOR",
                StatusCode::CONFLICT.as_u16(),
            ));
        }
        if exact_assistant_marker_count(&messages, CRON_MARKER) != baseline_marker_count {
            return Err(SmokeFailure::new(
                "cron.replay_messages",
                "CRON_REPLAY_MULTIPLIED_MARKER",
                StatusCode::CONFLICT.as_u16(),
            ));
        }

        let runs = list_cron_runs(router, "cron.runs_replay", cron_job_id).await?;
        if runs.len() != 1
            || runs[0]
                .get("cron_job_run_id")
                .and_then(Value::as_str)
                != Some(cron_job_run_id)
            || runs[0].get("status").and_then(Value::as_str) != Some("ok")
        {
            return Err(SmokeFailure::new(
                "cron.runs_replay",
                "CRON_RUN_REPLAY_NOT_IDEMPOTENT",
                StatusCode::CONFLICT.as_u16(),
            ));
        }

        let session_ids =
            list_cron_session_ids(router, "cron.sessions_replay", cron_job_id).await?;
        if session_ids.as_slice() != [session_id] {
            return Err(SmokeFailure::new(
                "cron.sessions_replay",
                "CRON_REPLAY_MULTIPLIED_SESSION",
                StatusCode::CONFLICT.as_u16(),
            ));
        }
        if tokio::time::Instant::now() >= settle_deadline {
            return Ok(());
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn verify_initial_cron_sessions(
    router: &Router,
    cron_job_id: &str,
    session_id: &str,
) -> Result<(), SmokeFailure> {
    let session_ids = list_cron_session_ids(router, "cron.sessions", cron_job_id).await?;
    if session_ids.as_slice() != [session_id] {
        return Err(SmokeFailure::new(
            "cron.sessions",
            "CRON_SESSION_RELATION_INVALID",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn delete_cron_job(router: &Router, cron_job_id: &str) -> Result<(), SmokeFailure> {
    let response = successful_json(
        router,
        "cron.delete",
        Method::DELETE,
        format!("/api/cron/jobs/{cron_job_id}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(SmokeFailure::new(
            "cron.delete",
            "CRON_DELETE_FAILED",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }
    let (status, body) = dispatch_json(
        router,
        "cron.delete_verify",
        Method::GET,
        format!("/api/cron/jobs/{cron_job_id}"),
        None,
        LOCAL_API_DEADLINE,
    )
    .await?;
    if status != StatusCode::NOT_FOUND {
        return Err(SmokeFailure::http("cron.delete_verify", status, &body));
    }
    Ok(())
}

async fn create_remote_binding(
    router: &Router,
    binding: &Value,
) -> Result<String, SmokeFailure> {
    let response = successful_json(
        router,
        "remote.binding",
        Method::POST,
        "/api/remote-bindings",
        Some(json!({
            "name": "Live Step Plan Remote binding",
            "agent_binding": binding
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let response = envelope_data("remote.binding", response)?;
    required_string(
        "remote.binding",
        &response,
        "/remote_binding_id",
        "REMOTE_BINDING_ID_MISSING",
    )
}

async fn open_remote(router: &Router, binding_id: &str) -> Result<String, SmokeFailure> {
    hard_deadline(
        "remote.open",
        "REMOTE_OPEN_DEADLINE_EXCEEDED",
        TURN_RESULT_DEADLINE,
        async {
            loop {
                let response = successful_json(
                    router,
                    "remote.open",
                    Method::POST,
                    "/api/remote/open",
                    Some(json!({
                        "binding_id": binding_id,
                        "idempotency_key": "live-stepfun-remote-open"
                    })),
                    LOCAL_API_DEADLINE,
                    &[StatusCode::OK],
                )
                .await?;
                let session_id = required_string(
                    "remote.open",
                    &response,
                    "/agent_session_id",
                    "REMOTE_SESSION_ID_MISSING",
                )?;
                match response.pointer("/open_state/state").and_then(Value::as_str) {
                    Some("ready") => return Ok(session_id),
                    Some("opening") => tokio::time::sleep(POLL_INTERVAL).await,
                    Some("failed") => {
                        return Err(SmokeFailure::new(
                            "remote.open",
                            response
                                .pointer("/open_state/code")
                                .and_then(Value::as_str)
                                .unwrap_or("REMOTE_OPEN_FAILED"),
                            StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                        ));
                    }
                    _ => {
                        return Err(SmokeFailure::new(
                            "remote.open",
                            "REMOTE_OPEN_STATE_INVALID",
                            StatusCode::BAD_GATEWAY.as_u16(),
                        ));
                    }
                }
            }
        },
    )
    .await
}

async fn send_remote_turn(router: &Router, session_id: &str) -> Result<(), SmokeFailure> {
    let response = successful_json(
        router,
        "remote.turn",
        Method::POST,
        "/api/remote/turn",
        Some(json!({
            "agent_session_id": session_id,
            "input": {
                "content": format!("Reply with exactly {REMOTE_MARKER} and no other text.")
            },
            "idempotency_key": "live-stepfun-remote-turn"
        })),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    if response.get("agent_session_id").and_then(Value::as_str) != Some(session_id) {
        return Err(SmokeFailure::new(
            "remote.turn",
            "REMOTE_TURN_IDENTITY_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    Ok(())
}

async fn wait_for_remote_turn(router: &Router, session_id: &str) -> Result<(), SmokeFailure> {
    hard_deadline(
        "remote.observe",
        "REMOTE_OBSERVE_DEADLINE_EXCEEDED",
        TURN_RESULT_DEADLINE,
        async {
            let mut after_seq = 0_u64;
            loop {
                let response = successful_json(
                    router,
                    "remote.observe",
                    Method::GET,
                    format!(
                        "/api/remote/observe?agent_session_id={session_id}&after_seq={after_seq}&limit=100"
                    ),
                    None,
                    LOCAL_API_DEADLINE,
                    &[StatusCode::OK],
                )
                .await?;
                let events = response
                    .get("events")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        SmokeFailure::new(
                            "remote.observe",
                            "REMOTE_EVENTS_MISSING",
                            StatusCode::BAD_GATEWAY.as_u16(),
                        )
                    })?;
                for event in events {
                    match event.get("kind").and_then(Value::as_str) {
                        Some("turn/completed") => return Ok(()),
                        Some("turn/failed") => {
                            return Err(SmokeFailure::new(
                                "remote.observe",
                                event
                                    .pointer("/payload/result_error_code")
                                    .and_then(Value::as_str)
                                    .unwrap_or("REMOTE_TURN_FAILED"),
                                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
                            ));
                        }
                        Some("turn/unknown") => {
                            return Err(SmokeFailure::new(
                                "remote.observe",
                                "REMOTE_TURN_UNKNOWN",
                                StatusCode::GATEWAY_TIMEOUT.as_u16(),
                            ));
                        }
                        _ => {}
                    }
                }
                after_seq = response
                    .pointer("/next_cursor/seq")
                    .and_then(Value::as_u64)
                    .ok_or_else(|| {
                        SmokeFailure::new(
                            "remote.observe",
                            "REMOTE_CURSOR_MISSING",
                            StatusCode::BAD_GATEWAY.as_u16(),
                        )
                    })?;
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        },
    )
    .await
}

async fn cancel_remote(router: &Router, session_id: &str) -> Result<(), SmokeFailure> {
    hard_deadline(
        "remote.cancel",
        "REMOTE_CANCEL_DEADLINE_EXCEEDED",
        REMOTE_CANCEL_DEADLINE,
        async {
            loop {
                let (status, response) = dispatch_json(
                    router,
                    "remote.cancel",
                    Method::POST,
                    "/api/remote/cancel".to_owned(),
                    Some(json!({
                        "agent_session_id": session_id,
                        "idempotency_key": "live-stepfun-remote-cancel"
                    })),
                    Duration::from_secs(35),
                )
                .await?;
                if status == StatusCode::OK {
                    return if response.get("session_status").and_then(Value::as_str)
                        == Some("cancelled")
                    {
                        Ok(())
                    } else {
                        Err(SmokeFailure::new(
                            "remote.cancel",
                            "REMOTE_CANCEL_STATE_INVALID",
                            StatusCode::BAD_GATEWAY.as_u16(),
                        ))
                    };
                }
                let failure = SmokeFailure::http("remote.cancel", status, &response);
                if status == StatusCode::GATEWAY_TIMEOUT
                    && failure.code == "NOMI_CORE_REMOTE_CANCEL_UNKNOWN"
                {
                    tokio::time::sleep(POLL_INTERVAL).await;
                    continue;
                }
                return Err(failure);
            }
        },
    )
    .await
}

async fn mint_live_remote_token(router: &Router) -> Result<Zeroizing<String>, SmokeFailure> {
    let response = successful_json(
        router,
        "remote.mcp_token",
        Method::POST,
        "/api/webui/access-token",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    Ok(Zeroizing::new(required_string(
        "remote.mcp_token",
        &response,
        "/data/token",
        "REMOTE_MCP_TOKEN_MISSING",
    )?))
}

async fn dispatch_mcp_request(
    router: &Router,
    phase: &'static str,
    token: &str,
    session_id: Option<&str>,
    request: Value,
    duration: Duration,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), SmokeFailure> {
    hard_deadline(
        phase,
        "REMOTE_MCP_HTTP_DEADLINE_EXCEEDED",
        duration,
        async {
            let mut builder = Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header(header::HOST, "127.0.0.1")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream");
            if let Some(session_id) = session_id {
                builder = builder
                    .header("mcp-session-id", session_id)
                    .header("mcp-protocol-version", "2025-06-18");
            }
            let request = builder
                .body(Body::from(serde_json::to_vec(&request).map_err(|_| {
                    SmokeFailure::new(
                        phase,
                        "REMOTE_MCP_REQUEST_SERIALIZATION_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })?))
                .map_err(|_| {
                    SmokeFailure::new(
                        phase,
                        "REMOTE_MCP_REQUEST_BUILD_FAILED",
                        StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
                    )
                })?;
            let response = router.clone().oneshot(request).await.map_err(|_| {
                SmokeFailure::new(
                    phase,
                    "REMOTE_MCP_ROUTER_DISPATCH_FAILED",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
            let status = response.status();
            let headers = response.headers().clone();
            let body = to_bytes(response.into_body(), BODY_LIMIT)
                .await
                .map_err(|_| {
                    SmokeFailure::new(
                        phase,
                        "REMOTE_MCP_RESPONSE_BODY_FAILED",
                        StatusCode::BAD_GATEWAY.as_u16(),
                    )
                })?;
            Ok((status, headers, body.to_vec()))
        },
    )
    .await
}

fn parse_mcp_json(phase: &'static str, body: &[u8]) -> Result<Value, SmokeFailure> {
    if let Ok(value) = serde_json::from_slice(body) {
        return Ok(value);
    }
    let text = String::from_utf8_lossy(body);
    let data = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .find(|line| !line.is_empty())
        .ok_or_else(|| {
            SmokeFailure::new(
                phase,
                "REMOTE_MCP_RESPONSE_JSON_MISSING",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
    serde_json::from_str(data).map_err(|_| {
        SmokeFailure::new(
            phase,
            "REMOTE_MCP_RESPONSE_JSON_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        )
    })
}

fn mcp_tool_value(
    phase: &'static str,
    response: Value,
) -> Result<Value, SmokeFailure> {
    let text = response
        .pointer("/result/content/0/text")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            SmokeFailure::new(
                phase,
                "REMOTE_MCP_TOOL_RESULT_MISSING",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
    let text = text.strip_prefix("Error: ").unwrap_or(text);
    let payload: Value = serde_json::from_str(text).map_err(|_| {
        SmokeFailure::new(
            phase,
            "REMOTE_MCP_TOOL_RESULT_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        )
    })?;
    if response.pointer("/result/isError") == Some(&Value::Bool(true)) {
        let code = payload
            .pointer("/error/code")
            .or_else(|| payload.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("REMOTE_MCP_TOOL_FAILED");
        return Err(SmokeFailure::new(
            phase,
            code,
            StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
        ));
    }
    Ok(payload)
}

async fn call_live_remote_mcp_tool(
    router: &Router,
    token: &str,
    transport_session_id: &str,
    request_id: u64,
    phase: &'static str,
    name: &str,
    arguments: Value,
) -> Result<Value, SmokeFailure> {
    let (status, _, body) = dispatch_mcp_request(
        router,
        phase,
        token,
        Some(transport_session_id),
        json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }),
        REMOTE_MCP_DEADLINE,
    )
    .await?;
    if status != StatusCode::OK {
        let response = parse_mcp_json(phase, &body)?;
        return Err(SmokeFailure::new(
            phase,
            response
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("REMOTE_MCP_HTTP_FAILURE"),
            status.as_u16(),
        ));
    }
    mcp_tool_value(phase, parse_mcp_json(phase, &body)?)
}

async fn run_remote_mcp_chain(
    router: &Router,
    remote_binding_id: &str,
) -> Result<(), SmokeFailure> {
    let token = mint_live_remote_token(router).await?;
    let (status, headers, body) = dispatch_mcp_request(
        router,
        "remote.mcp.initialize",
        token.as_str(),
        None,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {
                    "name": "nomifun-live-provider-smoke",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        LOCAL_API_DEADLINE,
    )
    .await?;
    if status != StatusCode::OK {
        return Err(SmokeFailure::new(
            "remote.mcp.initialize",
            "REMOTE_MCP_INITIALIZE_FAILED",
            status.as_u16(),
        ));
    }
    let transport_session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SmokeFailure::new(
                "remote.mcp.initialize",
                "REMOTE_MCP_SESSION_ID_MISSING",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?
        .to_owned();
    let initialize = parse_mcp_json("remote.mcp.initialize", &body)?;
    if initialize.pointer("/result/protocolVersion").is_none() {
        return Err(SmokeFailure::new(
            "remote.mcp.initialize",
            "REMOTE_MCP_INITIALIZE_RESULT_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }

    let (status, _, _) = dispatch_mcp_request(
        router,
        "remote.mcp.initialized",
        token.as_str(),
        Some(&transport_session_id),
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
        LOCAL_API_DEADLINE,
    )
    .await?;
    if status != StatusCode::ACCEPTED {
        return Err(SmokeFailure::new(
            "remote.mcp.initialized",
            "REMOTE_MCP_INITIALIZED_STATUS_INVALID",
            status.as_u16(),
        ));
    }

    let (status, _, body) = dispatch_mcp_request(
        router,
        "remote.mcp.tools_list",
        token.as_str(),
        Some(&transport_session_id),
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        LOCAL_API_DEADLINE,
    )
    .await?;
    if status != StatusCode::OK {
        return Err(SmokeFailure::new(
            "remote.mcp.tools_list",
            "REMOTE_MCP_TOOLS_LIST_FAILED",
            status.as_u16(),
        ));
    }
    let tools = parse_mcp_json("remote.mcp.tools_list", &body)?
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            SmokeFailure::new(
                "remote.mcp.tools_list",
                "REMOTE_MCP_TOOLS_MISSING",
                StatusCode::BAD_GATEWAY.as_u16(),
            )
        })?;
    let tool_names = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if tool_names != ["open", "turn", "observe", "cancel"] {
        return Err(SmokeFailure::new(
            "remote.mcp.tools_list",
            "REMOTE_MCP_TOOL_SET_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }

    let opened = call_live_remote_mcp_tool(
        router,
        token.as_str(),
        &transport_session_id,
        3,
        "remote.mcp.open",
        "open",
        json!({
            "binding_id": remote_binding_id,
            "idempotency_key": "live-stepfun-mcp-open"
        }),
    )
    .await?;
    let remote_session_id = required_string(
        "remote.mcp.open",
        &opened,
        "/agent_session_id",
        "REMOTE_MCP_SESSION_ID_MISSING",
    )?;
    if opened.pointer("/open_state/state").and_then(Value::as_str) != Some("ready") {
        return Err(SmokeFailure::new(
            "remote.mcp.open",
            "REMOTE_MCP_OPEN_STATE_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }

    let turned = call_live_remote_mcp_tool(
        router,
        token.as_str(),
        &transport_session_id,
        4,
        "remote.mcp.turn",
        "turn",
        json!({
            "agent_session_id": remote_session_id,
            "input": {
                "content": format!("Reply with exactly {REMOTE_MARKER} and no other text.")
            },
            "idempotency_key": "live-stepfun-mcp-turn"
        }),
    )
    .await?;
    if turned.get("agent_session_id").and_then(Value::as_str) != Some(remote_session_id.as_str()) {
        return Err(SmokeFailure::new(
            "remote.mcp.turn",
            "REMOTE_MCP_TURN_IDENTITY_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }

    let deadline = tokio::time::Instant::now() + TURN_RESULT_DEADLINE;
    let mut after_seq = 0_u64;
    let mut request_id = 5_u64;
    loop {
        let observed = call_live_remote_mcp_tool(
            router,
            token.as_str(),
            &transport_session_id,
            request_id,
            "remote.mcp.observe",
            "observe",
            json!({
                "agent_session_id": remote_session_id,
                "after_cursor": {
                    "agent_session_id": remote_session_id,
                    "seq": after_seq
                },
                "limit": SESSION_MESSAGE_PAGE_LIMIT
            }),
        )
        .await?;
        if value_contains(&observed, REMOTE_MARKER) {
            let completed = observed
                .get("events")
                .and_then(Value::as_array)
                .is_some_and(|events| {
                    events.iter().any(|event| event["kind"] == "turn/completed")
                });
            if completed {
                break;
            }
        }
        if observed
            .get("events")
            .and_then(Value::as_array)
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "turn/failed"))
        {
            return Err(SmokeFailure::new(
                "remote.mcp.observe",
                "REMOTE_MCP_TURN_FAILED",
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        if observed
            .get("events")
            .and_then(Value::as_array)
            .is_some_and(|events| events.iter().any(|event| event["kind"] == "turn/unknown"))
        {
            return Err(SmokeFailure::new(
                "remote.mcp.observe",
                "REMOTE_MCP_TURN_UNKNOWN",
                StatusCode::GATEWAY_TIMEOUT.as_u16(),
            ));
        }
        after_seq = observed
            .pointer("/next_cursor/seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                SmokeFailure::new(
                    "remote.mcp.observe",
                    "REMOTE_MCP_CURSOR_MISSING",
                    StatusCode::BAD_GATEWAY.as_u16(),
                )
            })?;
        if tokio::time::Instant::now() >= deadline {
            return Err(SmokeFailure::new(
                "remote.mcp.observe",
                "REMOTE_MCP_TURN_DEADLINE_EXCEEDED",
                StatusCode::REQUEST_TIMEOUT.as_u16(),
            ));
        }
        request_id += 1;
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    let cancelled = call_live_remote_mcp_tool(
        router,
        token.as_str(),
        &transport_session_id,
        request_id + 1,
        "remote.mcp.cancel",
        "cancel",
        json!({
            "agent_session_id": remote_session_id,
            "idempotency_key": "live-stepfun-mcp-cancel"
        }),
    )
    .await?;
    if cancelled.get("session_status").and_then(Value::as_str) != Some("cancelled") {
        return Err(SmokeFailure::new(
            "remote.mcp.cancel",
            "REMOTE_MCP_CANCEL_STATE_INVALID",
            StatusCode::BAD_GATEWAY.as_u16(),
        ));
    }

    successful_json(
        router,
        "remote.mcp_token_revoke",
        Method::DELETE,
        "/api/webui/access-token",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let (status, _, _) = dispatch_mcp_request(
        router,
        "remote.mcp.revoked_token",
        token.as_str(),
        Some(&transport_session_id),
        json!({
            "jsonrpc": "2.0",
            "id": request_id + 2,
            "method": "tools/list",
            "params": {}
        }),
        LOCAL_API_DEADLINE,
    )
    .await?;
    if status != StatusCode::UNAUTHORIZED {
        return Err(SmokeFailure::new(
            "remote.mcp.revoked_token",
            "REMOTE_MCP_REVOKED_TOKEN_ACCEPTED",
            status.as_u16(),
        ));
    }
    Ok(())
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

async fn run_product_chain(
    router: &Router,
    api_key: &str,
    base_url: &str,
    model: &str,
    workspace: &Path,
) -> Result<(), SmokeFailure> {
    let provider_id = configure_stepfun(router, api_key, base_url, model).await?;
    // Reproduce the real installation: an image model exists, while the
    // unmodified official chat.minimal Agent has an enforced empty tool list.
    successful_json(
        router, "guid.image_catalog", Method::POST, "/api/providers",
        Some(json!({
            "platform": "custom", "name": "Image catalog regression fixture",
            "base_url": "http://127.0.0.1:1/v1", "auth_scheme": "bearer",
            "credentials": {"api_keys": ["nonfunctional-image-fixture"]},
            "enabled": true, "initial_model": {
                "model": "image-fixture", "enabled": true,
                "capabilities": [{"task": "image_generation", "protocol": "openai.images",
                    "connection_role": "default", "provider_params": {}}]
            }, "connections": []
        })), LOCAL_API_DEADLINE, &[StatusCode::CREATED],
    ).await?;
    let official = successful_json(
        router, "guid.official_prepare", Method::POST,
        "/api/agent-presets/from-template/chat.minimal",
        Some(json!({"display_name": "Minimal", "reuse_existing": true,
            "model": {"provider_id": provider_id, "model": model},
            "model_route_refs": {}, "chat_route_records": {}})),
        LOCAL_API_DEADLINE, &[StatusCode::OK],
    ).await?;
    let official = envelope_data("guid.official_prepare", official)?;
    let minimal_id = required_string("guid.official_prepare", &official, "/preset/preset_id", "PRESET_ID_MISSING")?;
    let (minimal_session, _) = create_session(router, &minimal_id, &provider_id, model, None, json!([])).await?;
    warm_guid_session(router, &minimal_session).await?;
    start_guid_initial_turn(router, &minimal_session).await?;
    wait_for_session_marker(router, "guid.minimal_reply", &minimal_session, 0,
        GUID_INITIAL_MARKER, TURN_RESULT_DEADLINE).await?;
    let before_switch = successful_json(router, "session.before_switch", Method::GET,
        format!("/api/conversations/{minimal_session}"), None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let (original_messages, _) = session_messages_after(router, "session.original_history", &minimal_session, 0).await?;
    let next_provider = configure_stepfun(router, api_key, base_url, model).await?;
    successful_json(router, "session.switch_cancel", Method::POST,
        format!("/api/conversations/{minimal_session}/cancel"), Some(json!({})),
        TURN_COMMAND_DEADLINE, &[StatusCode::OK]).await?;
    let switched = successful_json(router, "session.switch_model", Method::PATCH,
        format!("/api/conversations/{minimal_session}"),
        Some(json!({"model": {"provider_id": next_provider, "model": model},
            "execution_model_pool": {"mode": "single", "model": {"provider_id": next_provider, "model": model}},
            "execution_template_id": null})), LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    if switched.pointer("/data/model/provider_id") != Some(&Value::String(next_provider))
        || switched.pointer("/data/agent_snapshot") != before_switch.pointer("/data/agent_snapshot")
    {
        return Err(SmokeFailure::new("session.switch_model", "SESSION_SWITCH_CONTRACT_MISMATCH", 409));
    }
    let switch_cursor = session_message_cursor(router, "session.switch_cursor", &minimal_session).await?;
    start_session_turn(router, "session.switched_turn", &minimal_session,
        &uuid::Uuid::now_v7().to_string(),
        "Reply with exactly NOMIFUN_SWITCHED_MODEL_OK and no other text.".to_owned()).await?;
    wait_for_session_marker(router, "session.switched_reply", &minimal_session, switch_cursor,
        "NOMIFUN_SWITCHED_MODEL_OK", TURN_RESULT_DEADLINE).await?;
    let (messages, _) = session_messages_after(router, "session.preserved_history", &minimal_session, 0).await?;
    if original_messages.iter().any(|original| !messages.contains(original))
        || exact_assistant_marker_count(&messages, GUID_INITIAL_MARKER) != 1
        || exact_assistant_marker_count(&messages, "NOMIFUN_SWITCHED_MODEL_OK") != 1
    {
        return Err(SmokeFailure::new("session.preserved_history", "SESSION_HISTORY_CHANGED", 409));
    }
    let library = successful_json(router, "guid.library", Method::GET,
        "/api/agent-preset-templates", None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    if library.pointer("/data/user_presets").and_then(Value::as_array).is_none_or(|items| items.is_empty()) {
        return Err(SmokeFailure::new("guid.library", "OFFICIAL_LAUNCH_CREATED_PERSONAL_AGENT", 409));
    }
    let (preset_id, _source_binding) =
        create_agent_preset(router, &provider_id, model, None, CODING_CAPABILITIES).await?;

    let before_agent_switch = successful_json(
        router,
        "session.before_agent_switch",
        Method::GET,
        format!("/api/conversations/{minimal_session}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let before_agent_switch = envelope_data("session.before_agent_switch", before_agent_switch)?;
    let history_before_agent_switch = session_messages_after(
        router,
        "session.agent_switch_history_before",
        &minimal_session,
        0,
    )
    .await?
    .0;
    let switched_agent = successful_json(
        router,
        "session.switch_agent",
        Method::PUT,
        format!("/api/agent-sessions/{minimal_session}/preset"),
        Some(json!({"preset_id": preset_id, "resource_selections": coding_resource_selections()})),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let switched_agent = envelope_data("session.switch_agent", switched_agent)?;
    verify_resource_selections(&switched_agent["agent_binding"], &coding_resource_selections())?;
    let after_agent_switch = successful_json(
        router,
        "session.after_agent_switch",
        Method::GET,
        format!("/api/conversations/{minimal_session}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let after_agent_switch = envelope_data("session.after_agent_switch", after_agent_switch)?;
    if after_agent_switch.get("conversation_id") != before_agent_switch.get("conversation_id")
        || after_agent_switch.get("name") != before_agent_switch.get("name")
        || after_agent_switch.get("model") != before_agent_switch.get("model")
        || before_agent_switch.pointer("/extra/agent_name")
            != Some(&Value::String("Minimal".to_owned()))
        || after_agent_switch.pointer("/extra/agent_name")
            != Some(&Value::String("Live Step Plan product smoke".to_owned()))
        || after_agent_switch.pointer("/agent_snapshot/preset_name")
            != Some(&Value::String("Live Step Plan product smoke".to_owned()))
        || after_agent_switch.get("agent_snapshot") == before_agent_switch.get("agent_snapshot")
    {
        return Err(SmokeFailure::new(
            "session.switch_agent",
            "SESSION_AGENT_SWITCH_CONTRACT_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let agent_switch_cursor = session_message_cursor(
        router,
        "session.agent_switch_cursor",
        &minimal_session,
    )
    .await?;
    start_session_turn(
        router,
        "session.agent_switched_turn",
        &minimal_session,
        &uuid::Uuid::now_v7().to_string(),
        format!(
            "The earlier conversation contains {GUID_INITIAL_MARKER}. If you can see that history, reply with exactly NOMIFUN_AGENT_SWITCH_OK and no other text. Do not call tools."
        ),
    )
    .await?;
    wait_for_session_marker(
        router,
        "session.agent_switched_reply",
        &minimal_session,
        agent_switch_cursor,
        "NOMIFUN_AGENT_SWITCH_OK",
        TURN_RESULT_DEADLINE,
    )
    .await?;
    let history_after_agent_switch = session_messages_after(
        router,
        "session.agent_switch_history_after",
        &minimal_session,
        0,
    )
    .await?
    .0;
    if history_before_agent_switch
        .iter()
        .any(|message| !history_after_agent_switch.contains(message))
    {
        return Err(SmokeFailure::new(
            "session.agent_switch_history_after",
            "SESSION_AGENT_SWITCH_LOST_HISTORY",
            StatusCode::CONFLICT.as_u16(),
        ));
    }

    let (session_id, binding) =
        create_session(router, &preset_id, &provider_id, model, None, coding_resource_selections()).await?;
    warm_guid_session(router, &session_id).await?;
    let guid_start_cursor =
        session_message_cursor(router, "guid.cursor_before", &session_id).await?;
    start_guid_initial_turn(router, &session_id).await?;
    wait_for_session_marker(
        router,
        "guid.messages",
        &session_id,
        guid_start_cursor,
        GUID_INITIAL_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    bind_session_workspace(router, &session_id, workspace).await?;
    run_coding_chain(router, &session_id, workspace).await?;

    let cron_job_id = create_cron_job(router, &session_id).await?;
    verify_initial_cron_sessions(router, &cron_job_id, &session_id).await?;
    let cron_start_cursor =
        session_message_cursor(router, "cron.cursor_before", &session_id).await?;
    run_cron_now(router, &cron_job_id, &session_id).await?;
    let cron_job_run_id = wait_for_cron_run(router, &cron_job_id).await?;
    let cron_marker = wait_for_session_marker(
        router,
        "cron.messages",
        &session_id,
        cron_start_cursor,
        CRON_MARKER,
        CRON_RESULT_DEADLINE,
    )
    .await?;
    replay_cron_run_and_verify_history(
        router,
        &cron_job_id,
        &session_id,
        &cron_job_run_id,
        cron_start_cursor,
        cron_marker.tail_seq,
        cron_marker.marker_count,
    )
    .await?;
    delete_cron_job(router, &cron_job_id).await?;

    let remote_binding_id = create_remote_binding(router, &binding).await?;
    let remote_session_id = open_remote(router, &remote_binding_id).await?;
    let remote_start_cursor =
        session_message_cursor(router, "remote.cursor_before", &remote_session_id).await?;
    send_remote_turn(router, &remote_session_id).await?;
    wait_for_remote_turn(router, &remote_session_id).await?;
    wait_for_session_marker(
        router,
        "remote.messages",
        &remote_session_id,
        remote_start_cursor,
        REMOTE_MARKER,
        TURN_RESULT_DEADLINE,
    )
    .await?;
    cancel_remote(router, &remote_session_id).await?;
    run_remote_mcp_chain(router, &remote_binding_id).await
}

async fn run_selected_model_chain(
    router: &Router,
    api_key: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    let provider_id =
        configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let (preset_id, _) =
        create_agent_preset(router, &provider_id, model, None, &[]).await?;
    let (session_id, _) =
        create_session(router, &preset_id, &provider_id, model, None, json!([])).await?;
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

fn engine_selection(catalog: &Value, family: &str, profile: &str) -> Result<Value, SmokeFailure> {
    let matches = catalog.as_array().ok_or_else(|| SmokeFailure::new("engine.catalog", "ENGINE_CATALOG_INVALID", 502))?
        .iter().filter(|item| item["family_id"] == family).collect::<Vec<_>>();
    let [descriptor] = matches.as_slice() else {
        return Err(SmokeFailure::new("engine.catalog", "OFFICIAL_ENGINE_MISSING_OR_AMBIGUOUS", 409));
    };
    let typed: nomifun_api_types::RuntimeEngineDescriptor = serde_json::from_value((**descriptor).clone())
        .map_err(|_| SmokeFailure::new("engine.catalog", "ENGINE_DESCRIPTOR_INVALID", 502))?;
    typed.validate().map_err(|_| SmokeFailure::new("engine.catalog", "ENGINE_DESCRIPTOR_INVALID", 502))?;
    if !typed.supported_profiles.iter().any(|item| item == profile) {
        return Err(SmokeFailure::new("engine.catalog", "ENGINE_PROFILE_MISSING", 409));
    }
    Ok(json!({"selector": {"selection":"exact", "family_id":typed.family_id,
        "build_id":typed.build_id, "build_digest":typed.build_digest}, "profile":profile}))
}

fn verify_engine_binding(binding: &Value, selection: &Value) -> Result<(), SmokeFailure> {
    let typed: nomifun_api_types::RuntimeEngineBinding = serde_json::from_value(binding.clone())
        .map_err(|_| SmokeFailure::new("engine.binding", "ENGINE_BINDING_MISSING_OR_INVALID", 409))?;
    typed.validate().map_err(|_| SmokeFailure::new("engine.binding", "ENGINE_BINDING_INVALID", 409))?;
    if ["family_id", "build_id", "build_digest"].iter().any(|key| binding[*key] != selection["selector"][*key])
        || binding["profile"] != selection["profile"]
    {
        return Err(SmokeFailure::new("engine.binding", "ENGINE_BINDING_MISMATCH", 409));
    }
    Ok(())
}

async fn assert_session_engine(router: &Router, session: &str, selection: &Value, provider: &str, model: &str) -> Result<(), SmokeFailure> {
    let response = successful_json(router, "engine.binding", Method::GET,
        format!("/api/agent-sessions/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let observation = envelope_data("engine.binding", response)?;
    if observation.pointer("/session/agent_session_id").and_then(Value::as_str) != Some(session)
        || observation.pointer("/session/agent_binding").is_none()
        || observation.pointer("/session/runtime_engine_binding").is_some()
    {
        return Err(SmokeFailure::new("engine.binding", "CANONICAL_SESSION_BINDING_INVALID", 409));
    }
    let _ = (selection, provider, model);
    Ok(())
}

// Coding projects canonical args, without Nomi's duplicate input field, and
// performs host-authored instruction reads before model step 1. These reads
// never count as the model's requested filesystem evidence.
fn coding_engine_evidence_messages(messages: &[Value]) -> Result<Vec<Value>, SmokeFailure> {
    let mut result = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if coding_rejection_corrected(messages, index) { continue; }
        let projection = &message["projection"];
        // These are Coding's mandatory planning/completion controls, not
        // platform file/command executions. Only successful controls are
        // excluded; running/error projections remain visible to the checker.
        if matches!(projection["name"].as_str(), Some("update_plan" | "report_completion" | "resume_task"
            | "search_tool_history" | "read_tool_history" | "load_tool_history"))
            && projection["status"] == "completed"
        {
            continue;
        }
        if projection["name"] == "read_file" && projection["args"]["format"] == "instruction_scope"
            && projection["status"] == "completed"
        {
            if !matches!(projection["args"]["path"].as_str(), Some("" | "." | "live-coding.txt")) {
                return Err(SmokeFailure::new("engine.instructions", "ENGINE_INSTRUCTION_SCOPE_INVALID", 422));
            }
            continue;
        }
        if projection["call_id"].as_str().is_some_and(|id| id.starts_with("coding-instructions:")) {
            if projection["name"] != "read_file" {
                return Err(SmokeFailure::new("engine.instructions", "ENGINE_INSTRUCTION_READ_INVALID", 422));
            }
            if projection["status"] != "completed" {
                // A poll can observe a running projection. Keep it so evidence
                // remains incomplete until its terminal update, not an early failure.
                result.push(message.clone());
            }
            continue;
        }
        let mut message = message.clone();
        if let Some(projection) = message.get_mut("projection").and_then(Value::as_object_mut) {
            if projection.contains_key("name") && projection.get("input").is_none_or(Value::is_null) {
                if let Some(args) = projection.get("args").cloned() {
                    projection.insert("input".to_owned(), args);
                }
            }
        }
        result.push(message);
    }
    Ok(result)
}

fn validate_engine_write(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["path", "content"])
        && args["path"].as_str().is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && args["content"] == "alpha\n"
}

fn validate_engine_read(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["path", "format", "offset", "limit"])
        && args["path"].as_str().is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && args.get("format").is_none_or(|format| format == "text")
        && args.get("offset").is_none_or(|offset| offset.as_u64() == Some(0))
        && args.get("limit").is_none_or(|limit| limit.as_u64().is_some_and(|limit| (4..=16384).contains(&limit)))
}

fn engine_patch_hunks() -> Value {
    json!([{"old_start":1,"old_lines":1,"new_start":1,"new_lines":2,
        "lines":[{"kind":"context","text":"alpha"},{"kind":"add","text":"beta"}]}])
}

fn validate_engine_patch(args: &Value, workspace: &Path) -> bool {
    if !object_has_only_keys(args, &["files"]) { return false; }
    let Some(files) = args["files"].as_array() else { return false; };
    let [file] = files.as_slice() else { return false; };
    object_has_only_keys(file, &["path", "hunks", "expected_source"])
        && file["path"].as_str().is_some_and(|path| argument_targets_workspace_file(path, workspace))
        && file["hunks"] == engine_patch_hunks()
        && file.get("expected_source").is_none_or(|source| {
            object_has_only_keys(source, &["kind", "sha256"])
                && source["kind"] == "existing"
                && source["sha256"].as_str().is_some_and(|digest| digest.len() == 64
                    && digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        })
}

fn validate_engine_exec(args: &Value, workspace: &Path) -> bool {
    object_has_only_keys(args, &["operation", "command", "args", "cwd", "timeout_ms"])
        && args.get("operation").is_none_or(|operation| operation == "exec")
        && args["command"] == "git" && args["args"] == json!(["--version"])
        && args.get("cwd").is_none_or(|cwd| cwd.as_str().is_some_and(|path| argument_targets_workspace(path, workspace)))
        && args["timeout_ms"].as_u64().is_some_and(|timeout| (1..=10000).contains(&timeout))
}

const ENGINE_CREATE_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "write_file", validate_args: validate_engine_write, output_contains: None,
}];
const ENGINE_PATCH_TOOLS: &[CodingToolExpectation] = &[
    CodingToolExpectation { name: "read_file", validate_args: validate_engine_read, output_contains: Some("alpha") },
    CodingToolExpectation { name: "apply_patch", validate_args: validate_engine_patch, output_contains: None },
];
const ENGINE_EXEC_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "exec_command", validate_args: validate_engine_exec, output_contains: Some("git version"),
}];
const ENGINE_CONTINUE_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "read_file", validate_args: validate_engine_read, output_contains: Some("beta"),
}];
const NOMI_CONTINUE_TOOLS: &[CodingToolExpectation] = &[CodingToolExpectation {
    name: "Read", validate_args: validate_read_args, output_contains: Some("beta"),
}];
const ENGINE_CONTINUE_MARKER: &str = "NOMIFUN_CODING_CREATE_OK|beta";

fn engine_stage_pass_line(phase: &str) -> Result<&'static str, SmokeFailure> {
    // Entire output lines are static: no model/tool text, paths, IDs or secrets.
    match phase {
        "engine.nomi.create" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.nomi.create status=pass"),
        "engine.nomi.patch" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.nomi.patch status=pass"),
        "engine.nomi.exec" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.nomi.exec status=pass"),
        "engine.nomi.continue" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.nomi.continue status=pass"),
        "engine.coding.create" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.coding.create status=pass"),
        "engine.coding.patch" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.coding.patch status=pass"),
        "engine.coding.exec" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.coding.exec status=pass"),
        "engine.coding.continue" => Ok("NOMIFUN_LIVE_SMOKE_STAGE phase=engine.coding.continue status=pass"),
        _ => Err(SmokeFailure::new("engine.evidence", "ENGINE_STAGE_NOT_ALLOWLISTED", 500)),
    }
}

fn emit_engine_stage_pass(phase: &str) -> Result<(), SmokeFailure> {
    // stderr avoids libtest's partial stdout `test <name> ... ` prefix.
    eprintln!("{}", engine_stage_pass_line(phase)?);
    Ok(())
}

async fn run_engine_chain(router: &Router, api_key: &str, model: &str, root: &Path,
    stages_passed: &mut Vec<&'static str>, failures: &mut Vec<SmokeFailure>) -> Result<(), SmokeFailure> {
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    let catalog = successful_json(router, "engine.catalog", Method::GET, "/api/runtime-engines",
        None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let catalog = envelope_data("engine.catalog", catalog)?;
    let selected_family = std::env::var("NOMIFUN_LIVE_ENGINE_FAMILY").unwrap_or_else(|_| "all".into());
    if !matches!(selected_family.as_str(), "all" | "nomi" | "coding") {
        return Err(SmokeFailure::new("engine.scope", "LIVE_ENGINE_SELECTION_INVALID", 400));
    }
    for (family, profile, coding) in [("nomifun.nomi", "default", false), ("nomifun.coding", "coding", true)] {
        if selected_family != "all" && family != format!("nomifun.{selected_family}") { continue; }
        // Each official engine gets its own Session/workspace even if the other fails.
        let result: Result<(), SmokeFailure> = async {
        let selection = engine_selection(&catalog, family, profile)?;
        let (preset, _) = create_agent_preset(router, &provider, model, Some(&selection), ENGINE_CAPABILITIES).await?;
        let (session, _) = create_session(router, &preset, &provider, model, Some(&selection), coding_resource_selections()).await?;
        let workspace = root.join("work");
        if !workspace.is_dir() {
            return Err(SmokeFailure::new(
                "engine.workspace",
                "WORKSPACE_RESOURCE_MISSING",
                500,
            ));
        }
        assert_session_engine(router, &session, &selection, &provider, model).await?;
        let (write, read, patch) = if coding { ("write_file", "read_file", "apply_patch") } else { ("Write", "Read", "ApplyPatch") };
        let patch_args = if coding {
            json!({"files":[{"path":CODING_FILE,"hunks":engine_patch_hunks()}]})
        } else {
            json!({"files":[{"file_path":CODING_FILE,"edits":[{"old_string":"alpha\n","new_string":CODING_FILE_CONTENT}]}]})
        };
        let exec_args = if coding { json!({"operation":"exec","command":"git","args":["--version"],"timeout_ms":10000}) }
            else { json!({"cmd":"git --version","yield_time_ms":1000}) };
        let stages = [
            ("create", CODING_CREATE_MARKER, format!("Use only {write} to create {CODING_FILE} with exactly alpha followed by one newline. After success reply with exactly {CODING_CREATE_MARKER}; no other text."),
                if coding { ENGINE_CREATE_TOOLS } else { CODING_CREATE_TOOLS }),
            ("patch", CODING_PATCH_MARKER, format!("Use {read} to read {CODING_FILE}, then {patch} with arguments {patch_args}. Do not replace the dedicated tool with a shell or full write. After success reply with exactly {CODING_PATCH_MARKER}; no other text."),
                if coding { ENGINE_PATCH_TOOLS } else { CODING_PATCH_TOOLS }),
            ("exec", CODING_EXEC_MARKER, format!("Use only exec_command with arguments {exec_args}. After success reply with exactly {CODING_EXEC_MARKER}; no other text."),
                if coding { ENGINE_EXEC_TOOLS } else { CODING_EXEC_TOOLS }),
            ("continue", ENGINE_CONTINUE_MARKER, format!("Continue this same conversation. Use only {read} to read {CODING_FILE}. Reply with your exact final response from the FIRST turn in this conversation, then |, then the file's second line, with no other text."),
                if coding { ENGINE_CONTINUE_TOOLS } else { NOMI_CONTINUE_TOOLS }),
        ];
        let mut prior_messages = Vec::new();
        for (stage, marker, prompt, tools) in stages {
            let prompt = if coding {
                format!("Follow the Coding Engine lifecycle. First call update_plan alone: use one step named stage with status in_progress and one requirement id task describing the stage and its constraints, with source.input=0 and a SHORT exact quote from this request (for example live-coding.txt). After the requested external tools succeed, call update_plan alone to set stage completed, omitting unchanged requirements. Then call report_completion alone with one criterion for step stage, requirement_ids=[task], disposition=supported, only CURRENT usable external observation call IDs as evidence_call_ids, and an accurate rationale without claiming unrequested tests. A read before a mutation belongs to an older workspace epoch and must not be cited as current evidence; for a mutation stage cite the successful final mutation result, and for a read-only stage cite its successful read or command. Do not cite planning, instruction-scope or history-control calls as execution evidence. Only then give the final answer. These engine-local controls, bounded tool-history inspection, and read_file(format=instruction_scope) for live-coding.txt or the workspace root are permitted; tool restrictions below apply to task file/command operations. This is a new task in the same conversation, not a request to resume unfinished historical work. {prompt}")
            } else { prompt };
            // Static phases are safe to print even if a provider echoes secrets.
            let phase = match (coding, stage) {
                (false, "create") => "engine.nomi.create", (false, "patch") => "engine.nomi.patch",
                (false, "exec") => "engine.nomi.exec", (false, _) => "engine.nomi.continue",
                (true, "create") => "engine.coding.create", (true, "patch") => "engine.coding.patch",
                (true, "exec") => "engine.coding.exec", (true, _) => "engine.coding.continue",
            };
            hard_deadline(phase, "ENGINE_STAGE_DEADLINE_EXCEEDED", CODING_STAGE_DEADLINE, async {
                let cursor = session_message_cursor(router, phase, &session).await?;
                start_session_turn(router, phase, &session, &uuid::Uuid::now_v7().to_string(), prompt).await?;
                wait_for_coding_stage(router, phase, &session, cursor, marker, tools, &workspace, coding).await
            }).await?;
            let expected = if stage == "create" { "alpha\n" } else { CODING_FILE_CONTENT };
            if std::fs::read_to_string(workspace.join(CODING_FILE)).ok().as_deref() != Some(expected) {
                return Err(SmokeFailure::new(phase, "ENGINE_FILE_CONTENT_MISMATCH", 422));
            }
            let (messages, _) = session_messages_after(router, phase, &session, 0).await?;
            if prior_messages.iter().any(|message| !messages.contains(message)) {
                return Err(SmokeFailure::new(phase, "ENGINE_CONTINUATION_HISTORY_CHANGED", 409));
            }
            prior_messages = messages;
            assert_session_engine(router, &session, &selection, &provider, model).await?;
            stages_passed.push(phase);
        }
        Ok(())
        }.await;
        if let Err(failure) = result { failures.push(failure); }
    }
    failures.first().cloned().map_or(Ok(()), Err)
}

#[derive(Clone, Copy)]
enum LiveSmokeMode { Product, Model, Engines, Compaction, BeforeTool }

async fn run_live_compaction_chain(router: &Router, key: &str, model: &str, root: &Path) -> Result<(), SmokeFailure> {
    use nomifun_db::sqlx::{Connection, sqlite::SqliteConnectOptions, SqliteConnection};
    const PHASE: &str = "engine.compaction";
    const MARKER: &str = "NOMIFUN_LIVE_COMPACTION_OK";
    let provider = configure_stepfun(router, key, STEPFUN_PLAN_BASE_URL, model).await?;
    let catalog = successful_json(router, PHASE, Method::GET, "/api/runtime-engines", None,
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let selection = engine_selection(&envelope_data(PHASE, catalog)?, "nomifun.coding", "coding")?;
    let (preset, _) = create_agent_preset(router, &provider, model, Some(&selection), &[]).await?;
    let (session, _) = create_session(router, &preset, &provider, model, Some(&selection), json!([])).await?;
    let count = nomifun_coding_engine::CodingContextBudget::default().max_history_messages + 32;
    if count > 512 { return Err(SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_TOO_LARGE", 500)); }
    let options = SqliteConnectOptions::new().filename(root.join("data/nomifun-backend.db")).create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options).await
        .map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_OPEN_FAILED", 500))?;
    // Synthetic imported history in this disposable Session only. These rows
    // are test inputs, never presented as model output or execution evidence.
    let mut transaction = connection.begin().await
        .map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_TRANSACTION_FAILED", 500))?;
    for index in 0..count {
        nomifun_db::sqlx::query("INSERT INTO messages (message_id,conversation_id,type,content,position,status,hidden,created_at) VALUES (?,?,'text',?,'right','finish',0,?)")
            .bind(uuid::Uuid::now_v7().to_string()).bind(&session)
            .bind(json!({"content":format!("Synthetic archived input {index}: retain this as test history data."),"fixture":"compaction-history"}).to_string())
            .bind(index as i64).execute(&mut *transaction).await
            .map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_INSERT_FAILED", 500))?;
    }
    transaction.commit().await.map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_COMMIT_FAILED", 500))?;
    let cursor = session_message_cursor(router, PHASE, &session).await?;
    start_session_turn(router, PHASE, &session, &uuid::Uuid::now_v7().to_string(),
        format!("The earlier entries are synthetic history data. This new request requires no tools. Reply with exactly {MARKER}.")).await?;
    wait_for_session_marker(router, PHASE, &session, cursor, MARKER, CODING_STAGE_DEADLINE).await?;
    let (summaries, replacements) = read_compaction_evidence(root).await?;
    if summaries < 1 || replacements < 1 {
        return Err(SmokeFailure::new(PHASE, "REAL_COMPACTION_EVIDENCE_MISSING", 422));
    }
    let retained: i64 = nomifun_db::sqlx::query_scalar("SELECT count(*) FROM messages WHERE conversation_id=? AND json_extract(content,'$.fixture')='compaction-history'")
        .bind(&session).fetch_one(&mut connection).await
        .map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_HISTORY_READ_FAILED", 500))?;
    connection.close().await.map_err(|_| SmokeFailure::new(PHASE, "COMPACTION_FIXTURE_CLOSE_FAILED", 500))?;
    if retained != count as i64 { return Err(SmokeFailure::new(PHASE, "COMPACTION_CHANGED_CANONICAL_HISTORY", 409)); }
    assert_session_engine(router, &session, &selection, &provider, model).await
}

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
    let engine_smoke = matches!(mode, LiveSmokeMode::Engines);
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
    let workspace = root.path().join("coding-workspace");
    if matches!(mode, LiveSmokeMode::Product) {
        initialize_git_workspace(&workspace).await?;
    }
    let fixture = build_fixture(&root).await?;
    let router = fixture.application.router();
    let mut stages_passed = Vec::new();
    let mut engine_failures = Vec::new();

    let result = if matches!(mode, LiveSmokeMode::Model) {
        run_selected_model_chain(&router, api_key.as_str(), &model).await
    } else if engine_smoke {
        hard_deadline(
            "engine.smoke",
            "ENGINE_SMOKE_DEADLINE_EXCEEDED",
            ENGINE_SMOKE_DEADLINE,
            run_engine_chain(&router, api_key.as_str(), &model, root.path(), &mut stages_passed, &mut engine_failures),
        )
        .await
    } else if matches!(mode, LiveSmokeMode::BeforeTool) {
        hard_deadline("before_tool.smoke", "BEFORE_TOOL_SMOKE_DEADLINE_EXCEEDED", ENGINE_SMOKE_DEADLINE,
            before_tool_smoke::run(&router, api_key.as_str(), &model, root.path(), &mut stages_passed)).await
    } else if matches!(mode, LiveSmokeMode::Compaction) {
        hard_deadline("engine.compaction", "LIVE_COMPACTION_DEADLINE_EXCEEDED", ENGINE_SMOKE_DEADLINE,
            run_live_compaction_chain(&router, api_key.as_str(), &model, root.path())).await
    } else {
        run_product_chain(
            &router,
            api_key.as_str(),
            STEPFUN_PLAN_BASE_URL,
            &model,
            &workspace,
        )
        .await
    };
    // Read only aggregate semantic event counts while the database is open.
    // Keep any probe failure until after normal shutdown and credential audit.
    let compaction_evidence = if matches!(mode, LiveSmokeMode::Engines | LiveSmokeMode::Compaction) {
        Some(read_compaction_evidence(root.path()).await)
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
    if let Some(evidence) = compaction_evidence {
        let (summaries, replacements) = evidence?;
        eprintln!("NOMIFUN_LIVE_SMOKE_COMPACTION summaries={summaries} replacements={replacements}");
    }
    for phase in stages_passed {
        if matches!(mode, LiveSmokeMode::BeforeTool) { before_tool_smoke::emit_stage_pass(phase)?; }
        else { emit_engine_stage_pass(phase)?; }
    }
    for failure in engine_failures { eprintln!("NOMIFUN_LIVE_SMOKE_ENGINE_FAILURE {failure}"); }
    // Native UI acceptance is independent from model instruction fidelity.
    // Explicit retention is safe only here, after shutdown and credential audit;
    // a failed smoke keeps its original failure and never becomes a pass.
    if let Some(parent) = retained_parent { retain_native_fixture(root, &parent)?; }
    result
}

async fn read_compaction_evidence(root: &Path) -> Result<(i64, i64), SmokeFailure> {
    use nomifun_db::sqlx::{Connection, sqlite::SqliteConnectOptions, SqliteConnection};
    let failure = |_| SmokeFailure::new("engine.compaction_evidence", "COMPACTION_EVIDENCE_READ_FAILED", 500);
    let options = SqliteConnectOptions::new().filename(root.join("data/nomifun-backend.db"))
        .read_only(true).create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options).await.map_err(failure)?;
    let counts = nomifun_db::sqlx::query_as::<_, (i64, i64)>(
        "SELECT COALESCE(SUM(json_extract(e.event_json,'$.event')='compaction_started'),0), \
         COALESCE(SUM(json_extract(e.event_json,'$.event')='context_compacted'),0) \
         FROM conversation_runtime_events e JOIN conversations c ON c.conversation_id=e.conversation_id \
         WHERE json_extract(c.extra,'$.runtime_engine_binding.family_id')='nomifun.coding'"
    ).fetch_one(&mut connection).await.map_err(failure)?;
    connection.close().await.map_err(failure)?;
    Ok(counts)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; run with the credential-isolating runner"]
async fn nomi_core_product_chain_reaches_live_stepfun_and_remote_binding() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::Product).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --model-smoke"]
async fn nomi_core_selected_model_reaches_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::Model).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --engine-smoke"]
async fn nomi_core_official_engines_reach_live_stepfun() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::Engines).await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; use the runner --compaction-smoke"]
async fn coding_compaction_reaches_live_stepfun_without_discarding_history() {
    if let Err(failure) = run_live_provider_smoke(LiveSmokeMode::Compaction).await {
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

    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn official_engines_cancel_active_native_process_trees() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        let root = tempfile::tempdir().unwrap();
        let fixture = build_fixture(&root).await.unwrap();
        let router = fixture.application.router();
        let catalog = envelope_data("engine.cancel", successful_json(&router, "engine.cancel", Method::GET,
            "/api/runtime-engines", None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap()).unwrap();
        for (family, profile, coding) in [("nomifun.coding", "coding", true), ("nomifun.nomi", "default", false)] {
            let workspace = root.path().join(format!("cancel-{profile}"));
            std::fs::create_dir(&workspace).unwrap();
            let parent_file = workspace.join("parent.pid");
            let child_file = workspace.join("child.pid");
            let script = "echo $$ > \"$1\"; sleep 120 & echo $! > \"$2\"; wait";
            let args = vec!["-c".to_owned(), script.to_owned(), "cancel-fixture".to_owned(),
                parent_file.to_string_lossy().into_owned(), child_file.to_string_lossy().into_owned()];
            let quote = |value: &str| format!("'{}'", value.replace('\'', "'\\''"));
            let command = std::iter::once("/bin/sh").chain(args.iter().map(String::as_str)).map(quote).collect::<Vec<_>>().join(" ");
            let upstream = wiremock::MockServer::start().await;
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(move |_: &wiremock::Request| {
                let step = counter.fetch_add(1, Ordering::SeqCst);
                let (id,name,input) = if coding && step == 0 {
                    ("cancel-plan", "update_plan", json!({"explanation":"Start the cancellable test command.",
                        "plan":[{"step":"wait for cancellation","status":"in_progress"}],
                        "requirements":[{"id":"cancel","description":"Start the long command for host cancellation.","source":{"input":0,"quote":"Start this long command"}}]}))
                } else if step == usize::from(coding) {
                    ("cancel-command", "exec_command", if coding {
                        json!({"operation":"exec","command":"/bin/sh","args":args,"timeout_ms":120000})
                    } else { json!({"cmd":command,"yield_time_ms":1000}) })
                } else {
                    return wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                        .set_body_string(LOCAL_REPLY_SSE).set_delay(Duration::from_secs(60));
                };
                let frame = json!({"id":"cancel","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":input.to_string()}}]},"finish_reason":null}]});
                let done = json!({"id":"cancel","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                    .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
            }).mount(&upstream).await;
            let provider = configure_stepfun(&router, "local-cancel-fixture", &format!("{}/v1",upstream.uri()), STEPFUN_PLAN_MODEL).await.unwrap();
            let selection = engine_selection(&catalog, family, profile).unwrap();
            let (preset,_) = create_agent_preset(&router, &provider, STEPFUN_PLAN_MODEL, Some(&selection), ENGINE_CAPABILITIES).await.unwrap();
            let (session,_) = create_session(&router, &preset, &provider, STEPFUN_PLAN_MODEL, Some(&selection), coding_resource_selections()).await.unwrap();
            bind_session_workspace(&router, &session, &workspace).await.unwrap();
            start_session_turn(&router, "engine.cancel", &session, &uuid::Uuid::now_v7().to_string(),
                "Start this long command in the isolated workspace; the test host will cancel it.".into()).await.unwrap();
            let pids = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if let (Ok(parent),Ok(child)) = (std::fs::read_to_string(&parent_file),std::fs::read_to_string(&child_file)) {
                        if let (Ok(parent),Ok(child)) = (parent.trim().parse::<u32>(),child.trim().parse::<u32>()) { break [parent,child]; }
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await.expect("both native process generations must actually start before cancellation");
            for pid in pids { assert!(nomi_process_runtime::probe_process_identity(pid).unwrap().is_some()); }
            successful_json(&router, "engine.cancel", Method::POST, format!("/api/conversations/{session}/cancel"),
                Some(json!({})), Duration::from_secs(20), &[StatusCode::OK]).await.unwrap();
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let state = successful_json(&router, "engine.cancel", Method::GET, format!("/api/conversations/{session}"),
                        None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                    if state["data"]["status"] == "finished"
                        && pids.iter().all(|pid| nomi_process_runtime::probe_process_identity(*pid).unwrap().is_none()) { break; }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await.expect("terminal publication and complete child-tree reclamation must both finish");
            assert_session_engine(&router, &session, &selection, &provider, STEPFUN_PLAN_MODEL).await.unwrap();
        }
        tokio::time::timeout(SHUTDOWN_DEADLINE, fixture.application.close()).await.unwrap().unwrap();
    }

    const LOCAL_REPLY_SSE: &str = "data: {\"id\":\"mock\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"LOCAL_ADMISSION_OK\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"mock\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn official_engines_cancel_inflight_model_and_continue() {
        let upstream = wiremock::MockServer::start().await;
        let root = tempfile::tempdir().unwrap();
        let fixture = build_fixture(&root).await.unwrap();
        let router = fixture.application.router();
        let provider = configure_stepfun(&router, "local-only-cancel-fixture", &format!("{}/step_plan/v1", upstream.uri()), STEPFUN_PLAN_MODEL).await.unwrap();
        let catalog = successful_json(&router, "engine.catalog", Method::GET, "/api/runtime-engines",
            None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
        let catalog = envelope_data("engine.catalog", catalog).unwrap();
        for (family, profile) in [("nomifun.nomi", "default"), ("nomifun.coding", "coding")] {
            upstream.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(LOCAL_REPLY_SSE)
                    .set_delay(Duration::from_secs(60)))
                .mount(&upstream).await;
            let selection = engine_selection(&catalog, family, profile).unwrap();
            let (preset, _) = create_agent_preset(&router, &provider, STEPFUN_PLAN_MODEL, Some(&selection), ENGINE_CAPABILITIES).await.unwrap();
            let (session, _) = create_session(&router, &preset, &provider, STEPFUN_PLAN_MODEL, Some(&selection), coding_resource_selections()).await.unwrap();
            let workspace = root.path().join(family);
            std::fs::create_dir(&workspace).unwrap();
            bind_session_workspace(&router, &session, &workspace).await.unwrap();
            start_session_turn(&router, "engine.mock.cancel", &session, &uuid::Uuid::now_v7().to_string(), "Wait for the model response".into()).await.unwrap();
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    if !upstream.received_requests().await.unwrap().is_empty() { break; }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await.expect("cancel must target an actual in-flight model request");
            let running = successful_json(&router, "engine.mock.cancel", Method::GET,
                format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
            assert_eq!(running["data"]["status"], "running", "{family}");
            successful_json(&router, "engine.mock.cancel", Method::POST,
                format!("/api/conversations/{session}/cancel"), Some(json!({})), Duration::from_secs(15), &[StatusCode::OK]).await.unwrap();
            let cancelled = successful_json(&router, "engine.mock.cancel", Method::GET,
                format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
            assert_eq!(cancelled["data"]["status"], "finished", "{family}");
            let cursor = session_message_cursor(&router, "engine.mock.cancel", &session).await.unwrap();
            upstream.reset().await;
            wiremock::Mock::given(wiremock::matchers::method("POST"))
                .respond_with(wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream").set_body_string(LOCAL_REPLY_SSE))
                .mount(&upstream).await;
            start_session_turn(&router, "engine.mock.resume", &session, &uuid::Uuid::now_v7().to_string(), "Reply LOCAL_ADMISSION_OK".into()).await.unwrap();
            tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let (messages, _) = session_messages_after(&router, "engine.mock.resume", &session, cursor).await.unwrap();
                    assert!(first_durable_error_code(&messages).is_none(), "{family}: local resume failed: {messages:#?}");
                    if exact_assistant_marker_count(&messages, "LOCAL_ADMISSION_OK") > 0 {
                        let terminal = successful_json(&router, "engine.mock.resume", Method::GET,
                            format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                        if terminal["data"]["status"] == "finished" { break; }
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            }).await.expect("same Session must resume after platform cancellation cleanup");
            assert_eq!(upstream.received_requests().await.unwrap().len(), 1, "{family}: resumed turn reached provider exactly once");
            assert_session_engine(&router, &session, &selection, &provider, STEPFUN_PLAN_MODEL).await.unwrap();
        }
        tokio::time::timeout(SHUTDOWN_DEADLINE, fixture.application.close()).await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn official_engines_admit_live_fixture_before_model_execution() {
        // Same product routes and Agent capabilities as the live chain. The
        // only credential and provider here are test-owned and loopback-only.
        let upstream = wiremock::MockServer::start().await;
        const REQUEST: &str = "Create live-coding.txt with alpha followed by one newline, run git --version, and reply LOCAL_ADMISSION_OK.";
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(|request: &wiremock::Request| {
                let body: Value = serde_json::from_slice(&request.body).unwrap();
                if body["messages"].as_array().unwrap().iter().any(|message|
                    message["role"] == "user" && message["content"].to_string().contains("LOCAL_CONTINUATION_OK"))
                {
                    let coding = body["tools"].as_array().unwrap().iter().any(|tool| tool["function"]["name"] == "write_file");
                    let observed = |id: &str| body["messages"].as_array().unwrap().iter().any(|message| message["role"] == "tool" && message["tool_call_id"] == id);
                    let call = if coding && !observed("next-plan") {
                        Some(("next-plan", "update_plan", json!({"explanation":"Run the requested version command.",
                            "plan":[{"step":"version","status":"in_progress"}],
                            "requirements":[{"id":"version","description":"Run git --version and reply LOCAL_CONTINUATION_OK.","source":{"input":0,"quote":"Run git --version"}}]})))
                    } else if !observed("next-exec") {
                        Some(("next-exec", "exec_command", if coding { json!({"operation":"exec","command":"git","args":["--version"],"timeout_ms":10000}) } else { json!({"cmd":"git --version","yield_time_ms":1000}) }))
                    } else if coding && !observed("next-close") {
                        Some(("next-close", "update_plan", json!({"explanation":"Version command returned.","plan":[{"step":"version","status":"completed"}]})))
                    } else if coding && !observed("next-report") {
                        Some(("next-report", "report_completion", json!({"summary":"Version observed.","criteria":[{"step":"version","requirement_ids":["version"],"disposition":"supported","evidence_call_ids":["next-exec"],"rationale":"The git version command succeeded."}]})))
                    } else { None };
                    let output = if let Some((id,name,args)) = call {
                        let frame = json!({"id":"next","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":null}]});
                        let terminal = json!({"id":"next","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                        format!("data: {frame}\n\ndata: {terminal}\n\ndata: [DONE]\n\n")
                    } else { LOCAL_REPLY_SSE.replace("LOCAL_ADMISSION_OK", "LOCAL_CONTINUATION_OK") };
                    return wiremock::ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(output);
                }
                let observed = |id: &str| body["messages"].as_array().unwrap().iter()
                    .any(|message| message["role"] == "tool" && message["tool_call_id"] == id);
                let coding = body["tools"].as_array().unwrap().iter()
                    .any(|tool| tool["function"]["name"] == "write_file");
                if (coding && observed("local-report")) || (!coding && observed("local-exec")) {
                    return wiremock::ResponseTemplate::new(200)
                        .insert_header("content-type", "text/event-stream")
                        .set_body_string(LOCAL_REPLY_SSE);
                }
                let (id, name, args) = if coding && !observed("local-plan") {
                    ("local-plan", "update_plan", json!({"explanation":"Perform the requested file creation.",
                        "plan":[{"step":"create file","status":"in_progress"}],
                        "requirements":[{"id":"request","description":REQUEST,"source":{"input":0,"quote":REQUEST}}]}))
                } else if observed("local-write") && !observed("local-exec") {
                    ("local-exec", "exec_command", if coding {
                        json!({"operation":"exec","command":"git","args":["--version"],"timeout_ms":10000})
                    } else { json!({"cmd":"git --version","yield_time_ms":1000}) })
                } else if coding && observed("local-exec") && !observed("local-plan-close") {
                    ("local-plan-close", "update_plan", json!({"explanation":"The requested file write returned.",
                        "plan":[{"step":"create file","status":"completed"}]}))
                } else if coding && observed("local-plan-close") {
                    ("local-report", "report_completion", json!({"summary":"The requested write returned successfully.",
                        "criteria":[{"step":"create file","requirement_ids":["request"],"disposition":"supported",
                            "evidence_call_ids":["local-exec"],"rationale":"The write returned and git version command succeeded; no claim of tests."}]}))
                } else if coding {
                    ("local-write", "write_file", json!({"path":CODING_FILE,"content":"alpha\n"}))
                } else {
                    ("local-write", "Write", json!({"file_path":CODING_FILE,"content":"alpha\n"}))
                };
                let frame = json!({"id":"local-write","choices":[{"index":0,"delta":{
                    "tool_calls":[{"index":0,"id":id,"type":"function",
                        "function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":null}]});
                let terminal = json!({"id":"local-write","choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]});
                wiremock::ResponseTemplate::new(200)
                    .insert_header("content-type", "text/event-stream")
                    .set_body_string(format!("data: {frame}\n\ndata: {terminal}\n\ndata: [DONE]\n\n"))
            })
            .mount(&upstream).await;
        let root = tempfile::tempdir().unwrap();
        let fixture = build_fixture(&root).await.unwrap();
        let router = fixture.application.router();
        let provider = configure_stepfun(&router, "local-only-admission-fixture", &format!("{}/step_plan/v1", upstream.uri()), STEPFUN_PLAN_MODEL).await.unwrap();
        let catalog = successful_json(&router, "engine.catalog", Method::GET, "/api/runtime-engines",
            None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
        let catalog = envelope_data("engine.catalog", catalog).unwrap();
        let mut errors = Vec::new();
        for (family, profile) in [("nomifun.coding", "coding"), ("nomifun.nomi", "default")] {
            let selection = engine_selection(&catalog, family, profile).unwrap();
            let (preset, _) = create_agent_preset(&router, &provider, STEPFUN_PLAN_MODEL, Some(&selection), ENGINE_CAPABILITIES).await.unwrap();
            let (session, binding) = create_session(&router, &preset, &provider, STEPFUN_PLAN_MODEL, Some(&selection), coding_resource_selections()).await.unwrap();
            // Creation, Agent switching and explicit capability replacement
            // must all preserve the canonical Agent's empty Skill ceiling.
            for mutation in [None, Some("preset"), Some("capability-selection")] {
                if let Some(endpoint) = mutation {
                    let body = if endpoint == "preset" {
                        json!({"preset_id":preset,"resource_selections":coding_resource_selections()})
                    } else {
                        json!({"capability_selection":{"enabled_skills":[],"excluded_auto_skills":[],"mcp_server_ids":[]}})
                    };
                    successful_json(&router, "engine.mock.skills", Method::PUT,
                        format!("/api/agent-sessions/{session}/{endpoint}"), Some(body), LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                }
                let projected = successful_json(&router, "engine.mock.skills", Method::GET,
                    format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                let projected = envelope_data("engine.mock.skills", projected).unwrap();
                assert_eq!(projected["extra"]["skills"], json!([]), "{family} {mutation:?}: global Skills escaped the immutable selection");
            }
            let workspace = root.path().join(family);
            std::fs::create_dir(&workspace).unwrap();
            bind_session_workspace(&router, &session, &workspace).await.unwrap();
            start_session_turn(&router, "engine.mock", &session, &uuid::Uuid::now_v7().to_string(), REQUEST.into()).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut completed = false;
            loop {
                let (messages, _) = session_messages_after(&router, "engine.mock", &session, 0).await.unwrap();
                if first_durable_error_code(&messages).is_some() {
                    errors.push((family, messages));
                    break;
                }
                if exact_assistant_marker_count(&messages, "LOCAL_ADMISSION_OK") > 0 {
                    let state = successful_json(&router, "engine.mock.terminal", Method::GET,
                        format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                    if state["data"]["status"] == "finished" {
                        completed = true;
                        break;
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    errors.push((family, messages));
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if !completed { continue; }
            assert_eq!(std::fs::read_to_string(workspace.join(CODING_FILE)).unwrap(), "alpha\n");
            let cursor = session_message_cursor(&router, "engine.mock.continuation", &session).await.unwrap();
            start_session_turn(&router, "engine.mock.continuation", &session,
                &uuid::Uuid::now_v7().to_string(), "Run git --version and reply LOCAL_CONTINUATION_OK.".into()).await.unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
            let mut continued = false;
            loop {
                let (messages, _) = session_messages_after(&router, "engine.mock.continuation", &session, cursor).await.unwrap();
                if first_durable_error_code(&messages).is_some() {
                    errors.push((family, messages));
                    break;
                }
                if exact_assistant_marker_count(&messages, "LOCAL_CONTINUATION_OK") > 0 {
                    let state = successful_json(&router, "engine.mock.terminal", Method::GET,
                        format!("/api/conversations/{session}"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
                    if state["data"]["status"] == "finished" { continued = true; break; }
                }
                if tokio::time::Instant::now() >= deadline { errors.push((family, messages)); break; }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            if !continued { continue; }
            let (actual_messages, _) = session_messages_after(&router, "engine.mock.exec", &session, 0).await.unwrap();
            let commands = actual_messages.iter().filter(|message| message["projection"]["name"] == "exec_command").collect::<Vec<_>>();
            assert_eq!(commands.len(), 2, "{family}: both turns must execute their command");
            assert!(commands.iter().all(|message| message["projection"]["status"] == "completed"
                && message["projection"]["output"].as_str().is_some_and(|output| output.contains("git version"))),
                "{family}: command success must come from real owner output: {commands:?}");
            let (_, through_seq) = session_messages_after(&router, "engine.mock.fork", &session, 0).await.unwrap();
            let child = successful_json(&router, "engine.mock.fork", Method::POST,
                format!("/api/agent-sessions/{session}/forks"),
                Some(json!({"target_agent_binding":binding,"parent_through_seq":through_seq,"title":"Official Engine inherited binding"})),
                LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
            let child = envelope_data("engine.mock.fork", child).unwrap();
            let child_id = child["child_agent_session_id"].as_str().unwrap();
            assert_session_engine(&router, child_id, &selection, &provider, STEPFUN_PLAN_MODEL).await.unwrap();

            let (next_family, next_profile) = if family == "nomifun.nomi" { ("nomifun.coding", "coding") } else { ("nomifun.nomi", "default") };
            let next_selection = engine_selection(&catalog, next_family, next_profile).unwrap();
            let editor = successful_json(&router, "engine.mock.edit", Method::GET,
                format!("/api/agent-presets/{preset}/editor"), None, LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
            let editor = envelope_data("engine.mock.edit", editor).unwrap();
            let mut draft = editor["draft"].clone();
            draft["document"]["runtime_engine"] = next_selection.clone();
            successful_json(&router, "engine.mock.edit", Method::POST,
                format!("/api/agent-presets/{preset}/revisions"),
                Some(json!({"expected_current_revision":editor["revision"]["reference"],"draft":draft})),
                LOCAL_API_DEADLINE, &[StatusCode::OK]).await.unwrap();
            create_session(&router, &preset, &provider, STEPFUN_PLAN_MODEL, Some(&next_selection), coding_resource_selections()).await.unwrap();
            assert_session_engine(&router, &session, &selection, &provider, STEPFUN_PLAN_MODEL).await.unwrap();
            assert_session_engine(&router, child_id, &selection, &provider, STEPFUN_PLAN_MODEL).await.unwrap();
        }
        fixture.application.close().await.unwrap();
        assert!(errors.is_empty(), "local-only admission failures: {errors:#?}");
        assert_eq!(upstream.received_requests().await.unwrap().len(), 16);
    }

    #[test]
    fn admission_diagnostics_emit_only_static_codes_and_preserve_failure() {
        let location = trusted_error_source_location(&json!({"detail":"process controls cannot change launch parameters; NEVER_PRINT_ME"})).unwrap();
        assert!(location.starts_with("DIAG_PROCESS_HOST_L"));
        assert!(!location.contains("NEVER_PRINT_ME"));
        assert!(trusted_error_source_location(&json!("NEVER_PRINT_ME")).is_none());
        let unknown_upstream = json!({"projection":{"type":"error","code":"UNKNOWN_UPSTREAM_ERROR",
            "error":{"detail":"Coding history: Coding checkpoint is invalid: model reused a prior tool-call identity in history; NEVER_PRINT_ME"}}});
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
    fn coding_controls_are_not_external_effects_and_errors_remain_failures() {
        for name in ["update_plan", "report_completion", "resume_task", "search_tool_history", "read_tool_history", "load_tool_history"] {
            let completed = json!({"projection":{"name":name,"status":"completed"}});
            assert!(coding_engine_evidence_messages(&[completed]).unwrap().is_empty());
            for status in ["running", "error"] {
                let message = json!({"projection":{"name":name,"status":status}});
                assert_eq!(coding_engine_evidence_messages(&[message.clone()]).unwrap().len(), 1);
                assert_eq!(first_durable_error_code(&[message]).is_some(), status == "error");
            }
        }
        let unknown = json!({"projection":{"name":"unexpected_tool","status":"completed"}});
        assert_eq!(coding_engine_evidence_messages(&[unknown]).unwrap().len(), 1);
        let scope = json!({"projection":{"name":"read_file","status":"completed",
            "args":{"format":"instruction_scope","path":CODING_FILE}}});
        assert!(coding_engine_evidence_messages(&[scope.clone()]).unwrap().is_empty());
        let mut outside = scope;
        outside["projection"]["args"]["path"] = json!("../outside");
        assert!(coding_engine_evidence_messages(&[outside]).is_err());
        let ordinary_read = json!({"projection":{"name":"read_file","status":"completed","args":{"path":CODING_FILE}}});
        assert_eq!(coding_engine_evidence_messages(&[ordinary_read]).unwrap().len(), 1);
    }

    #[test]
    fn coding_recovery_requires_proven_pre_execution_rejection_and_later_success() {
        let failed_plan = json!({"projection":{"name":"update_plan","status":"error"}});
        let successful_plan = json!({"projection":{"name":"update_plan","status":"completed"}});
        assert!(coding_rejection_corrected(&[failed_plan.clone(), successful_plan.clone()], 0));
        assert!(!coding_rejection_corrected(&[successful_plan, failed_plan.clone()], 1));
        assert!(!coding_engine_evidence_messages(&[failed_plan]).unwrap().is_empty());
        let invalid_process = json!({"projection":{"name":"exec_command","status":"error",
            "args":{"command":"git","args":["--version"],"env":null},
            "output":"Capability Kernel rejected Coding Tool (INVALID_PAYLOAD): hidden owner detail"}});
        assert!(coding_pre_execution_rejection(&invalid_process));
        let mut owner_failure = invalid_process.clone();
        owner_failure["projection"]["args"].as_object_mut().unwrap().remove("env");
        assert!(!coding_pre_execution_rejection(&owner_failure), "schema-valid owner failures remain fatal");
        owner_failure["projection"]["name"] = json!("write_file");
        assert!(!coding_pre_execution_rejection(&owner_failure), "filesystem failures never become successful evidence");
        owner_failure["projection"]["output"] = json!("Call update_plan with an in_progress step before executing workspace mutations or commands.");
        assert!(coding_pre_execution_rejection(&owner_failure), "the engine plan gate has not invoked the filesystem owner");
        let successful_process = json!({"projection":{"name":"exec_command","status":"completed",
            "args":{"command":"git","args":["--version"]}}});
        assert_eq!(coding_engine_evidence_messages(&[invalid_process, successful_process]).unwrap().len(), 1);
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
    fn stage_success_lines_are_exact_static_and_unique() {
        let phases = [
            "engine.nomi.create", "engine.nomi.patch", "engine.nomi.exec", "engine.nomi.continue",
            "engine.coding.create", "engine.coding.patch", "engine.coding.exec", "engine.coding.continue",
        ];
        let mut lines = std::collections::BTreeSet::new();
        for phase in phases {
            let line = engine_stage_pass_line(phase).unwrap();
            assert_eq!(line, format!("NOMIFUN_LIVE_SMOKE_STAGE phase={phase} status=pass"));
            assert!(lines.insert(line));
        }
        assert_eq!(lines.len(), 8);
        for invalid in ["", "engine.nomi.create\n", "engine.nomi.create status=pass", "engine.other.create"] {
            assert!(engine_stage_pass_line(invalid).is_err());
        }
    }

    #[test]
    fn official_engine_binding_cannot_substitute_nomi_for_coding() {
        let catalog = json!([
            {"family_id":"nomifun.nomi","build_id":"nomi-build","build_digest":"a".repeat(64),
             "display_name":"Nomi","host_contract_version":1,"supported_profiles":["default"]},
            {"family_id":"nomifun.coding","build_id":"coding-build","build_digest":"b".repeat(64),
             "display_name":"Coding","host_contract_version":1,"supported_profiles":["coding"]}
        ]);
        let selection = engine_selection(&catalog, "nomifun.coding", "coding").unwrap();
        let mut binding = json!({"family_id":"nomifun.coding","build_id":"coding-build",
            "build_digest":"b".repeat(64),"host_contract_version":1,"profile":"coding"});
        assert!(verify_engine_binding(&binding, &selection).is_ok());
        binding["family_id"] = json!("nomifun.nomi");
        assert!(verify_engine_binding(&binding, &selection).is_err());
        binding["family_id"] = json!("nomifun.coding");
        binding["build_digest"] = json!("c".repeat(64));
        assert!(verify_engine_binding(&binding, &selection).is_err());
        assert!(engine_selection(&json!([]), "nomifun.coding", "coding").is_err());
        assert!(engine_selection(&catalog, "nomifun.coding", "default").is_err());
        assert!(verify_engine_binding(&Value::Null, &selection).is_err());
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
    fn canonical_coding_arguments_and_host_reads_are_separate_evidence() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(CODING_FILE), CODING_FILE_CONTENT).unwrap();
        assert!(validate_engine_write(&json!({"path":CODING_FILE,"content":"alpha\n"}), root.path()));
        assert!(validate_engine_read(&json!({"path":CODING_FILE}), root.path()));
        assert!(validate_engine_patch(&json!({"files":[{"path":CODING_FILE,"hunks":engine_patch_hunks()}]}), root.path()));
        assert!(!validate_engine_patch(&json!({"files":[{"path":CODING_FILE,"hunks":[]}]}), root.path()));
        assert!(validate_engine_exec(&json!({"operation":"exec","command":"git","args":["--version"],"timeout_ms":10000}), root.path()));
        assert!(!validate_engine_exec(&json!({"operation":"exec","command":"git","args":["commit"],"timeout_ms":10000}), root.path()));
        assert!(!validate_engine_exec(&json!({"cmd":"git --version"}), root.path()));
        assert!(!validate_engine_write(&json!({"path":"../escape.txt","content":"alpha\n"}), root.path()));
        let host_read = message(json!({"call_id":"coding-instructions:0","name":"read_file","status":"completed"}));
        let read = message(json!({"call_id":"model-read","name":"read_file","status":"completed",
            "args":{"path":CODING_FILE},"turn_id":"turn","output":"beta"}));
        let text = message(json!({"content":ENGINE_CONTINUE_MARKER,"turn_id":"turn"}));
        let projected = coding_engine_evidence_messages(&[host_read.clone(), read, text.clone()]).unwrap();
        assert_eq!(projected.len(), 2);
        assert!(matches!(inspect_coding_evidence(&projected, ENGINE_CONTINUE_MARKER, ENGINE_CONTINUE_TOOLS, root.path()), CodingEvidence::Complete));
        let host_only = coding_engine_evidence_messages(&[host_read, text]).unwrap();
        assert!(matches!(inspect_coding_evidence(&host_only, ENGINE_CONTINUE_MARKER, ENGINE_CONTINUE_TOOLS, root.path()), CodingEvidence::Incomplete("CODING_TOOL_EVIDENCE_MISSING")));
    }

    fn message(projection: Value) -> Value {
        json!({
            "session_id": "session",
            "projection_id": "projection",
            "first_seq": 1,
            "last_seq": 1,
            "presentation_intent": "left",
            "projection": projection,
            "semantic_digest": "digest"
        })
    }

    #[test]
    fn strict_coding_evidence_rejects_shell_substitution_and_marker_suffixes() {
        let root = tempfile::tempdir().expect("temporary evidence workspace");
        std::fs::write(root.path().join(CODING_FILE), "alpha\n")
            .expect("seed coding evidence file");
        let args = json!({
            "file_path": CODING_FILE,
            "content": "alpha\n"
        });
        let native = message(json!({
            "call_id": "call-write",
            "name": "Write",
            "args": args,
            "input": args,
            "status": "completed",
            "turn_id": "turn"
        }));
        let exact_text = message(json!({
            "content": CODING_CREATE_MARKER,
            "turn_id": "turn"
        }));
        assert!(matches!(
            inspect_coding_evidence(
                &[native.clone(), exact_text],
                CODING_CREATE_MARKER,
                CODING_CREATE_TOOLS,
                root.path(),
            ),
            CodingEvidence::Complete
        ));

        let shell = message(json!({
            "call_id": "call-shell",
            "name": "Bash",
            "args": {"command": "echo alpha"},
            "input": {"command": "echo alpha"},
            "status": "completed",
            "turn_id": "turn"
        }));
        assert!(matches!(
            inspect_coding_evidence(
                &[shell, message(json!({
                    "content": CODING_CREATE_MARKER,
                    "turn_id": "turn"
                }))],
                CODING_CREATE_MARKER,
                CODING_CREATE_TOOLS,
                root.path(),
            ),
            CodingEvidence::Incomplete("CODING_TOOL_SET_MISMATCH")
        ));

        assert!(matches!(
            inspect_coding_evidence(
                &[
                    native,
                    message(json!({
                        "content": format!("{CODING_CREATE_MARKER} extra"),
                        "turn_id": "turn"
                    }))
                ],
                CODING_CREATE_MARKER,
                CODING_CREATE_TOOLS,
                root.path(),
            ),
            CodingEvidence::Incomplete("CODING_ASSISTANT_TEXT_INVALID")
        ));
    }

    #[test]
    fn coding_argument_contracts_pin_native_paths_and_payloads() {
        let root = tempfile::tempdir().expect("temporary argument workspace");
        std::fs::write(root.path().join(CODING_FILE), CODING_FILE_CONTENT)
            .expect("seed coding argument file");

        assert!(validate_write_args(
            &json!({"file_path": CODING_FILE, "content": "alpha\n"}),
            root.path(),
        ));
        assert!(validate_read_args(
            &json!({"file_path": CODING_FILE}),
            root.path(),
        ));
        assert!(validate_apply_patch_args(
            &json!({
                "files": [{
                    "file_path": CODING_FILE,
                    "edits": [{
                        "old_string": "alpha\n",
                        "new_string": CODING_FILE_CONTENT
                    }]
                }]
            }),
            root.path(),
        ));
        let preserved_newline_root = tempfile::tempdir().expect("preserved newline workspace");
        std::fs::write(
            preserved_newline_root.path().join(CODING_FILE),
            CODING_FILE_CONTENT,
        )
        .expect("seed preserved newline workspace");
        assert!(validate_apply_patch_args(
            &json!({
                "files": [{
                    "file_path": CODING_FILE,
                    "edits": [{
                        "old_string": "alpha",
                        "new_string": "alpha\nbeta"
                    }]
                }]
            }),
            preserved_newline_root.path(),
        ));
        assert!(!validate_apply_patch_args(
            &json!({
                "files": [{
                    "file_path": CODING_FILE,
                    "edits": [{
                        "old_string": "alpha",
                        "new_string": "beta"
                    }]
                }]
            }),
            preserved_newline_root.path(),
        ));
        assert!(validate_exec_command_args(
            &json!({"cmd": "git --version"}),
            root.path(),
        ));
        assert!(validate_grep_args(
            &json!({"pattern": "beta", "path": CODING_FILE}),
            root.path(),
        ));
        assert!(validate_vcs_status_args(&json!({}), root.path()));
        assert!(validate_vcs_diff_args(
            &json!({"path": CODING_FILE}),
            root.path(),
        ));
        assert!(validate_vcs_stage_args(
            &json!({"path": CODING_FILE}),
            root.path(),
        ));
        assert!(validate_vcs_commit_args(
            &json!({"message": CODING_COMMIT_MESSAGE}),
            root.path(),
        ));
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
