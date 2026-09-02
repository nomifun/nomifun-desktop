//! Application-owned Wave 2 capability host.
//!
//! Only operations with an existing typed, owner-scoped resource API are
//! configured here. Unsupported families fail closed instead of delegating to
//! the legacy Gateway or manufacturing an acknowledgement.

use std::collections::{BTreeSet, HashMap};
use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use nomifun_agent_contracts::{
    DigestHex, PluginStateCompareAndSwapOutcome, ScopeKey, StateKey, StrictJsonValue,
    TypedResourceBinding, VersionString, canonical_json_bytes, digest_payload,
};
use nomifun_agent_domain_wave2::{
    Wave2CapabilityOperation, Wave2HostContext, Wave2HostPort, Wave2HostPortError,
    Wave2HostRequest, Wave2StateHandle,
};
use nomifun_api_types::{TypedResourceBindingDto, WebSocketMessage};
use nomifun_common::AppError;
use nomifun_file::{
    AgentSessionPatchRequest, AgentSessionWorkspaceBinding, FileService,
    ISnapshotService, SnapshotInfo, SnapshotMode, SnapshotService,
    WORKSPACE_READ_OPERATION, WORKSPACE_RESOURCE_KIND, WORKSPACE_ROOT_PARAMETER,
    WORKSPACE_WRITE_OPERATION,
};
use nomifun_realtime::UserEventSink;
use nomifun_terminal::pty::{PtyExit, PtyHandle, SpawnParams};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_SEARCH_QUERY_CHARS: usize = 1024;
const MAX_SEARCH_LINE_CHARS: usize = 4096;
const MAX_DIFF_BYTES: usize = 1024 * 1024;
const MAX_SNAPSHOT_CHANGES: usize = 512;
const MAX_SNAPSHOT_BASELINE_BYTES: usize = 1024 * 1024;
const WAVE2_EFFECT_STATE_FORMAT: &str = "1.0.0";
const MAX_WAVE2_EFFECT_RECORDS: usize = 128;
const MAX_WAVE2_EFFECT_CAS_ATTEMPTS: usize = 8;
const MAX_WAVE2_IDEMPOTENCY_KEY_BYTES: usize = 128;
const PROCESS_SESSION_RESOURCE_KIND: &str = "process_session";
const PROCESS_EXECUTE_OPERATION: &str = "execute";
const DEFAULT_PROCESS_TIMEOUT_MS: u64 = 30_000;
const MAX_PROCESS_TIMEOUT_MS: u64 = 10 * 60 * 1000;
const MAX_PROCESS_COMMAND_CHARS: usize = 32 * 1024;
const MAX_PROCESS_ARGUMENTS: usize = 256;
const MAX_PROCESS_ARGUMENT_CHARS: usize = 64 * 1024;
const MAX_PROCESS_ENVIRONMENT_ENTRIES: usize = 128;
const MAX_PROCESS_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_VCS_STAGE_ENTRIES: usize = 100_000;

#[derive(Clone)]
pub(crate) struct Wave2ApplicationHost {
    files: Arc<FileService>,
    snapshots: Arc<SnapshotService>,
    snapshot_sessions: Arc<tokio::sync::Mutex<BTreeSet<(String, String)>>>,
    workspace_write_lock: Arc<tokio::sync::Mutex<()>>,
    configured_workspace_root: PathBuf,
}

#[derive(Clone)]
struct Wave2EffectReservation {
    state: Wave2StateHandle,
    scope: ScopeKey,
    state_key: StateKey,
    idempotency_key: String,
    request_digest: DigestHex,
}

enum Wave2EffectAdmission {
    Replay(StrictJsonValue),
    Reserved(Wave2EffectReservation),
}

#[derive(Clone, Copy)]
enum Wave2EffectCompletion<'a> {
    Succeeded(&'a StrictJsonValue),
    Failed(&'a Wave2HostPortError),
}

impl Wave2ApplicationHost {
    pub(crate) fn new() -> Self {
        Self::for_workspace_root(std::env::temp_dir())
    }

    pub(crate) fn for_workspace_root(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        Self {
            files: Arc::new(FileService::new(
                Arc::new(NullUserEvents),
                vec![workspace_root.clone()],
            )),
            snapshots: Arc::new(SnapshotService::new()),
            snapshot_sessions: Arc::new(tokio::sync::Mutex::new(BTreeSet::new())),
            workspace_write_lock: Arc::new(tokio::sync::Mutex::new(())),
            configured_workspace_root: workspace_root,
        }
    }
}

impl Default for Wave2ApplicationHost {
    fn default() -> Self {
        Self::new()
    }
}

fn wave2_effect_scope(
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
) -> Result<ScopeKey, Wave2HostPortError> {
    let state_descriptor = context.state.descriptor();
    if state_descriptor.package_id.as_ref()
        != nomifun_agent_domain_wave2::WORKSPACE_EXECUTION_PACKAGE_ID
        || state_descriptor.mount_id.as_ref()
            != nomifun_agent_domain_wave2::WORKSPACE_EXECUTION_MOUNT_ID
    {
        return Err(Wave2HostPortError::unavailable(
            "Wave 2 workspace state handle is not owned by the workspace package mount",
        ));
    }
    if binding.resource_id.as_ref().trim().is_empty() {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            "workspace effect journal requires a non-empty resource ID",
        ));
    }
    let scope = ScopeKey::from(format!("resource:{}", binding.resource_id.as_ref()));
    if scope.as_ref().len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_KEY_BYTES {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            "workspace effect journal scope exceeds the PluginState key limit",
        ));
    }
    Ok(scope)
}

fn wave2_effect_request_digest(
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
) -> Result<DigestHex, Wave2HostPortError> {
    let fingerprint = json!({
        "capability_id": context.capability_id.as_ref(),
        "action_id": context.action_id.as_ref(),
        "resource_binding": binding,
        "input": input.0,
    });
    digest_payload(&fingerprint).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 effect request could not be canonicalized: {error}"),
        )
    })
}

fn wave2_effect_state_key(
    context: &Wave2HostContext,
) -> Result<StateKey, Wave2HostPortError> {
    let key = format!("action.idempotency.{}", context.capability_id.as_ref());
    if key.len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_KEY_BYTES {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            "Wave 2 effect journal state key exceeds the PluginState key limit",
        ));
    }
    Ok(StateKey::from(key))
}

fn decode_wave2_effect_records(
    current: Option<&nomifun_agent_contracts::PluginStateEntry>,
) -> Result<Vec<Value>, Wave2HostPortError> {
    let Some(current) = current else {
        return Ok(Vec::new());
    };
    if current.revision == 0 {
        return Err(Wave2HostPortError::unavailable(
            "Wave 2 effect journal has an invalid zero revision",
        ));
    }
    if current.state_format_version.as_ref() != WAVE2_EFFECT_STATE_FORMAT {
        return Err(Wave2HostPortError::unavailable(format!(
            "Wave 2 effect journal format {} is unsupported; expected {}",
            current.state_format_version.as_ref(),
            WAVE2_EFFECT_STATE_FORMAT
        )));
    }
    let object = current.value.0.as_object().ok_or_else(|| {
        Wave2HostPortError::unavailable("Wave 2 effect journal has an invalid stored shape")
    })?;
    if object.keys().any(|key| key != "entries") {
        return Err(Wave2HostPortError::unavailable(
            "Wave 2 effect journal contains unknown top-level fields",
        ));
    }
    let entries = object
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            Wave2HostPortError::unavailable(
                "Wave 2 effect journal entries must be an array",
            )
        })?;
    if entries.len() > MAX_WAVE2_EFFECT_RECORDS {
        return Err(Wave2HostPortError::unavailable(format!(
            "Wave 2 effect journal contains more than {MAX_WAVE2_EFFECT_RECORDS} records"
        )));
    }
    let mut keys = std::collections::BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let object = entry.as_object().ok_or_else(|| {
            Wave2HostPortError::unavailable(format!(
                "Wave 2 effect journal record {index} is not an object"
            ))
        })?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "idempotency_key" | "request_digest" | "status" | "result"
                    | "error_code" | "error_message"
            )
        }) {
            return Err(Wave2HostPortError::unavailable(format!(
                "Wave 2 effect journal record {index} contains unknown fields"
            )));
        }
        let key = object
            .get("idempotency_key")
            .and_then(Value::as_str)
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                Wave2HostPortError::unavailable(format!(
                    "Wave 2 effect journal record {index} has an invalid idempotency key"
                ))
            })?;
        if !keys.insert(key.to_owned()) {
            return Err(Wave2HostPortError::unavailable(format!(
                "Wave 2 effect journal contains duplicate idempotency key at record {index}"
            )));
        }
        let digest = object
            .get("request_digest")
            .and_then(Value::as_str)
            .filter(|digest| {
                digest.len() == 64
                    && digest.bytes().all(|byte| {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    })
            })
            .ok_or_else(|| {
                Wave2HostPortError::unavailable(format!(
                    "Wave 2 effect journal record {index} has an invalid request digest"
                ))
            })?;
        let _ = digest;
        let status = object
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Wave2HostPortError::unavailable(format!(
                    "Wave 2 effect journal record {index} has no status"
                ))
            })?;
        match status {
            "started" => {
                if object.contains_key("result")
                    || object.contains_key("error_code")
                    || object.contains_key("error_message")
                {
                    return Err(Wave2HostPortError::unavailable(format!(
                        "Wave 2 started effect record {index} has terminal fields"
                    )));
                }
            }
            "completed" => {
                if object.get("result").is_none_or(Value::is_null)
                    || object.contains_key("error_code")
                    || object.contains_key("error_message")
                {
                    return Err(Wave2HostPortError::unavailable(format!(
                        "Wave 2 completed effect record {index} is incomplete"
                    )));
                }
            }
            "failed" => {
                if object
                    .get("error_code")
                    .and_then(Value::as_str)
                    .is_none_or(|code| code.trim().is_empty())
                    || object
                        .get("error_message")
                        .and_then(Value::as_str)
                        .is_none_or(|message| message.trim().is_empty())
                    || object.contains_key("result")
                {
                    return Err(Wave2HostPortError::unavailable(format!(
                        "Wave 2 failed effect record {index} is incomplete"
                    )));
                }
            }
            _ => {
                return Err(Wave2HostPortError::unavailable(format!(
                    "Wave 2 effect journal record {index} has an unknown status"
                )));
            }
        }
        let bytes = canonical_json_bytes(entry).map_err(|error| {
            Wave2HostPortError::unavailable(format!(
                "Wave 2 effect journal record {index} could not be encoded: {error}"
            ))
        })?;
        if bytes.len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_BYTES {
            return Err(Wave2HostPortError::unavailable(format!(
                "Wave 2 effect journal record {index} exceeds the PluginState limit"
            )));
        }
    }
    let bytes = canonical_json_bytes(&current.value.0).map_err(|error| {
        Wave2HostPortError::unavailable(format!(
            "Wave 2 effect journal could not be encoded: {error}"
        ))
    })?;
    if bytes.len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_BYTES {
        return Err(Wave2HostPortError::unavailable(
            "Wave 2 effect journal exceeds the PluginState limit",
        ));
    }
    Ok(entries.clone())
}

