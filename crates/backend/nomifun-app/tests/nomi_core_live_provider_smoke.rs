//! Ignored product-chain smoke against a real Step Plan provider.
//!
//! Run explicitly with the credential-isolating runner:
//! `bun scripts/validation/run-nomi-core-live-provider-smoke.mjs`

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
const STEPFUN_SOURCE_MODEL: &str = "step-3.7-flash-source-placeholder";
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
const CODING_CAPABILITIES: &[&str] = &[
    "fs.read",
    "fs.search",
    "fs.write",
    "fs.patch",
    "process.exec",
    "vcs.status",
    "vcs.diff",
    "vcs.stage",
    "vcs.commit",
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
            SmokeFailure::new(
                phase,
                "RESPONSE_JSON_INVALID",
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
    let source_model = successful_json(
        router,
        "provider.source_model",
        Method::PUT,
        "/api/provider-models",
        Some(json!({
            "provider_id": provider_id,
            "model": {
                "model": STEPFUN_SOURCE_MODEL,
                "enabled": true,
                "sort_order": 1,
                "capabilities": [{
                    "task": "chat",
                    "traits": ["function_calling", "reasoning", "streaming"],
                    "protocol": "openai.chat_text",
                    "connection_role": "default",
                    "provider_params": {"temperature": 0.0},
                    "output_limit": 4096
                }]
            }
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let _ = envelope_data("provider.source_model", source_model)?;
    Ok(provider_id)
}

async fn create_agent_preset(
    router: &Router,
    provider_id: &str,
    model: &str,
) -> Result<(String, Value), SmokeFailure> {
    let created = successful_json(
        router,
        "agent_settings.create",
        Method::POST,
        "/api/agent-presets/from-template/chat.minimal",
        Some(json!({
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
    let mut selections = Vec::with_capacity(CODING_CAPABILITIES.len());
    for capability_id in CODING_CAPABILITIES {
        let required_resource_kind = if *capability_id == "process.exec" {
            "process_session"
        } else {
            "workspace"
        };
        let item = capabilities
            .iter()
            .find(|item| {
                item.pointer("/capability/id").and_then(Value::as_str) == Some(*capability_id)
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
            "action_allowlist": []
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
            "Use the exact requested native tools. Never replace a dedicated filesystem or VCS tool with shell commands. Stop immediately on the first tool error."
                .to_owned(),
        ),
    );
    let preview = successful_json(
        router,
        "agent_settings.preview",
        Method::POST,
        format!("/api/agent-presets/{preset_id}/resolve-preview"),
        Some(json!({
            "expected_current_revision": revision,
            "draft": draft,
            "scene": "agent_settings",
            "surface": "desktop",
            "audience": "owner"
        })),
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let preview = envelope_data("agent_settings.preview", preview)?;
    if preview.get("status").and_then(Value::as_str) != Some("ready")
        || preview.get("can_create_session").and_then(Value::as_bool) != Some(true)
        || preview.pointer("/summary/enabled_count").and_then(Value::as_u64)
            != Some(CODING_CAPABILITIES.len() as u64)
        || preview
            .pointer("/inspector/runtime_profile")
            .and_then(Value::as_str)
            != Some("managed_minimal")
    {
        return Err(SmokeFailure::new(
            "agent_settings.preview",
            preview
                .pointer("/diagnostics/0/code")
                .and_then(Value::as_str)
                .unwrap_or("PREVIEW_NOT_READY"),
            StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
        ));
    }
    let preview_digest = required_string(
        "agent_settings.preview",
        &preview,
        "/preview_digest",
        "PREVIEW_DIGEST_MISSING",
    )?;
    let saved = successful_json(
        router,
        "agent_settings.save",
        Method::POST,
        format!("/api/agent-presets/{preset_id}/revisions"),
        Some(json!({
            "expected_current_revision": revision,
            "preview_digest": preview_digest,
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
            .is_none_or(|items| items.is_empty())
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
    let mut expected_ids = CODING_CAPABILITIES
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    expected_ids.sort();
    if saved_ids != expected_ids {
        return Err(SmokeFailure::new(
            "agent_settings.save",
            "CODING_CAPABILITY_SET_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
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

async fn create_session(
    router: &Router,
    preset_id: &str,
    provider_id: &str,
    model: &str,
) -> Result<(String, Value), SmokeFailure> {
    let created = successful_json(
        router,
        "session.create",
        Method::POST,
        "/api/agent-sessions",
        Some(json!({
            "preset_id": preset_id,
            "title": "Live Step Plan session",
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
    let conversation = successful_json(
        router,
        "session.model_projection",
        Method::GET,
        format!("/api/conversations/{session_id}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let conversation = envelope_data("session.model_projection", conversation)?;
    if conversation.pointer("/model/provider_id") != Some(&Value::String(provider_id.to_owned()))
        || conversation.pointer("/model/model") != Some(&Value::String(model.to_owned()))
    {
        return Err(SmokeFailure::new(
            "session.model_projection",
            "SESSION_MODEL_OVERRIDE_MISMATCH",
            StatusCode::CONFLICT.as_u16(),
        ));
    }
    let binding = require_value(
        "session.create",
        &session,
        "/agent_binding",
        "SESSION_BINDING_MISSING",
    )?;
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
    if projection
        .pointer("/extra/workspace")
        .and_then(Value::as_str)
        != Some(workspace.as_str())
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
) -> Result<(), SmokeFailure> {
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
    Ok(())
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
            return Some(code);
        }
        return Some("TOOL_OR_TURN_FAILED".to_owned());
    }
    None
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
    if message.get("presentation_intent").and_then(Value::as_str) != Some("left") {
        return None;
    }
    let projection = message.get("projection")?;
    let object = projection.as_object()?;
    (object.get("name").is_none()
        && object.get("status").is_none()
        && object.get("type").is_none()
        && object.get("error").is_none()
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
) -> Result<(), SmokeFailure> {
    let deadline = tokio::time::Instant::now() + CODING_STAGE_DEADLINE;
    loop {
        // Re-read the complete stage window on every poll. Tool projections are
        // updated in place from running to completed, so advancing the cursor
        // permanently would miss the terminal update even though pagination is
        // otherwise exclusive.
        let (messages, _) = session_messages_after(router, phase, session_id, after_seq).await?;
        if let Some(code) = first_durable_error_code(&messages) {
            return Err(SmokeFailure::new(
                phase,
                code,
                StatusCode::UNPROCESSABLE_ENTITY.as_u16(),
            ));
        }
        let latest_evidence =
            inspect_coding_evidence(&messages, marker, expected_tools, workspace);
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
            return Ok(());
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
            message
                .pointer("/projection/name")
                .and_then(Value::as_str)
                .is_some()
        });
        let assistant_text = messages
            .iter()
            .filter_map(assistant_text_projection)
            .collect::<Vec<_>>();
        let latest_marker_count = exact_assistant_marker_count(&messages, marker);
        let exact_marker = assistant_text.len() == 1
            && latest_marker_count == 1
            && assistant_text[0]
                .get("turn_id")
                .and_then(Value::as_str)
                .is_some_and(|turn_id| !turn_id.is_empty());
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
            "model_route_refs": {}, "chat_route_records": {}})),
        LOCAL_API_DEADLINE, &[StatusCode::OK],
    ).await?;
    let official = envelope_data("guid.official_prepare", official)?;
    let minimal_id = required_string("guid.official_prepare", &official, "/preset/preset_id", "PRESET_ID_MISSING")?;
    let (minimal_session, _) = create_session(router, &minimal_id, &provider_id, model).await?;
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
        create_agent_preset(router, &provider_id, STEPFUN_SOURCE_MODEL).await?;

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
    successful_json(
        router,
        "session.switch_agent",
        Method::PUT,
        format!("/api/agent-sessions/{minimal_session}/preset"),
        Some(json!({"preset_id": preset_id})),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
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
        create_session(router, &preset_id, &provider_id, model).await?;
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

async fn run_live_provider_smoke() -> Result<(), SmokeFailure> {
    let api_key = required_secret_from_stdin()?;
    let root = tempfile::tempdir().map_err(|_| {
        SmokeFailure::new(
            "bootstrap",
            "TEMP_ROOT_CREATE_FAILED",
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
        )
    })?;
    let workspace = root.path().join("coding-workspace");
    initialize_git_workspace(&workspace).await?;
    let fixture = build_fixture(&root).await?;
    let router = fixture.application.router();

    let result = run_product_chain(
        &router,
        api_key.as_str(),
        STEPFUN_PLAN_BASE_URL,
        STEPFUN_PLAN_MODEL,
        &workspace,
    )
    .await;
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
    result
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a live credential on stdin; run with the credential-isolating runner"]
async fn nomi_core_product_chain_reaches_live_stepfun_and_remote_binding() {
    if let Err(failure) = run_live_provider_smoke().await {
        eprintln!("NOMIFUN_LIVE_SMOKE_FAILURE {failure}");
        panic!("NOMIFUN_LIVE_SMOKE_FAILED");
    }
}

#[cfg(test)]
mod evidence_tests {
    use super::*;

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