async fn begin_wave2_effect(
    context: &Wave2HostContext,
    binding: &TypedResourceBinding,
    input: &StrictJsonValue,
) -> Result<Wave2EffectAdmission, Wave2HostPortError> {
    let idempotency_key = context.idempotency_key.as_ref().trim();
    if idempotency_key.is_empty() {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            "Wave 2 effect action requires a non-empty idempotency key",
        ));
    }
    if idempotency_key.len() > MAX_WAVE2_IDEMPOTENCY_KEY_BYTES
        || !idempotency_key
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
    {
        return Err(Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!(
                "Wave 2 effect idempotency key must be 1..={MAX_WAVE2_IDEMPOTENCY_KEY_BYTES} visible ASCII bytes"
            ),
        ));
    }
    let scope = wave2_effect_scope(context, binding)?;
    let digest = wave2_effect_request_digest(context, binding, input)?;
    let state_key = wave2_effect_state_key(context)?;
    let format = VersionString::from(WAVE2_EFFECT_STATE_FORMAT);
    for _attempt in 0..MAX_WAVE2_EFFECT_CAS_ATTEMPTS {
        let current = context.state.get(&scope, &state_key).await.map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("Wave 2 effect journal could not be read: {error}"),
            )
        })?;
        let revision = current.as_ref().map_or(0, |entry| entry.revision);
        let mut entries = decode_wave2_effect_records(current.as_ref())?;
        if let Some(previous) = entries.iter().find(|entry| {
            entry
                .get("idempotency_key")
                .and_then(Value::as_str)
                == Some(idempotency_key)
        }) {
            let previous_digest = previous
                .get("request_digest")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    Wave2HostPortError::unavailable(
                        "Wave 2 effect journal record has no request digest",
                    )
                })?;
            if previous_digest != digest.as_ref() {
                return Err(Wave2HostPortError::new(
                    "IDEMPOTENCY_CONFLICT",
                    "Wave 2 idempotency key was already used for different input",
                ));
            }
            match previous.get("status").and_then(Value::as_str) {
                Some("completed") => {
                    let result = previous.get("result").cloned().ok_or_else(|| {
                        Wave2HostPortError::unavailable(
                            "Wave 2 completed effect record has no result",
                        )
                    })?;
                    return Ok(Wave2EffectAdmission::Replay(StrictJsonValue(result)));
                }
                Some("failed") => {
                    let code = previous
                        .get("error_code")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            Wave2HostPortError::unavailable(
                                "Wave 2 failed effect record has no error code",
                            )
                        })?;
                    let message = previous
                        .get("error_message")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            Wave2HostPortError::unavailable(
                                "Wave 2 failed effect record has no error message",
                            )
                        })?;
                    return Err(Wave2HostPortError::new(code, message));
                }
                Some("started") => {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "Wave 2 effect outcome is in progress or uncertain; automatic retry is disabled",
                    ));
                }
                _ => {
                    return Err(Wave2HostPortError::unavailable(
                        "Wave 2 effect journal record has an invalid status",
                    ));
                }
            }
        }
        if entries.len() >= MAX_WAVE2_EFFECT_RECORDS {
            return Err(Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!(
                    "Wave 2 effect journal reached its {MAX_WAVE2_EFFECT_RECORDS}-record limit"
                ),
            ));
        }
        entries.push(json!({
            "idempotency_key": idempotency_key,
            "request_digest": digest.as_ref(),
            "status": "started",
        }));
        let next = StrictJsonValue(json!({"entries": entries}));
        let bytes = canonical_json_bytes(&next.0).map_err(|error| {
            Wave2HostPortError::new(
                "INVALID_PAYLOAD",
                format!("Wave 2 effect journal could not be encoded: {error}"),
            )
        })?;
        if bytes.len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_BYTES {
            return Err(Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                "Wave 2 effect journal would exceed the PluginState limit",
            ));
        }
        match context
            .state
            .compare_and_swap(&scope, &state_key, revision, &format, Some(next))
            .await
            .map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("Wave 2 effect journal could not reserve the action: {error}"),
                )
            })? {
            PluginStateCompareAndSwapOutcome::Applied { .. } => {
                return Ok(Wave2EffectAdmission::Reserved(Wave2EffectReservation {
                    state: context.state.clone(),
                    scope,
                    state_key,
                    idempotency_key: idempotency_key.to_owned(),
                    request_digest: digest,
                }));
            }
            PluginStateCompareAndSwapOutcome::Conflict { .. } => continue,
        }
    }
    Err(Wave2HostPortError::new(
        "CAPABILITY_UNAVAILABLE",
        "Wave 2 effect journal changed concurrently; bounded CAS retry exhausted",
    ))
}

async fn finish_wave2_effect(
    reservation: &Wave2EffectReservation,
    completion: Wave2EffectCompletion<'_>,
) -> Result<(), Wave2HostPortError> {
    let state_key = reservation.state_key.clone();
    let format = VersionString::from(WAVE2_EFFECT_STATE_FORMAT);
    for _attempt in 0..MAX_WAVE2_EFFECT_CAS_ATTEMPTS {
        let current = reservation
            .state
            .get(&reservation.scope, &state_key)
            .await
            .map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("Wave 2 effect journal could not be read for completion: {error}"),
                )
            })?;
        let revision = current.as_ref().map_or(0, |entry| entry.revision);
        let mut entries = decode_wave2_effect_records(current.as_ref())?;
        let Some(record) = entries.iter_mut().find(|entry| {
            entry
                .get("idempotency_key")
                .and_then(Value::as_str)
                == Some(reservation.idempotency_key.as_str())
        }) else {
            return Err(Wave2HostPortError::unavailable(
                "Wave 2 effect reservation disappeared before completion",
            ));
        };
        if record
            .get("request_digest")
            .and_then(Value::as_str)
            != Some(reservation.request_digest.as_ref())
        {
            return Err(Wave2HostPortError::new(
                "IDEMPOTENCY_CONFLICT",
                "Wave 2 effect reservation digest changed before completion",
            ));
        }
        match record.get("status").and_then(Value::as_str) {
            Some("completed") | Some("failed") => return Ok(()),
            Some("started") => {}
            _ => {
                return Err(Wave2HostPortError::unavailable(
                    "Wave 2 effect reservation has an invalid status",
                ));
            }
        }
        let Some(record_object) = record.as_object_mut() else {
            return Err(Wave2HostPortError::unavailable(
                "Wave 2 effect reservation is not an object",
            ));
        };
        match completion {
            Wave2EffectCompletion::Succeeded(output) => {
                record_object.insert("status".to_owned(), json!("completed"));
                record_object.insert("result".to_owned(), output.0.clone());
            }
            Wave2EffectCompletion::Failed(error) => {
                record_object.insert("status".to_owned(), json!("failed"));
                record_object.insert("error_code".to_owned(), json!(error.code.as_str()));
                record_object.insert("error_message".to_owned(), json!(error.message));
            }
        }
        let next = StrictJsonValue(json!({"entries": entries}));
        let bytes = canonical_json_bytes(&next.0).map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("Wave 2 effect completion could not be encoded: {error}"),
            )
        })?;
        if bytes.len() > nomifun_agent_kernel::MAX_PLUGIN_STATE_BYTES {
            return Err(Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                "Wave 2 effect completion exceeds the PluginState limit",
            ));
        }
        match reservation
            .state
            .compare_and_swap(&reservation.scope, &state_key, revision, &format, Some(next))
            .await
            .map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("Wave 2 effect completion could not be committed: {error}"),
                )
            })? {
            PluginStateCompareAndSwapOutcome::Applied { .. } => return Ok(()),
            PluginStateCompareAndSwapOutcome::Conflict { .. } => continue,
        }
    }
    Err(Wave2HostPortError::new(
        "CAPABILITY_UNAVAILABLE",
        "Wave 2 effect completion changed concurrently; bounded CAS retry exhausted",
    ))
}

impl Wave2HostPort for Wave2ApplicationHost {
    fn invoke<'a>(
        &'a self,
        request: Wave2HostRequest,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let capability_id = request.context.capability_id.as_ref().to_owned();
            let expected_action = format!("{capability_id}.invoke");
            if request.context.action_id.as_ref() != expected_action {
                return Err(Wave2HostPortError::new(
                    "INVALID_PAYLOAD",
                    format!(
                        "{capability_id} action identity does not match the host context"
                    ),
                ));
            }
            match request.operation {
                Wave2CapabilityOperation::WorkspaceExecution { input } => {
                    self.invoke_workspace(&request.context, &capability_id, input)
                        .await
                }
                Wave2CapabilityOperation::Ssh { .. }
                | Wave2CapabilityOperation::McpConnectors { .. }
                | Wave2CapabilityOperation::Browser { .. }
                | Wave2CapabilityOperation::ComputerA11y { .. } => {
                    Err(unavailable(&capability_id))
                }
            }
        })
    }
}

impl Wave2ApplicationHost {
    async fn invoke_workspace(
        &self,
        context: &Wave2HostContext,
        capability_id: &str,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        if capability_id == "process.exec" {
            return self.invoke_process_exec(context, input).await;
        }

        let scope = self.workspace_scope(context)?;
        match capability_id {
            "fs.read" => {
                let params: PathParams = decode(input)?;
                let content = self
                    .files
                    .read_file_for_agent_session(&scope, &params.path)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?
                    .ok_or_else(|| {
                        Wave2HostPortError::new(
                            "RESOURCE_NOT_FOUND",
                            format!("workspace file '{}' was not found", params.path),
                        )
                    })?;
                Ok(StrictJsonValue(json!({
                    "path": params.path,
                    "content": content
                })))
            }
            "fs.write" => {
                let params: WriteParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                match begin_wave2_effect(context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let _write_guard = self.workspace_write_lock.lock().await;
                        let result = self
                            .files
                            .write_file_for_agent_session(
                                &scope,
                                &params.path,
                                params.content.as_bytes(),
                            )
                            .await;
                        match result {
                            Ok(created) => {
                                let output = StrictJsonValue(json!({
                                    "path": params.path,
                                    "written": true,
                                    "created": created
                                }));
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) => {
                                let owner_error = operation_error(capability_id, error);
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "fs.patch" => {
                let request: AgentSessionPatchRequest = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(
                    serde_json::to_value(&request).map_err(|error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    })?,
                );
                match begin_wave2_effect(context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let _write_guard = self.workspace_write_lock.lock().await;
                        let result = self.files.apply_patch_for_agent_session(&scope, request).await;
                        match result {
                            Ok(result) => {
                                let output = StrictJsonValue(
                                    serde_json::to_value(result).map_err(|error| {
                                        Wave2HostPortError::new(
                                            "CAPABILITY_UNAVAILABLE",
                                            format!(
                                                "{capability_id} result could not be encoded: {error}"
                                            ),
                                        )
                                    })?,
                                );
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) => {
                                let owner_error = operation_error(capability_id, error);
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "fs.delete" => {
                let params: PathParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                match begin_wave2_effect(context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        let _write_guard = self.workspace_write_lock.lock().await;
                        let result = self
                            .files
                            .remove_entry_for_agent_session(&scope, &params.path)
                            .await;
                        match result {
                            Ok(()) => {
                                let output = StrictJsonValue(json!({
                                    "path": params.path,
                                    "deleted": true
                                }));
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(error) => {
                                let owner_error = operation_error(capability_id, error);
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "fs.search" => {
                let params: SearchParams = decode(input)?;
                let query = params.query.trim();
                if query.is_empty() {
                    return Err(Wave2HostPortError::invalid_payload(
                        "fs.search query must not be empty",
                    ));
                }
                if query.chars().count() > MAX_SEARCH_QUERY_CHARS {
                    return Err(Wave2HostPortError::invalid_payload(format!(
                        "fs.search query must not exceed {MAX_SEARCH_QUERY_CHARS} characters"
                    )));
                }
                let limit = params.limit.unwrap_or(100);
                if !(1..=200).contains(&limit) {
                    return Err(Wave2HostPortError::invalid_payload(
                        "fs.search limit must be between 1 and 200",
                    ));
                }
                let prefix = params
                    .path
                    .as_deref()
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(|path| {
                        scope
                            .resolve_relative_path(path)
                            .and_then(|resolved| {
                                resolved
                                    .strip_prefix(scope.workspace_root())
                                    .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                                    .map_err(|_| {
                                        AppError::BadRequest(
                                            "fs.search path is outside the workspace".to_owned(),
                                        )
                                    })
                            })
                    })
                    .transpose()
                    .map_err(|error| operation_error(capability_id, error))?;
                let files = self
                    .files
                    .list_workspace_files_for_agent_session(&scope)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                let mut matches = Vec::new();
                let mut truncated = false;
                for file in files {
                    let relative_path = file.relative_path.replace('\\', "/");
                    if prefix
                        .as_deref()
                        .is_some_and(|prefix| {
                            !relative_path
                                .strip_prefix(prefix)
                                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
                        })
                    {
                        continue;
                    }
                    let Some(content) = self
                        .files
                        .read_file_for_agent_session(&scope, &relative_path)
                        .await
                        .map_err(|error| operation_error(capability_id, error))?
                    else {
                        continue;
                    };
                    for (line_index, line) in content.lines().enumerate() {
                        if !line.contains(query) {
                            continue;
                        }
                        if matches.len() == limit {
                            truncated = true;
                            break;
                        }
                        let text = line.chars().take(MAX_SEARCH_LINE_CHARS).collect::<String>();
                        matches.push(json!({
                            "path": &relative_path,
                            "line": line_index + 1,
                            "text": text,
                            "truncated": line.chars().count() > MAX_SEARCH_LINE_CHARS
                        }));
                    }
                    if truncated {
                        break;
                    }
                }
                Ok(StrictJsonValue(json!({
                    "query": query,
                    "matches": matches,
                    "truncated": truncated
                })))
            }
            "fs.snapshot" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: SnapshotParams = decode(input)?;
                self.invoke_snapshot(context, &scope, capability_id, params)
                    .await
            }
            "vcs.status" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                self.invoke_vcs_status(&scope, capability_id).await
            }
            "vcs.diff" => {
                scope
                    .require_operation(WORKSPACE_READ_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: VcsPathParams = decode(input)?;
                self.invoke_vcs_diff(&scope, capability_id, params.path.as_deref())
                    .await
            }
            "vcs.stage" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: PathParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(serde_json::to_value(&params).map_err(
                    |error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    },
                )?);
                match begin_wave2_effect(context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        match self
                            .invoke_vcs_stage(&scope, capability_id, &params.path)
                            .await
                        {
                            Ok(output) => {
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(owner_error) => {
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            "vcs.commit" => {
                scope
                    .require_operation(WORKSPACE_WRITE_OPERATION)
                    .map_err(|error| operation_error(capability_id, error))?;
                let params: VcsCommitParams = decode(input)?;
                let binding = workspace_typed_binding(context)?;
                let effect_input = StrictJsonValue(
                    serde_json::to_value(&params).map_err(|error| {
                        Wave2HostPortError::new(
                            "INVALID_PAYLOAD",
                            format!("{capability_id} input could not be encoded: {error}"),
                        )
                    })?,
                );
                match begin_wave2_effect(context, binding, &effect_input).await? {
                    Wave2EffectAdmission::Replay(output) => Ok(output),
                    Wave2EffectAdmission::Reserved(reservation) => {
                        match self
                            .invoke_vcs_commit(&scope, capability_id, &params.message)
                            .await
                        {
                            Ok(output) => {
                                finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Succeeded(&output),
                                )
                                .await?;
                                Ok(output)
                            }
                            Err(owner_error) => {
                                let _ = finish_wave2_effect(
                                    &reservation,
                                    Wave2EffectCompletion::Failed(&owner_error),
                                )
                                .await;
                                Err(owner_error)
                            }
                        }
                    }
                }
            }
            _ => Err(unavailable(capability_id)),
        }
    }

    async fn invoke_process_exec(
        &self,
        context: &Wave2HostContext,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let binding = self.process_binding(context)?;
        let process_session_id = binding.resource_id.as_ref().to_owned();
        let requested_root = binding
            .typed_parameters
            .get(WORKSPACE_ROOT_PARAMETER)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Wave2HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "process_session binding {} has no host-resolved {} parameter",
                        binding.binding_id.as_ref(),
                        WORKSPACE_ROOT_PARAMETER
                    ),
                )
            })?;
        let process_root = resolve_allowed_workspace_root(
            &self.configured_workspace_root,
            requested_root,
        )?;
        let params: ProcessExecParams = decode(input)?;
        validate_process_exec_params(&params)?;
        let (cwd, cwd_label) =
            resolve_process_cwd(&process_root, params.cwd.as_deref())?;
        let timeout = Duration::from_millis(
            params.timeout_ms.unwrap_or(DEFAULT_PROCESS_TIMEOUT_MS),
        );
        let capture = Arc::new(Mutex::new(ProcessOutputCapture::default()));
        let capture_output = Arc::clone(&capture);
        let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
        let process = PtyHandle::spawn(
            SpawnParams {
                program: params.command,
                args: params.args,
                cwd: cwd.to_string_lossy().into_owned(),
                env: params.env,
                cols: 120,
                rows: 40,
            },
            1,
            move |chunk| append_process_output(&capture_output, &chunk),
            move |exit, _scrollback| {
                let _ = exit_tx.send(exit);
            },
        )
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("process.exec could not start a managed process: {error}"),
            )
        })?;
        process.activate();

        // The coordinator owns the process even if the capability request is
        // cancelled. It always observes an exit or times out and reaps the
        // complete process tree through the existing Windows/Unix owner.
        let worker = tokio::spawn(async move {
            match tokio::time::timeout(timeout, exit_rx).await {
                Ok(Ok(exit)) => Ok((exit, finish_process_output(&capture))),
                Ok(Err(_)) => Err(Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    "process.exec lost its managed exit observation",
                )),
                Err(_) => match process.kill().await {
                    Ok(()) => Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "process.exec timed out after {}ms; the managed process tree was reaped",
                            timeout.as_millis()
                        ),
                    )),
                    Err(error) => Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "process.exec timed out after {}ms and process-tree cleanup is unproven: {error}",
                            timeout.as_millis()
                        ),
                    )),
                },
            }
        });
        let (exit, output) = worker.await.map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("process.exec coordinator failed: {error}"),
            )
        })??;
        match exit {
            PtyExit::Exited(exit_code) => Ok(StrictJsonValue(json!({
                "process_session_id": process_session_id,
                "cwd": cwd_label,
                "success": exit_code == Some(0),
                "exit_code": exit_code,
                "output": output.text,
                "truncated": output.truncated
            }))),
            PtyExit::Lost {
                message,
                cleanup_reaped,
            } => Err(Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!(
                    "process.exec lost managed process ownership \
                     (cleanup_reaped={cleanup_reaped}): {message}"
                ),
            )),
        }
    }

    async fn invoke_vcs_status(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let status = tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let mut options = git2::StatusOptions::new();
            options
                .include_untracked(true)
                .recurse_untracked_dirs(true)
                .include_ignored(false);
            let statuses = repository.statuses(Some(&mut options)).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "{worker_capability_id} could not read Git status: {error}"
                    ),
                )
            })?;
            let mut entries = Vec::new();
            for entry in statuses.iter() {
                let Some(path) = entry.path() else {
                    continue;
                };
                let Some(path) = path_relative_to_workspace(path, &workspace_prefix) else {
                    continue;
                };
                entries.push(json!({
                    "path": path,
                    "status": git_status_name(entry.status())
                }));
            }
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "repository": "workspace",
                "entries": entries
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} status worker failed: {error}"),
            )
        })??;
        Ok(status)
    }

    async fn invoke_snapshot(
        &self,
        context: &Wave2HostContext,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        params: SnapshotParams,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_string_lossy().into_owned();
        let session_key = snapshot_session_key(context, scope);
        match params.operation {
            SnapshotOperation::Init => {
                let mut sessions = self.snapshot_sessions.lock().await;
                let info = if sessions.contains(&session_key) && self.snapshots.is_tracked(&workspace) {
                    self.snapshots
                        .info(&workspace)
                        .await
                        .map_err(|error| operation_error(capability_id, error))?
                } else {
                    let info = self
                        .snapshots
                        .init(&workspace)
                        .await
                        .map_err(|error| operation_error(capability_id, error))?;
                    sessions.insert(session_key);
                    info
                };
                Ok(StrictJsonValue(snapshot_info_value(info)))
            }
            SnapshotOperation::Compare => {
                let owned = self.snapshot_sessions.lock().await.contains(&session_key);
                if !owned || !self.snapshots.is_tracked(&workspace) {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "fs.snapshot compare requires a snapshot initialized by this AgentSession",
                    ));
                }
                let compare = self
                    .snapshots
                    .compare(&workspace)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                Ok(StrictJsonValue(snapshot_compare_value(compare)?))
            }
            SnapshotOperation::Baseline => {
                let path = params.path.as_deref().ok_or_else(|| {
                    Wave2HostPortError::invalid_payload(
                        "fs.snapshot baseline requires a workspace-relative path",
                    )
                })?;
                let path = path.trim();
                if path.is_empty() {
                    return Err(Wave2HostPortError::invalid_payload(
                        "fs.snapshot baseline path must not be empty",
                    ));
                }
                let path = scope
                    .resolve_relative_path(path)
                    .and_then(|resolved| {
                        resolved
                            .strip_prefix(scope.workspace_root())
                            .map(|relative| relative.to_string_lossy().replace('\\', "/"))
                            .map_err(|_| {
                                AppError::BadRequest(
                                    "fs.snapshot baseline path is outside the workspace"
                                        .to_owned(),
                                )
                            })
                    })
                    .map_err(|error| operation_error(capability_id, error))?;
                let owned = self.snapshot_sessions.lock().await.contains(&session_key);
                if !owned || !self.snapshots.is_tracked(&workspace) {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "fs.snapshot baseline requires a snapshot initialized by this AgentSession",
                    ));
                }
                let content = self
                    .snapshots
                    .get_baseline_content(&workspace, &path)
                    .await
                    .map_err(|error| operation_error(capability_id, error))?;
                let found = content.is_some();
                let (content, truncated) = match content {
                    Some(content) if content.len() > MAX_SNAPSHOT_BASELINE_BYTES => {
                        let mut end = MAX_SNAPSHOT_BASELINE_BYTES;
                        while end > 0 && !content.is_char_boundary(end) {
                            end -= 1;
                        }
                        (content[..end].to_owned(), true)
                    }
                    Some(content) => (content, false),
                    None => (String::new(), false),
                };
                Ok(StrictJsonValue(json!({
                    "path": path,
                    "content": content,
                    "found": found,
                    "truncated": truncated
                })))
            }
            SnapshotOperation::Dispose => {
                let mut sessions = self.snapshot_sessions.lock().await;
                if sessions.remove(&session_key) {
                    self.snapshots
                        .dispose(&workspace)
                        .await
                        .map_err(|error| operation_error(capability_id, error))?;
                }
                Ok(StrictJsonValue(json!({"disposed": true})))
            }
        }
    }

    async fn invoke_vcs_diff(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: Option<&str>,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let path = match path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) => {
                let resolved = scope
                    .resolve_relative_path(path)
                    .map_err(|error| operation_error(&capability_id, error))?;
                let relative = resolved.strip_prefix(scope.workspace_root()).map_err(|_| {
                    Wave2HostPortError::new(
                        "INVALID_PAYLOAD",
                        "vcs.diff path is outside the workspace",
                    )
                })?;
                Some(git_path_to_string(relative)?)
            }
            None => None,
        };
        let repository_prefix = scoped_repository(&workspace)
            .map(|(_, prefix)| prefix)
            .map_err(|error| error)?;
        let pathspec = path
            .as_deref()
            .map(|relative| join_repo_path(&repository_prefix, relative));
        tokio::task::spawn_blocking(move || {
            let (repository, actual_prefix) = scoped_repository(&workspace)?;
            debug_assert_eq!(actual_prefix, repository_prefix);
            let scope_pathspec = pathspec
                .as_deref()
                .or_else(|| (!repository_prefix.is_empty()).then_some(repository_prefix.as_str()));
            let head_tree = repository
                .head()
                .ok()
                .and_then(|head| head.peel_to_tree().ok());
            let mut staged_options = git2::DiffOptions::new();
            if let Some(pathspec) = scope_pathspec {
                staged_options.pathspec(pathspec);
            }
            let staged = repository
                .diff_tree_to_index(
                    head_tree.as_ref(),
                    None,
                    Some(&mut staged_options),
                )
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not read staged Git diff: {error}"
                        ),
                    )
                })?;
            let mut unstaged_options = git2::DiffOptions::new();
            if let Some(pathspec) = scope_pathspec {
                unstaged_options.pathspec(pathspec);
            }
            let unstaged = repository
                .diff_index_to_workdir(None, Some(&mut unstaged_options))
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not read unstaged Git diff: {error}"
                        ),
                    )
                })?;
            let mut staged_patch = String::new();
            let mut truncated = false;
            append_diff_patch(
                &staged,
                &mut staged_patch,
                &mut truncated,
                &worker_capability_id,
            )?;
            let mut unstaged_patch = String::new();
            append_diff_patch(
                &unstaged,
                &mut unstaged_patch,
                &mut truncated,
                &worker_capability_id,
            )?;
            let patch = format!("{staged_patch}{unstaged_patch}");
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path,
                "patch": patch,
                "staged_patch": staged_patch,
                "unstaged_patch": unstaged_patch,
                "truncated": truncated
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} diff worker failed: {error}"),
            )
        })?
    }

    async fn invoke_vcs_stage(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        path: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let path = path.trim();
        if path.is_empty() {
            return Err(Wave2HostPortError::invalid_payload(
                "vcs.stage path must not be empty",
            ));
        }
        let resolved = scope
            .resolve_relative_path(path)
            .map_err(|error| operation_error(capability_id, error))?;
        let relative = resolved
            .strip_prefix(scope.workspace_root())
            .map(|relative| relative.to_path_buf())
            .map_err(|_| {
                operation_error(
                    capability_id,
                    AppError::BadRequest(
                        "vcs.stage path is outside the workspace".to_owned(),
                    ),
                )
            })?;
        let target_exists = resolved.exists();
        let target_is_dir = resolved.is_dir();
        if target_exists {
            let canonical_workspace =
                std::fs::canonicalize(scope.workspace_root()).map_err(|error| {
                    Wave2HostPortError::new(
                        "PRESET_RESOURCE_NOT_BOUND",
                        format!("vcs.stage workspace is unavailable: {error}"),
                    )
                })?;
            let canonical_target = std::fs::canonicalize(&resolved).map_err(|error| {
                Wave2HostPortError::new(
                    "RESOURCE_NOT_FOUND",
                    format!("vcs.stage target is unavailable: {error}"),
                )
            })?;
            if !canonical_target.starts_with(&canonical_workspace) {
                return Err(Wave2HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!("vcs.stage path '{path}' escapes the workspace"),
                ));
            }
        }
        let _write_guard = self.workspace_write_lock.lock().await;
        let workspace = scope.workspace_root().to_path_buf();
        let path_label = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let repo_path = join_repo_path(
                &workspace_prefix,
                &git_path_to_string(&relative)?,
            );
            let mut index = repository.index().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.stage could not open the Git index: {error}"),
                )
            })?;
            if target_exists && target_is_dir {
                let mut stage_paths = Vec::new();
                collect_directory_stage_paths(
                    &resolved,
                    &repo_path,
                    &mut stage_paths,
                )?;
                for indexed_path in indexed_paths_for_target(&index, &repo_path)? {
                    index.remove_path(&indexed_path).map_err(|error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!(
                                "vcs.stage could not refresh {}: {error}",
                                path_label
                            ),
                        )
                    })?;
                }
                for stage_path in stage_paths {
                    index.add_path(&stage_path).map_err(|error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!(
                                "vcs.stage could not stage {}: {error}",
                                stage_path.display()
                            ),
                        )
                    })?;
                }
            } else if target_exists {
                index.add_path(Path::new(&repo_path)).map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "vcs.stage could not stage {}: {error}",
                            path_label
                        ),
                    )
                })?;
            } else {
                let indexed_paths = indexed_paths_for_target(&index, &repo_path)?;
                if indexed_paths.is_empty() {
                    return Err(Wave2HostPortError::new(
                        "RESOURCE_NOT_FOUND",
                        format!("vcs.stage path '{}' is not tracked", path_label),
                    ));
                }
                for indexed_path in indexed_paths {
                    index.remove_path(&indexed_path).map_err(|error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!(
                                "vcs.stage could not stage deletion {}: {error}",
                                indexed_path.display()
                            ),
                        )
                    })?;
                }
            }
            index.write().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.stage could not persist the Git index: {error}"),
                )
            })?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "path": path_label,
                "staged": true
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} stage worker failed: {error}"),
            )
        })?
    }

    async fn invoke_vcs_commit(
        &self,
        scope: &AgentSessionWorkspaceBinding,
        capability_id: &str,
        message: &str,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        let message = message.trim();
        if message.is_empty() {
            return Err(Wave2HostPortError::invalid_payload(
                "vcs.commit message must not be empty",
            ));
        }
        if message.chars().count() > 512 {
            return Err(Wave2HostPortError::invalid_payload(
                "vcs.commit message must not exceed 512 characters",
            ));
        }
        let workspace = scope.workspace_root().to_path_buf();
        let capability_id = capability_id.to_owned();
        let worker_capability_id = capability_id.clone();
        let message = message.to_owned();
        tokio::task::spawn_blocking(move || {
            let (repository, workspace_prefix) = scoped_repository(&workspace)?;
            let mut index = repository.index().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.commit could not open the Git index: {error}"),
                )
            })?;

            let parent = match repository.head() {
                Ok(head) if head.target().is_none() => {
                    if repository.is_empty().map_err(|error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!("vcs.commit could not inspect repository emptiness: {error}"),
                        )
                    })? {
                        None
                    } else {
                        return Err(Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            "vcs.commit found an unborn HEAD in a non-empty repository",
                        ));
                    }
                }
                Ok(head) => Some(head.peel_to_commit().map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("vcs.commit could not peel the repository HEAD: {error}"),
                    )
                })?),
                Err(error)
                    if matches!(
                        error.code(),
                        git2::ErrorCode::UnbornBranch | git2::ErrorCode::NotFound
                    ) && repository.is_empty().map_err(|inspect_error| {
                        Wave2HostPortError::new(
                            "CAPABILITY_UNAVAILABLE",
                            format!(
                                "vcs.commit could not inspect repository emptiness: {inspect_error}"
                            ),
                        )
                    })? =>
                {
                    None
                }
                Err(error) => {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("vcs.commit could not read the repository HEAD: {error}"),
                    ));
                }
            };
            let parent_tree = parent.as_ref().map(|commit| commit.tree()).transpose().map_err(
                |error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("vcs.commit could not load the parent tree: {error}"),
                    )
                },
            )?;
            let staged_diff = repository
                .diff_tree_to_index(parent_tree.as_ref(), Some(&index), None)
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("vcs.commit could not inspect staged changes: {error}"),
                    )
                })?;
            let mut scoped_paths = Vec::new();
            for delta in staged_diff.deltas() {
                let old_path = delta.old_file().path().map(git_path_to_string).transpose()?;
                let new_path = delta.new_file().path().map(git_path_to_string).transpose()?;
                let paths = [old_path.as_deref(), new_path.as_deref()];
                if paths.iter().all(Option::is_none) {
                    return Err(Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        "vcs.commit encountered a staged change without a path",
                    ));
                }
                for path in paths.into_iter().flatten() {
                    let Some(relative) = path_relative_to_workspace(path, &workspace_prefix) else {
                        return Err(Wave2HostPortError::new(
                            "PRESET_RESOURCE_NOT_BOUND",
                            "vcs.commit refuses to commit staged paths outside the bound workspace",
                        ));
                    };
                    scoped_paths.push(relative);
                }
            }
            if scoped_paths.is_empty() {
                return Err(Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    "vcs.commit has no staged changes in the bound workspace",
                ));
            }
            scoped_paths.sort();
            scoped_paths.dedup();

            let tree_id = index.write_tree().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.commit could not write the Git tree: {error}"),
                )
            })?;
            let tree = repository.find_tree(tree_id).map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("vcs.commit could not load the Git tree: {error}"),
                )
            })?;
            let signature = repository.signature().map_err(|error| {
                Wave2HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "vcs.commit requires configured Git user.name/user.email: {error}"
                    ),
                )
            })?;
            let parents = parent.iter().collect::<Vec<_>>();
            let commit_id = repository
                .commit(
                    Some("HEAD"),
                    &signature,
                    &signature,
                    &message,
                    &tree,
                    &parents,
                )
                .map_err(|error| {
                    Wave2HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "{worker_capability_id} could not create the commit: {error}"
                        ),
                    )
                })?;
            Ok::<_, Wave2HostPortError>(StrictJsonValue(json!({
                "committed": true,
                "commit_id": commit_id.to_string(),
                "message": message,
                "paths": scoped_paths
            })))
        })
        .await
        .map_err(|error| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!("{capability_id} commit worker failed: {error}"),
            )
        })?
    }
}

fn append_diff_patch(
    diff: &git2::Diff<'_>,
    patch: &mut String,
    truncated: &mut bool,
    capability_id: &str,
) -> Result<(), Wave2HostPortError> {
    if *truncated {
        return Ok(());
    }
    diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
        if line.origin() != '\0' {
            patch.push(line.origin());
        }
        patch.push_str(&String::from_utf8_lossy(line.content()));
        if patch.len() > MAX_DIFF_BYTES {
            let mut end = MAX_DIFF_BYTES;
            while end > 0 && !patch.is_char_boundary(end) {
                end -= 1;
            }
            patch.truncate(end);
            *truncated = true;
            false
        } else {
            true
        }
    })
    .map_err(|error| {
        Wave2HostPortError::new(
            "CAPABILITY_UNAVAILABLE",
            format!("{capability_id} could not render Git diff: {error}"),
        )
    })
}

fn repo_path_component_matches(left: &str, right: &str) -> bool {
    #[cfg(windows)]
    {
        left.eq_ignore_ascii_case(right)
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn path_relative_to_workspace(path: &str, prefix: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    let prefix = prefix.replace('\\', "/");
    if prefix.is_empty() {
        return Some(path);
    }
    let mut path_components = path.split('/');
    for expected in prefix.split('/') {
        let actual = path_components.next()?;
        if !repo_path_component_matches(actual, expected) {
            return None;
        }
    }
    Some(path_components.collect::<Vec<_>>().join("/"))
}

fn indexed_paths_for_target(
    index: &git2::Index,
    repo_path: &str,
) -> Result<Vec<PathBuf>, Wave2HostPortError> {
    let mut paths = Vec::new();
    for entry in index.iter() {
        let candidate = std::str::from_utf8(&entry.path).map_err(|_| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                "vcs.stage encountered a non-UTF-8 Git index path",
            )
        })?;
        if path_relative_to_workspace(candidate, repo_path).is_some() {
            paths.push(PathBuf::from(candidate));
        }
    }
    Ok(paths)
}

fn collect_directory_stage_paths(
    directory: &Path,
    repo_path: &str,
    output: &mut Vec<PathBuf>,
) -> Result<(), Wave2HostPortError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| {
            Wave2HostPortError::new(
                "RESOURCE_NOT_FOUND",
                format!(
                    "vcs.stage could not read directory '{}': {error}",
                    directory.display()
                ),
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            Wave2HostPortError::new(
                "RESOURCE_NOT_FOUND",
                format!(
                    "vcs.stage could not enumerate directory '{}': {error}",
                    directory.display()
                ),
            )
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if output.len() == MAX_VCS_STAGE_ENTRIES {
            return Err(Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                format!(
                    "vcs.stage directory exceeds {MAX_VCS_STAGE_ENTRIES} entries"
                ),
            ));
        }
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            Wave2HostPortError::new(
                "RESOURCE_NOT_FOUND",
                format!(
                    "vcs.stage could not inspect '{}': {error}",
                    entry.path().display()
                ),
            )
        })?;
        let entry_repo_path = join_repo_path(
            repo_path,
            &entry.file_name().to_string_lossy().replace('\\', "/"),
        );
        if metadata_is_windows_reparse_point(&metadata) {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "vcs.stage refuses Windows reparse entry '{}'",
                    entry.path().display()
                ),
            ));
        }
        if metadata.is_dir() {
            collect_directory_stage_paths(
                &entry.path(),
                &entry_repo_path,
                output,
            )?;
        } else {
            output.push(PathBuf::from(entry_repo_path));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_windows_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_windows_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

/// Open the repository containing the bound workspace and return the
/// repository-relative prefix of that workspace. Git status/index APIs operate
/// from the repository root, so every result and mutation must be projected
/// back into the exact typed workspace scope.
fn scoped_repository(
    workspace: &Path,
) -> Result<(git2::Repository, String), Wave2HostPortError> {
    let repository = git2::Repository::discover(workspace).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("workspace is not a Git repository: {error}"),
        )
    })?;
    let repository_root = repository.workdir().ok_or_else(|| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            "Git repository has no working directory",
        )
    })?;
    let repository_root = std::fs::canonicalize(repository_root).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("Git repository working directory is unavailable: {error}"),
        )
    })?;
    let workspace = std::fs::canonicalize(workspace).map_err(|error| {
        Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("workspace is unavailable: {error}"),
        )
    })?;
    let workspace_relative = workspace.strip_prefix(&repository_root).map_err(|_| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "workspace is outside the discovered Git repository",
        )
    })?;
    let prefix = git_path_to_string(workspace_relative)?
        .trim_matches('/')
        .to_owned();
    Ok((repository, prefix))
}

fn git_path_to_string(path: &Path) -> Result<String, Wave2HostPortError> {
    path.to_str()
        .map(|path| normalize_git_path(path.to_owned()))
        .ok_or_else(|| {
            Wave2HostPortError::new(
                "CAPABILITY_UNAVAILABLE",
                "Git path is not valid UTF-8 and cannot be projected safely",
            )
        })
}

fn normalize_git_path(path: String) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path
    }
}

fn join_repo_path(prefix: &str, relative: &str) -> String {
    let relative = relative.trim_matches('/');
    if prefix.is_empty() {
        relative.to_owned()
    } else if relative.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}/{relative}")
    }
}

fn git_status_name(status: git2::Status) -> Vec<&'static str> {
    let mut names = Vec::new();
    for (flag, name) in [
        (git2::Status::INDEX_NEW, "index_new"),
        (git2::Status::INDEX_MODIFIED, "index_modified"),
        (git2::Status::INDEX_DELETED, "index_deleted"),
        (git2::Status::INDEX_RENAMED, "index_renamed"),
        (git2::Status::INDEX_TYPECHANGE, "index_typechange"),
        (git2::Status::WT_NEW, "worktree_new"),
        (git2::Status::WT_MODIFIED, "worktree_modified"),
        (git2::Status::WT_DELETED, "worktree_deleted"),
        (git2::Status::WT_RENAMED, "worktree_renamed"),
        (git2::Status::WT_TYPECHANGE, "worktree_typechange"),
        (git2::Status::CONFLICTED, "conflicted"),
        (git2::Status::IGNORED, "ignored"),
    ] {
        if status.contains(flag) {
            names.push(name);
        }
    }
    names
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PathParams {
    path: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WriteParams {
    path: String,
    content: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchParams {
    query: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VcsPathParams {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct VcsCommitParams {
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SnapshotParams {
    operation: SnapshotOperation,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SnapshotOperation {
    Init,
    Compare,
    Baseline,
    Dispose,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessExecParams {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: HashMap<String, String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Default)]
struct ProcessOutputCapture {
    bytes: Vec<u8>,
    total_bytes: usize,
}

struct FinishedProcessOutput {
    text: String,
    truncated: bool,
}

fn decode<T: for<'de> Deserialize<'de>>(
    input: StrictJsonValue,
) -> Result<T, Wave2HostPortError> {
    serde_json::from_value(input.0).map_err(|error| {
        Wave2HostPortError::new(
            "INVALID_PAYLOAD",
            format!("Wave 2 filesystem input is invalid: {error}"),
        )
    })
}

fn snapshot_session_key(
    context: &Wave2HostContext,
    scope: &AgentSessionWorkspaceBinding,
) -> (String, String) {
    (
        context.agent_session_id.as_ref().to_owned(),
        scope.workspace_root().to_string_lossy().into_owned(),
    )
}

fn snapshot_info_value(info: SnapshotInfo) -> Value {
    let (mode, reason) = match info.mode {
        SnapshotMode::GitRepo => ("git-repo", None),
        SnapshotMode::Snapshot => ("snapshot", None),
        SnapshotMode::Disabled { reason } => ("disabled", Some(reason)),
    };
    json!({
        "mode": mode,
        "branch": info.branch,
        "reason": reason,
    })
}

fn snapshot_compare_value(
    compare: nomifun_file::CompareResult,
) -> Result<Value, Wave2HostPortError> {
    if compare.staged.len() > MAX_SNAPSHOT_CHANGES
        || compare.unstaged.len() > MAX_SNAPSHOT_CHANGES
    {
        return Err(Wave2HostPortError::new(
            "CAPABILITY_UNAVAILABLE",
            format!(
                "fs.snapshot compare exceeds the {MAX_SNAPSHOT_CHANGES}-entry result limit"
            ),
        ));
    }
    let convert = |changes: Vec<nomifun_file::FileChangeInfo>| {
        changes
            .into_iter()
            .map(|change| {
                json!({
                    "relative_path": change.relative_path.replace('\\', "/"),
                    "operation": change.operation,
                })
            })
            .collect::<Vec<_>>()
    };
    Ok(json!({
        "staged": convert(compare.staged),
        "unstaged": convert(compare.unstaged),
    }))
}

fn validate_process_exec_params(
    params: &ProcessExecParams,
) -> Result<(), Wave2HostPortError> {
    if params.command.trim().is_empty()
        || params.command.trim() != params.command
        || params.command.contains('\0')
    {
        return Err(Wave2HostPortError::invalid_payload(
            "process.exec command must be a non-empty executable without edge whitespace or NUL bytes",
        ));
    }
    if params.command.chars().count() > MAX_PROCESS_COMMAND_CHARS {
        return Err(Wave2HostPortError::invalid_payload(format!(
            "process.exec command must not exceed {MAX_PROCESS_COMMAND_CHARS} characters"
        )));
    }
    if params.args.len() > MAX_PROCESS_ARGUMENTS {
        return Err(Wave2HostPortError::invalid_payload(format!(
            "process.exec args must not contain more than {MAX_PROCESS_ARGUMENTS} entries"
        )));
    }
    if params.args.iter().any(|argument| {
        argument.contains('\0')
            || argument.chars().count() > MAX_PROCESS_ARGUMENT_CHARS
    }) {
        return Err(Wave2HostPortError::invalid_payload(format!(
            "process.exec arguments must not contain NUL bytes or exceed \
             {MAX_PROCESS_ARGUMENT_CHARS} characters"
        )));
    }
    if params.env.len() > MAX_PROCESS_ENVIRONMENT_ENTRIES {
        return Err(Wave2HostPortError::invalid_payload(format!(
            "process.exec env must not contain more than \
             {MAX_PROCESS_ENVIRONMENT_ENTRIES} entries"
        )));
    }
    if params.env.iter().any(|(key, value)| {
        key.is_empty()
            || key.contains(['=', '\0'])
            || value.contains('\0')
    }) {
        return Err(Wave2HostPortError::invalid_payload(
            "process.exec env contains an invalid key or NUL byte",
        ));
    }
    let timeout_ms = params.timeout_ms.unwrap_or(DEFAULT_PROCESS_TIMEOUT_MS);
    if !(1..=MAX_PROCESS_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(Wave2HostPortError::invalid_payload(format!(
            "process.exec timeout_ms must be between 1 and {MAX_PROCESS_TIMEOUT_MS}"
        )));
    }
    Ok(())
}

fn resolve_process_cwd(
    configured_root: &Path,
    requested_cwd: Option<&str>,
) -> Result<(PathBuf, String), Wave2HostPortError> {
    let configured_root = std::fs::canonicalize(configured_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "configured process workspace root '{}' is unavailable: {error}",
                configured_root.display()
            ),
        )
    })?;
    let requested_cwd = requested_cwd.unwrap_or("").trim();
    if requested_cwd.is_empty() {
        return Ok((configured_root, ".".to_owned()));
    }
    let relative = Path::new(requested_cwd);
    if relative.is_absolute()
        || requested_cwd.starts_with('/')
        || requested_cwd.starts_with('\\')
        || requested_cwd.contains('\\')
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Wave2HostPortError::invalid_payload(
            "process.exec cwd must be a normalized workspace-relative path",
        ));
    }
    let resolved = std::fs::canonicalize(configured_root.join(relative))
        .map_err(|error| {
            Wave2HostPortError::new(
                "RESOURCE_NOT_FOUND",
                format!("process.exec cwd '{requested_cwd}' is unavailable: {error}"),
            )
        })?;
    if !resolved.is_dir() {
        return Err(Wave2HostPortError::new(
            "RESOURCE_NOT_FOUND",
            format!("process.exec cwd '{requested_cwd}' is not a directory"),
        ));
    }
    if !resolved.starts_with(&configured_root) {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!("process.exec cwd '{requested_cwd}' escapes the configured workspace"),
        ));
    }
    let label = resolved
        .strip_prefix(&configured_root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    Ok((
        resolved,
        if label.is_empty() {
            ".".to_owned()
        } else {
            label
        },
    ))
}

fn append_process_output(
    capture: &Arc<Mutex<ProcessOutputCapture>>,
    chunk: &[u8],
) {
    let mut capture = capture
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    capture.total_bytes = capture.total_bytes.saturating_add(chunk.len());
    if chunk.len() >= MAX_PROCESS_OUTPUT_BYTES {
        capture.bytes.clear();
        capture
            .bytes
            .extend_from_slice(&chunk[chunk.len() - MAX_PROCESS_OUTPUT_BYTES..]);
        return;
    }
    let required = capture
        .bytes
        .len()
        .saturating_add(chunk.len())
        .saturating_sub(MAX_PROCESS_OUTPUT_BYTES);
    if required > 0 {
        capture.bytes.drain(..required);
    }
    capture.bytes.extend_from_slice(chunk);
}

fn finish_process_output(
    capture: &Arc<Mutex<ProcessOutputCapture>>,
) -> FinishedProcessOutput {
    let mut capture = capture
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let bytes = std::mem::take(&mut capture.bytes);
    FinishedProcessOutput {
        truncated: capture.total_bytes > bytes.len(),
        text: nomifun_terminal::strip_ansi(&bytes),
    }
}

impl Wave2ApplicationHost {
    fn process_binding<'a>(
        &self,
        context: &'a Wave2HostContext,
    ) -> Result<&'a TypedResourceBinding, Wave2HostPortError> {
        let mut bindings = context.resource_bindings.iter().filter(|binding| {
            binding.resource_kind.as_ref() == PROCESS_SESSION_RESOURCE_KIND
        });
        let binding = bindings.next().ok_or_else(|| {
            Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "process.exec requires one process_session resource binding",
            )
        })?;
        if bindings.next().is_some() {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "process.exec received more than one process_session resource binding",
            ));
        }
        if binding.owner_id != context.principal.principal_id {
            return Err(Wave2HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                format!(
                    "process_session binding {} belongs to a different principal",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        if !binding.operations.contains(PROCESS_EXECUTE_OPERATION) {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "process_session binding {} does not allow execute",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        Ok(binding)
    }

    fn workspace_scope(
        &self,
        context: &Wave2HostContext,
    ) -> Result<AgentSessionWorkspaceBinding, Wave2HostPortError> {
        if context
            .resource_bindings
            .iter()
            .any(|binding| binding.resource_kind.as_ref() != WORKSPACE_RESOURCE_KIND)
        {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action received a non-workspace resource binding",
            ));
        }
        let mut bindings = context
            .resource_bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == WORKSPACE_RESOURCE_KIND);
        let binding = bindings.next().ok_or_else(|| {
            Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action requires one workspace resource binding",
            )
        })?;
        if bindings.next().is_some() {
            return Err(Wave2HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "Wave 2 filesystem action received more than one workspace resource binding",
            ));
        }
        if binding.owner_id != context.principal.principal_id {
            return Err(Wave2HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                format!(
                    "workspace binding {} belongs to a different principal",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        let requested_root = binding
            .typed_parameters
            .get(WORKSPACE_ROOT_PARAMETER)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Wave2HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "workspace binding {} has no host-resolved {} parameter",
                        binding.binding_id.as_ref(),
                        WORKSPACE_ROOT_PARAMETER
                    ),
                )
            })?;
        let workspace_root = resolve_allowed_workspace_root(
            &self.configured_workspace_root,
            requested_root,
        )?;
        AgentSessionWorkspaceBinding::new(
            context.agent_session_id.as_ref(),
            binding_dto(binding),
            workspace_root,
        )
        .map_err(|error| operation_error(context.capability_id.as_ref(), error))
    }
}

fn resolve_allowed_workspace_root(
    configured_root: &Path,
    requested_root: &str,
) -> Result<PathBuf, Wave2HostPortError> {
    let configured_root = std::fs::canonicalize(configured_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "configured host workspace root '{}' is unavailable: {error}",
                configured_root.display()
            ),
        )
    })?;
    let requested_root = PathBuf::from(requested_root.trim());
    if !requested_root.is_absolute() {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "workspace_root must be an absolute host-resolved path",
        ));
    }
    let requested_root = std::fs::canonicalize(&requested_root).map_err(|error| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!("workspace_root is unavailable: {error}"),
        )
    })?;
    if !requested_root.starts_with(&configured_root) {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "workspace_root '{}' is outside the configured host workspace root '{}'",
                requested_root.display(),
                configured_root.display()
            ),
        ));
    }
    Ok(requested_root)
}

fn binding_dto(binding: &TypedResourceBinding) -> TypedResourceBindingDto {
    TypedResourceBindingDto {
        binding_id: binding.binding_id.as_ref().to_owned(),
        resource_kind: binding.resource_kind.as_ref().to_owned(),
        resource_id: binding.resource_id.as_ref().to_owned(),
        owner_id: binding.owner_id.clone(),
        operations: binding.operations.clone(),
        connection_config_ref: binding
            .connection_config_ref
            .as_ref()
            .map(|reference| reference.as_ref().to_owned()),
        typed_parameters: binding.typed_parameters.clone(),
    }
}

fn workspace_typed_binding<'a>(
    context: &'a Wave2HostContext,
) -> Result<&'a TypedResourceBinding, Wave2HostPortError> {
    let mut bindings = context
        .resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == WORKSPACE_RESOURCE_KIND);
    let binding = bindings.next().ok_or_else(|| {
        Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Wave 2 effect action requires one workspace resource binding",
        )
    })?;
    if bindings.next().is_some() {
        return Err(Wave2HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            "Wave 2 effect action received more than one workspace resource binding",
        ));
    }
    Ok(binding)
}

fn operation_error(capability_id: &str, error: AppError) -> Wave2HostPortError {
    let code = match error {
        AppError::BadRequest(_) => "INVALID_PAYLOAD",
        AppError::Forbidden(_) => "PRESET_RESOURCE_NOT_BOUND",
        AppError::NotFound(_) => "RESOURCE_NOT_FOUND",
        AppError::Conflict(_) | AppError::RevisionConflict(_) => "CAPABILITY_UNAVAILABLE",
        _ => "CAPABILITY_UNAVAILABLE_ON_PLATFORM",
    };
    Wave2HostPortError::new(code, format!("{capability_id} failed: {error}"))
}

fn unavailable(capability_id: &str) -> Wave2HostPortError {
    Wave2HostPortError::unavailable(format!(
        "no canonical application owner is wired for {capability_id}"
    ))
}

struct NullUserEvents;

impl UserEventSink for NullUserEvents {
    fn send_to_user(&self, _user_id: &str, _event: WebSocketMessage<Value>) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Mutex, OnceLock};
    use std::task::{Context, Poll, Waker};

    use nomifun_agent_contracts::{
        ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload,
        AgentSessionId, CapabilityExposure, CapabilityId, CapabilityRef, CapabilitySelection,
        CorrelationId, DigestHex, IdempotencyKey, OperationId, PresetRevisionRef,
        PrincipalRef, ResolvedSnapshotRef, ResourceBindingId, ResourceId, ResourceKind,
        RuntimeProfileKind, RuntimeTarget, ScopeKey, StrictJsonValue, TypedResourceBinding,
        UserId, VersionString,
    };
    use nomifun_agent_kernel::{
        AgentPresetCompiler, CapabilityInvocationRequest, CompileRequest,
        CompilerEnvironment, InMemoryPluginStatePersistence, KernelRegistry,
        MaterializationPolicy, SessionCapabilityState,
    };

    struct StateCaptureHostPort {
        captured: Arc<Mutex<Option<Wave2StateHandle>>>,
    }

    impl Wave2HostPort for StateCaptureHostPort {
        fn invoke<'a>(
            &'a self,
            request: Wave2HostRequest,
        ) -> Pin<Box<dyn Future<Output = Result<StrictJsonValue, Wave2HostPortError>> + Send + 'a>>
        {
            let captured = Arc::clone(&self.captured);
            let state = request.context.state;
            Box::pin(std::future::ready({
                *captured.lock().expect("state capture mutex") = Some(state);
                Ok(StrictJsonValue(json!({})))
            }))
        }
    }

    fn poll_ready<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("state capture future must complete synchronously"),
        }
    }

    fn test_state_handle() -> Wave2StateHandle {
        static HANDLE: OnceLock<Wave2StateHandle> = OnceLock::new();
        HANDLE.get_or_init(capture_state_handle).clone()
    }

    fn capture_state_handle() -> Wave2StateHandle {
        let captured = Arc::new(Mutex::new(None));
        let registry = KernelRegistry::new(
            MaterializationPolicy::stable(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            Arc::new(InMemoryPluginStatePersistence::new()),
        )
        .expect("kernel registry");
        let materialized = registry
            .replace_all(
                nomifun_agent_domain_wave2::registrations_with_host_port(Arc::new(
                    StateCaptureHostPort {
                        captured: Arc::clone(&captured),
                    },
                ))
                .expect("Wave 2 registrations"),
            )
            .expect("publish Wave 2 registrations");
        let principal = PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave2-host-owner".to_owned(),
        };
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from("wave2-host-workspace"),
            resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
            resource_id: ResourceId::from("wave2-host-resource"),
            owner_id: principal.principal_id.clone(),
            operations: BTreeSet::from([WORKSPACE_READ_OPERATION.to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let action = ActionId::from("fs.read.invoke");
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities: vec![CapabilitySelection {
                capability: CapabilityRef {
                    id: CapabilityId::from("fs.read"),
                    version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([action.clone()]),
                resource_binding_refs: vec![binding.binding_id.clone()],
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: StrictJsonValue(json!({})),
            }],
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: vec![binding],
            persona: "Wave 2 host test".to_owned(),
            instructions: "Invoke the selected capability.".to_owned(),
            context_policy: StrictJsonValue(json!({})),
            execution_constraints: StrictJsonValue(json!({})),
            runtime_budget: StrictJsonValue(json!({})),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from("wave2-host-test"),
                revision: 1,
                revision_digest: nomifun_agent_contracts::digest_payload(&payload)
                    .expect("revision digest"),
            },
            payload,
            created_by: UserId::from(principal.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = AgentPresetCompiler::compile(
            &materialized,
            &CompilerEnvironment {
                resolver_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(nomifun_agent_domain_wave2::CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("test-target"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "wave2-host-test".to_owned(),
            },
            CompileRequest {
                revision,
                principal: principal.clone(),
                scene: "wave2-host-test".to_owned(),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from("wave2-host-resolve"),
            },
        )
        .expect("compile selected capability");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        poll_ready(registry.invoke(
            &snapshot,
            &active,
            CapabilityInvocationRequest {
                principal: principal.clone(),
                session_owner: principal,
                agent_session_id: AgentSessionId::from("wave2-host-session"),
                operation_id: OperationId::from("wave2-host-operation"),
                idempotency_key: IdempotencyKey::from("wave2-host-idempotency"),
                correlation_id: CorrelationId::from("wave2-host-correlation"),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("fs.read"),
                action_id: action,
                resource_binding_ids: BTreeSet::from([ResourceBindingId::from(
                    "wave2-host-workspace",
                )]),
                state_scope_key: ScopeKey::from("session:wave2-host"),
                input: StrictJsonValue(json!({})),
            },
        ))
        .expect("state projection invocation");
        captured
            .lock()
            .expect("state capture mutex")
            .take()
            .expect("host received the state handle")
    }

    fn context(root: &std::path::Path) -> Wave2HostContext {
        Wave2HostContext {
            principal: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "owner-1".to_owned(),
            },
            agent_session_id: AgentSessionId::from(nomifun_common::generate_id()),
            operation_id: OperationId::from("operation-1"),
            idempotency_key: IdempotencyKey::from("idempotency-1"),
            correlation_id: CorrelationId::from("correlation-1"),
            resolved_snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: "snapshot-1".into(),
                snapshot_digest: "a".repeat(64).into(),
            },
            registry_generation: 1,
            capability_id: CapabilityId::from("fs.write"),
            action_id: ActionId::from("fs.write.invoke"),
            state: test_state_handle(),
            resource_bindings: vec![TypedResourceBinding {
                binding_id: ResourceBindingId::from("workspace-binding"),
                resource_kind: ResourceKind::from(WORKSPACE_RESOURCE_KIND),
                resource_id: ResourceId::from(format!(
                    "workspace-resource:{}",
                    root.to_string_lossy()
                )),
                owner_id: "owner-1".to_owned(),
                operations: BTreeSet::from([
                    "read".to_owned(),
                    "write".to_owned(),
                    "delete".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::from([(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    root.to_string_lossy().into_owned(),
                )]),
            }],
        }
    }

    fn process_context(root: &std::path::Path) -> Wave2HostContext {
        let mut context = context(root);
        context.resource_bindings = vec![TypedResourceBinding {
            binding_id: ResourceBindingId::from("process-session-binding"),
            resource_kind: ResourceKind::from(PROCESS_SESSION_RESOURCE_KIND),
            resource_id: ResourceId::from("process-session-resource"),
            owner_id: "owner-1".to_owned(),
            operations: BTreeSet::from([PROCESS_EXECUTE_OPERATION.to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([(
                WORKSPACE_ROOT_PARAMETER.to_owned(),
                root.to_string_lossy().into_owned(),
            )]),
        }];
        context
    }

    const PROCESS_TREE_FIXTURE_MODE_ENV: &str =
        "NOMIFUN_WAVE2_PROCESS_TREE_FIXTURE_MODE";
    const PROCESS_TREE_FIXTURE_ROOT_ENV: &str =
        "NOMIFUN_WAVE2_PROCESS_TREE_FIXTURE_ROOT";
    const PROCESS_TREE_FIXTURE_TEST: &str =
        "router::agent_wave2_host::tests::managed_process_tree_fixture";

    fn process_tree_request(
        root: &Path,
        timeout_ms: u64,
    ) -> Value {
        let environment = HashMap::from([
            (
                PROCESS_TREE_FIXTURE_MODE_ENV.to_owned(),
                "parent".to_owned(),
            ),
            (
                PROCESS_TREE_FIXTURE_ROOT_ENV.to_owned(),
                root.to_string_lossy().into_owned(),
            ),
        ]);
        json!({
            "command": std::env::current_exe().unwrap(),
            "args": [
                "--exact",
                PROCESS_TREE_FIXTURE_TEST,
                "--nocapture"
            ],
            "env": environment,
            "timeout_ms": timeout_ms
        })
    }

    async fn wait_for_fixture_file(path: &Path) -> bool {
        for _ in 0..60 {
            if path.exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        false
    }

    #[test]
    fn managed_process_tree_fixture() {
        let Ok(mode) = std::env::var(PROCESS_TREE_FIXTURE_MODE_ENV) else {
            return;
        };
        let root = PathBuf::from(
            std::env::var(PROCESS_TREE_FIXTURE_ROOT_ENV)
                .expect("process-tree fixture root"),
        );
        match mode.as_str() {
            "parent" => {
                let child = std::process::Command::new(
                    std::env::current_exe().expect("fixture executable"),
                )
                .args([
                    "--exact",
                    PROCESS_TREE_FIXTURE_TEST,
                    "--nocapture",
                ])
                .env(PROCESS_TREE_FIXTURE_MODE_ENV, "child")
                .env(PROCESS_TREE_FIXTURE_ROOT_ENV, &root)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .expect("spawn process-tree fixture child");
                std::fs::write(root.join("started.txt"), child.id().to_string())
                    .expect("write process-tree start marker");
                std::thread::sleep(Duration::from_secs(30));
            }
            "child" => {
                std::thread::sleep(Duration::from_secs(3));
                std::fs::write(root.join("survived.txt"), "survived")
                    .expect("write process-tree survival marker");
            }
            other => panic!("unknown process-tree fixture mode {other}"),
        }
    }

    async fn invoke(
        host: &Wave2ApplicationHost,
        mut context: Wave2HostContext,
        capability_id: &str,
        input: Value,
    ) -> Result<StrictJsonValue, Wave2HostPortError> {
        context.capability_id = CapabilityId::from(capability_id.to_owned());
        context.action_id = ActionId::from(format!("{capability_id}.invoke"));
        host.invoke(Wave2HostRequest {
            context,
            operation: Wave2CapabilityOperation::WorkspaceExecution {
                input: StrictJsonValue(input),
            },
        })
        .await
    }

    #[tokio::test]
    async fn workspace_file_actions_use_the_typed_binding_root() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let written = invoke(
            &host,
            context.clone(),
            "fs.write",
            json!({"path": "test.txt", "content": "hello"}),
        )
        .await
        .unwrap();
        assert_eq!(written.0["written"], true);

        let read = invoke(
            &host,
            context.clone(),
            "fs.read",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(read.0["content"], "hello");

        let deleted = invoke(
            &host,
            context,
            "fs.delete",
            json!({"path": "test.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(deleted.0["deleted"], true);
        assert!(!directory.path().join("test.txt").exists());
    }

    #[tokio::test]
    async fn workspace_patch_replaces_exact_content_and_rejects_stale_context() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("patch.txt"),
            "before\nold value\nafter\n",
        )
        .unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let patched = invoke(
            &host,
            context.clone(),
            "fs.patch",
            json!({
                "path": "patch.txt",
                "old_content": "old value",
                "new_content": "new value"
            }),
        )
        .await
        .unwrap();
        assert_eq!(patched.0["patched"], true);
        assert_eq!(patched.0["replacements"], 1);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("patch.txt")).unwrap(),
            "before\nnew value\nafter\n"
        );

        let stale = invoke(
            &host,
            context,
            "fs.patch",
            json!({
                "path": "patch.txt",
                "old_content": "old value",
                "new_content": "another value"
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(stale.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("patch.txt")).unwrap(),
            "before\nnew value\nafter\n"
        );
    }

    #[tokio::test]
    async fn concurrent_workspace_patches_do_not_overwrite_each_other() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("patch.txt"), "alpha beta\n")
            .unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let first = invoke(
            &host,
            context.clone(),
            "fs.patch",
            json!({
                "path": "patch.txt",
                "old_content": "alpha",
                "new_content": "ALPHA"
            }),
        );
        let second = invoke(
            &host,
            context,
            "fs.patch",
            json!({
                "path": "patch.txt",
                "old_content": "beta",
                "new_content": "BETA"
            }),
        );
        let (first, second) = tokio::join!(first, second);
        first.unwrap();
        second.unwrap();
        assert_eq!(
            std::fs::read_to_string(directory.path().join("patch.txt")).unwrap(),
            "ALPHA BETA\n"
        );
    }

    #[tokio::test]
    async fn workspace_snapshot_uses_the_existing_snapshot_owner() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "baseline\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let snapshot = invoke(&host, context, "fs.snapshot", json!({}))
            .await
            .unwrap();
        assert_eq!(snapshot.0["mode"], "snapshot");
        assert_eq!(snapshot.0["branch"], Value::Null);
        assert!(host.snapshots.is_tracked(&directory.path().to_string_lossy()));

        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let comparison = host
            .snapshots
            .compare(&directory.path().to_string_lossy())
            .await
            .unwrap();
        assert!(
            comparison
                .unstaged
                .iter()
                .any(|change| change.relative_path == "tracked.txt")
        );
        host.snapshots
            .dispose(&directory.path().to_string_lossy())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn process_exec_uses_the_managed_process_owner_and_confined_cwd() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("nested")).unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        #[cfg(windows)]
        let (command, args) = (
            std::env::var("ComSpec")
                .unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_owned()),
            vec![
                "/d".to_owned(),
                "/c".to_owned(),
                "echo executed>marker.txt".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (command, args) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "printf executed > marker.txt".to_owned(),
            ],
        );

        let result = invoke(
            &host,
            process_context(directory.path()),
            "process.exec",
            json!({
                "command": command,
                "args": args,
                "cwd": "nested",
                "timeout_ms": 10_000
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.0["success"], true);
        assert_eq!(result.0["exit_code"], 0);
        assert_eq!(result.0["cwd"], "nested");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("nested/marker.txt"))
                .unwrap()
                .trim(),
            "executed"
        );
    }

    #[tokio::test]
    async fn process_exec_rejects_workspace_escape_before_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let error = invoke(
            &host,
            process_context(directory.path()),
            "process.exec",
            json!({
                "command": "unused",
                "cwd": "../outside"
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "INVALID_PAYLOAD");
    }

    #[tokio::test]
    async fn process_exec_requires_a_host_resolved_process_root() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut context = process_context(directory.path());
        context.resource_bindings[0].typed_parameters.clear();
        let error = invoke(
            &host,
            context,
            "process.exec",
            json!({"command": "unused"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert!(error.message.contains(WORKSPACE_ROOT_PARAMETER));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn process_exec_rejects_junction_cwd_escape_on_windows() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        junction::create(
            outside.path(),
            directory.path().join("outside-junction"),
        )
        .unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let error = invoke(
            &host,
            process_context(directory.path()),
            "process.exec",
            json!({
                "command": "unused",
                "cwd": "outside-junction"
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn process_exec_timeout_reaps_the_managed_process_tree() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let error = invoke(
            &host,
            process_context(directory.path()),
            "process.exec",
            process_tree_request(directory.path(), 1_500),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
        assert!(error.message.contains("process tree was reaped"));
        assert!(directory.path().join("started.txt").exists());

        tokio::time::sleep(Duration::from_millis(3_500)).await;
        assert!(
            !directory.path().join("survived.txt").exists(),
            "a timed-out process.exec left its descendant alive"
        );
    }

    #[tokio::test]
    async fn cancelling_process_exec_keeps_cleanup_coordinator_alive() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let root = directory.path().to_path_buf();
        let task = tokio::spawn(async move {
            invoke(
                &host,
                process_context(&root),
                "process.exec",
                process_tree_request(&root, 1_500),
            )
            .await
        });
        assert!(
            wait_for_fixture_file(&directory.path().join("started.txt")).await,
            "process-tree fixture did not start"
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());

        tokio::time::sleep(Duration::from_millis(3_500)).await;
        assert!(
            !directory.path().join("survived.txt").exists(),
            "cancelling process.exec abandoned its descendant process"
        );
    }

    #[tokio::test]
    async fn missing_workspace_root_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut context = context(directory.path());
        context.resource_bindings[0].typed_parameters.clear();
        let error = invoke(&host, context, "fs.read", json!({"path": "x"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn host_rejects_action_identity_and_extra_resource_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut wrong_action = context(directory.path());
        wrong_action.capability_id = CapabilityId::from("fs.read");
        wrong_action.action_id = ActionId::from("fs.write.invoke");
        let error = host
            .invoke(Wave2HostRequest {
                context: wrong_action,
                operation: Wave2CapabilityOperation::WorkspaceExecution {
                    input: StrictJsonValue(json!({"path": "x.txt"})),
                },
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, "INVALID_PAYLOAD");

        let mut extra_binding = context(directory.path());
        extra_binding.resource_bindings.push(TypedResourceBinding {
            binding_id: ResourceBindingId::from("process-binding"),
            resource_kind: ResourceKind::from("process_session"),
            resource_id: ResourceId::from("process-resource"),
            owner_id: "owner-1".to_owned(),
            operations: BTreeSet::from(["execute".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        });
        let error = invoke(
            &host,
            extra_binding,
            "fs.read",
            json!({"path": "x.txt"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
    }

    #[tokio::test]
    async fn workspace_search_returns_real_content_matches() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("needle.txt"), "before\nneedle line\nafter\n")
            .unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let result = invoke(
            &host,
            context(directory.path()),
            "fs.search",
            json!({"query": "needle"}),
        )
        .await
        .unwrap();
        assert_eq!(result.0["matches"][0]["path"], "needle.txt");
        assert_eq!(result.0["matches"][0]["line"], 2);
        assert_eq!(result.0["truncated"], false);
    }

    #[tokio::test]
    async fn workspace_snapshot_is_session_scoped_and_returns_bounded_real_state() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let session_context = context(directory.path());

        let initialized = invoke(
            &host,
            session_context.clone(),
            "fs.snapshot",
            json!({"operation": "init"}),
        )
        .await
        .unwrap();
        assert_eq!(initialized.0["mode"], "git-repo");
        assert!(initialized.0["branch"].is_string());
        assert_eq!(host.snapshots.workspace_count(), 1);

        let initialized_again = invoke(
            &host,
            session_context.clone(),
            "fs.snapshot",
            json!({"operation": "init"}),
        )
        .await
        .unwrap();
        assert_eq!(initialized_again, initialized);
        assert_eq!(
            host.snapshots.workspace_count(),
            1,
            "same AgentSession init must not leak a second snapshot reference"
        );

        let other_session = context(directory.path());
        let other_session_error = invoke(
            &host,
            other_session.clone(),
            "fs.snapshot",
            json!({"operation": "compare"}),
        )
        .await
        .unwrap_err();
        assert_eq!(other_session_error.code, "CAPABILITY_UNAVAILABLE");

        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let compared = invoke(
            &host,
            session_context.clone(),
            "fs.snapshot",
            json!({"operation": "compare"}),
        )
        .await
        .unwrap();
        assert_eq!(compared.0["unstaged"][0]["relative_path"], "tracked.txt");
        assert_eq!(compared.0["unstaged"][0]["operation"], "modify");

        let baseline = invoke(
            &host,
            session_context.clone(),
            "fs.snapshot",
            json!({"operation": "baseline", "path": "tracked.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(baseline.0["found"], true);
        assert_eq!(baseline.0["content"], "base\n");
        assert_eq!(baseline.0["truncated"], false);

        let disposed = invoke(
            &host,
            session_context.clone(),
            "fs.snapshot",
            json!({"operation": "dispose"}),
        )
        .await
        .unwrap();
        assert_eq!(disposed.0["disposed"], true);
        assert_eq!(host.snapshots.workspace_count(), 0);

        let compare_after_dispose = invoke(
            &host,
            session_context,
            "fs.snapshot",
            json!({"operation": "compare"}),
        )
        .await
        .unwrap_err();
        assert_eq!(compare_after_dispose.code, "CAPABILITY_UNAVAILABLE");
        drop(repository);
    }

    #[tokio::test]
    async fn workspace_snapshot_uses_real_temporary_baseline_for_non_git_workspace() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("note.txt"), "initial\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        let initialized = invoke(
            &host,
            context.clone(),
            "fs.snapshot",
            json!({"operation": "init"}),
        )
        .await
        .unwrap();
        assert_eq!(initialized.0["mode"], "snapshot");
        let snapshot_repo = host.snapshots.repo_path_for(
            directory.path().to_str().unwrap(),
        ).unwrap();
        assert!(snapshot_repo.exists());

        std::fs::write(directory.path().join("note.txt"), "updated\n").unwrap();
        let compare = invoke(
            &host,
            context.clone(),
            "fs.snapshot",
            json!({"operation": "compare"}),
        )
        .await
        .unwrap();
        assert_eq!(compare.0["unstaged"][0]["relative_path"], "note.txt");
        let baseline = invoke(
            &host,
            context.clone(),
            "fs.snapshot",
            json!({"operation": "baseline", "path": "note.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(baseline.0["content"], "initial\n");

        invoke(
            &host,
            context,
            "fs.snapshot",
            json!({"operation": "dispose"}),
        )
        .await
        .unwrap();
        assert!(!snapshot_repo.exists());
    }

    #[tokio::test]
    async fn workspace_patch_uses_the_bound_file_owner_and_is_all_or_nothing() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("a.txt"), "alpha\n").unwrap();
        std::fs::write(directory.path().join("b.txt"), "bravo\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let patch = |path: &str, old: &str, new: &str| {
            json!({
                "path": path,
                "hunks": [{
                    "old_start": 1,
                    "old_lines": 1,
                    "new_start": 1,
                    "new_lines": 1,
                    "lines": [
                        {"kind": "remove", "text": old},
                        {"kind": "add", "text": new}
                    ]
                }]
            })
        };

        let result = invoke(
            &host,
            context(directory.path()),
            "fs.patch",
            json!({
                "files": [
                    patch("a.txt", "alpha", "ALPHA"),
                    patch("b.txt", "bravo", "BRAVO")
                ]
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.0["file_count"], 2);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("a.txt")).unwrap(),
            "ALPHA\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("b.txt")).unwrap(),
            "BRAVO\n"
        );

        let mut invalid_context = context(directory.path());
        invalid_context.idempotency_key = IdempotencyKey::from("patch-invalid-hunk-key");
        let error = invoke(
            &host,
            invalid_context,
            "fs.patch",
            json!({
                "files": [
                    patch("a.txt", "ALPHA", "again"),
                    patch("b.txt", "not-present", "must-not-write")
                ]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("a.txt")).unwrap(),
            "ALPHA\n"
        );
        assert_eq!(
            std::fs::read_to_string(directory.path().join("b.txt")).unwrap(),
            "BRAVO\n"
        );
    }

    #[tokio::test]
    async fn workspace_patch_rejects_traversal_and_read_only_bindings() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut read_only = context(directory.path());
        read_only.resource_bindings[0].operations =
            BTreeSet::from([WORKSPACE_READ_OPERATION.to_owned()]);

        let read_only_error = invoke(
            &host,
            read_only,
            "fs.patch",
            json!({
                "files": [{
                    "path": "new.txt",
                    "hunks": [{
                        "old_start": 0,
                        "old_lines": 0,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [{"kind": "add", "text": "nope"}]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(read_only_error.code, "PRESET_RESOURCE_NOT_BOUND");

        let mut traversal_context = context(directory.path());
        traversal_context.idempotency_key = IdempotencyKey::from("patch-traversal-key");
        let traversal_error = invoke(
            &host,
            traversal_context,
            "fs.patch",
            json!({
                "files": [{
                    "path": format!("../{}", outside.path().join("secret.txt").file_name().unwrap().to_string_lossy()),
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "secret"},
                            {"kind": "add", "text": "escaped"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(traversal_error.code, "INVALID_PAYLOAD");
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "secret\n"
        );
    }

    #[tokio::test]
    async fn effectful_workspace_actions_replay_and_conflict_by_idempotency_key() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("entry.txt"), "before\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut context = context(directory.path());
        context.idempotency_key = IdempotencyKey::from("patch-replay-key");
        let patch = json!({
            "files": [{
                "path": "entry.txt",
                "hunks": [{
                    "old_start": 1,
                    "old_lines": 1,
                    "new_start": 1,
                    "new_lines": 1,
                    "lines": [
                        {"kind": "remove", "text": "before"},
                        {"kind": "add", "text": "after"}
                    ]
                }]
            }]
        });
        let first = invoke(&host, context.clone(), "fs.patch", patch.clone())
            .await
            .unwrap();
        let replay = invoke(&host, context.clone(), "fs.patch", patch)
            .await
            .unwrap();
        assert_eq!(replay, first);
        assert_eq!(
            std::fs::read_to_string(directory.path().join("entry.txt")).unwrap(),
            "after\n"
        );

        let conflict = invoke(
            &host,
            context,
            "fs.patch",
            json!({
                "files": [{
                    "path": "entry.txt",
                    "hunks": [{
                        "old_start": 1,
                        "old_lines": 1,
                        "new_start": 1,
                        "new_lines": 1,
                        "lines": [
                            {"kind": "remove", "text": "after"},
                            {"kind": "add", "text": "different"}
                        ]
                    }]
                }]
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(conflict.code, "IDEMPOTENCY_CONFLICT");
    }

    fn initialize_git_repository(root: &Path) -> git2::Repository {
        let repository = git2::Repository::init(root).unwrap();
        std::fs::write(root.join("tracked.txt"), "base\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repository.find_tree(tree_id).unwrap();
        let signature = git2::Signature::now("NomiFun test", "test@nomifun.invalid").unwrap();
        repository
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "initial",
                &tree,
                &[],
            )
            .unwrap();
        drop(tree);
        repository
    }

    #[tokio::test]
    async fn vcs_status_diff_and_stage_use_the_bound_repository() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let base_context = context(directory.path());

        let status = invoke(&host, base_context.clone(), "vcs.status", json!({}))
            .await
            .unwrap();
        assert_eq!(status.0["entries"][0]["path"], "tracked.txt");
        assert!(
            status.0["entries"][0]["status"]
                .as_array()
                .is_some_and(|values| values.iter().any(|value| value == "worktree_modified"))
        );

        let diff = invoke(&host, base_context.clone(), "vcs.diff", json!({}))
            .await
            .unwrap();
        assert!(diff.0["patch"].as_str().unwrap().contains("changed"));

        let staged = invoke(
            &host,
            base_context.clone(),
            "vcs.stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        assert_eq!(staged.0["staged"], true);
        let status_after = repository.statuses(None).unwrap();
        assert!(
            status_after
                .iter()
                .any(|entry| entry.status().contains(git2::Status::INDEX_MODIFIED))
        );
        let staged_diff = invoke(&host, base_context, "vcs.diff", json!({}))
            .await
            .unwrap();
        assert!(
            staged_diff.0["staged_patch"]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        assert_eq!(staged_diff.0["unstaged_patch"], "");
    }

    #[tokio::test]
    async fn vcs_stage_recurses_directories_and_records_deletions() {
        let directory = tempfile::tempdir().unwrap();
        let _repository = initialize_git_repository(directory.path());
        let batch = directory.path().join("batch");
        std::fs::create_dir(&batch).unwrap();
        std::fs::write(batch.join("keep.txt"), "keep\n").unwrap();
        std::fs::write(batch.join("remove.txt"), "remove\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        invoke(
            &host,
            context.clone(),
            "vcs.stage",
            json!({"path": "batch"}),
        )
        .await
        .unwrap();
        let index = git2::Repository::open(directory.path())
            .unwrap()
            .index()
            .unwrap();
        assert!(index.get_path(Path::new("batch/keep.txt"), 0).is_some());
        assert!(index.get_path(Path::new("batch/remove.txt"), 0).is_some());
        drop(index);

        std::fs::remove_file(batch.join("remove.txt")).unwrap();
        invoke(&host, context, "vcs.stage", json!({"path": "batch"}))
            .await
            .unwrap();
        let index = git2::Repository::open(directory.path())
            .unwrap()
            .index()
            .unwrap();
        assert!(index.get_path(Path::new("batch/keep.txt"), 0).is_some());
        assert!(index.get_path(Path::new("batch/remove.txt"), 0).is_none());
    }

    #[test]
    fn vcs_workspace_prefix_projection_matches_host_path_semantics() {
        assert_eq!(
            path_relative_to_workspace("nested/file.txt", "nested"),
            Some("file.txt".to_owned())
        );
        #[cfg(windows)]
        assert_eq!(
            path_relative_to_workspace("Nested/File.txt", "nested"),
            Some("File.txt".to_owned())
        );
        #[cfg(not(windows))]
        assert_eq!(
            path_relative_to_workspace("Nested/File.txt", "nested"),
            None
        );
    }

    #[tokio::test]
    async fn vcs_operations_remain_confined_to_a_repository_subdirectory() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("inside.txt"), "inside\n").unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "root changed\n").unwrap();

        let host = Wave2ApplicationHost::for_workspace_root(&nested);
        let context = {
            let mut context = context(&nested);
            context.resource_bindings[0]
                .typed_parameters
                .insert(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    nested.to_string_lossy().into_owned(),
                );
            context
        };

        let status = invoke(&host, context.clone(), "vcs.status", json!({}))
            .await
            .unwrap();
        let status_paths = status.0["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["path"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(status_paths, vec!["inside.txt"]);

        let diff = invoke(&host, context.clone(), "vcs.diff", json!({}))
            .await
            .unwrap();
        assert!(!diff.0["patch"].as_str().unwrap().contains("root changed"));

        invoke(
            &host,
            context,
            "vcs.stage",
            json!({"path": "inside.txt"}),
        )
        .await
        .unwrap();
        let status_after = repository.statuses(None).unwrap();
        assert!(
            status_after.iter().any(|entry| {
                entry.path() == Some("nested/inside.txt")
                    && entry.status().contains(git2::Status::INDEX_NEW)
            })
        );
        assert!(
            status_after.iter().any(|entry| {
                entry.path() == Some("tracked.txt")
                    && entry.status().contains(git2::Status::WT_MODIFIED)
            })
        );
    }

    #[tokio::test]
    async fn vcs_commit_commits_only_staged_changes_in_the_bound_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "base\nchanged\n").unwrap();
        {
            let mut config = repository.config().unwrap();
            config.set_str("user.name", "NomiFun Test").unwrap();
            config.set_str("user.email", "nomifun-test@nomifun.invalid").unwrap();
        }
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());

        invoke(
            &host,
            context.clone(),
            "vcs.stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        let committed = invoke(
            &host,
            context.clone(),
            "vcs.commit",
            json!({"message": "record workspace change"}),
        )
        .await
        .unwrap();
        assert_eq!(committed.0["committed"], true);
        assert_eq!(committed.0["message"], "record workspace change");
        assert_eq!(
            committed.0["paths"],
            json!(["tracked.txt"])
        );

        let head = repository.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.message(), Some("record workspace change"));
        assert!(repository.statuses(None).unwrap().is_empty());

        let mut retry_context = context;
        retry_context.idempotency_key = IdempotencyKey::from("vcs-commit-empty-retry");
        let error = invoke(&host, retry_context, "vcs.commit", json!({"message": "empty"}))
            .await
            .unwrap_err();
        assert_eq!(error.code, "CAPABILITY_UNAVAILABLE");
    }

    #[tokio::test]
    async fn vcs_commit_replays_the_original_commit_result_for_the_same_key() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "new\n").unwrap();
        {
            let mut config = repository.config().unwrap();
            config.set_str("user.name", "NomiFun Test").unwrap();
            config.set_str("user.email", "nomifun-test@nomifun.invalid").unwrap();
        }
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let context = context(directory.path());
        invoke(
            &host,
            context.clone(),
            "vcs.stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap();
        let first = invoke(
            &host,
            context.clone(),
            "vcs.commit",
            json!({"message": "replayable commit"}),
        )
        .await
        .unwrap();
        let replay = invoke(
            &host,
            context,
            "vcs.commit",
            json!({"message": "replayable commit"}),
        )
        .await
        .unwrap();
        assert_eq!(replay, first);
        assert_eq!(repository.head().unwrap().target(), first.0["commit_id"].as_str().and_then(|id| git2::Oid::from_str(id).ok()));
    }

    #[tokio::test]
    async fn vcs_commit_rejects_staged_paths_outside_a_nested_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(directory.path().join("tracked.txt"), "root changed\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();

        let host = Wave2ApplicationHost::for_workspace_root(&nested);
        let context = {
            let mut context = context(&nested);
            context.resource_bindings[0]
                .typed_parameters
                .insert(
                    WORKSPACE_ROOT_PARAMETER.to_owned(),
                    nested.to_string_lossy().into_owned(),
                );
            context
        };
        let error = invoke(
            &host,
            context,
            "vcs.commit",
            json!({"message": "must stay scoped"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert!(repository.head().unwrap().peel_to_commit().unwrap().message() == Some("initial"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn vcs_commit_rejects_literal_backslash_paths_outside_a_nested_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let literal = directory.path().join("nested\\outside.txt");
        std::fs::write(&literal, "outside\n").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("nested\\outside.txt")).unwrap();
        index.write().unwrap();

        let host = Wave2ApplicationHost::for_workspace_root(&nested);
        let mut context = context(&nested);
        context.resource_bindings[0]
            .typed_parameters
            .insert(
                WORKSPACE_ROOT_PARAMETER.to_owned(),
                nested.to_string_lossy().into_owned(),
            );
        let error = invoke(
            &host,
            context,
            "vcs.commit",
            json!({"message": "reject literal separator"}),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("initial")
        );
    }

    #[tokio::test]
    async fn vcs_read_actions_do_not_allow_stage_or_commit_without_write_grant() {
        let directory = tempfile::tempdir().unwrap();
        let repository = initialize_git_repository(directory.path());
        std::fs::write(directory.path().join("tracked.txt"), "changed\n").unwrap();
        let host = Wave2ApplicationHost::for_workspace_root(directory.path());
        let mut read_only = context(directory.path());
        read_only.resource_bindings[0].operations =
            BTreeSet::from(["read".to_owned()]);

        let status = invoke(&host, read_only.clone(), "vcs.status", json!({}))
            .await
            .unwrap();
        assert_eq!(status.0["entries"][0]["path"], "tracked.txt");

        let stage_error = invoke(
            &host,
            read_only.clone(),
            "vcs.stage",
            json!({"path": "tracked.txt"}),
        )
        .await
        .unwrap_err();
        assert_eq!(stage_error.code, "PRESET_RESOURCE_NOT_BOUND");

        let commit_error = invoke(
            &host,
            read_only,
            "vcs.commit",
            json!({"message": "must be denied"}),
        )
        .await
        .unwrap_err();
        assert_eq!(commit_error.code, "PRESET_RESOURCE_NOT_BOUND");
        assert_eq!(
            repository.head().unwrap().peel_to_commit().unwrap().message(),
            Some("initial")
        );
    }
}
