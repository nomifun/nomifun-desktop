//! Live H1a evidence through the ordinary template, publication and Agent APIs.
//! No model interception, alternate endpoint, injected execution evidence or raw diagnostics.
use super::*;
use nomifun_db::sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use sha2::{Digest, Sha256};

const ALLOW_FILE: &str = "before-tool-allowed.txt";
const DENY_FILE: &str = ".env.hook-smoke";
const CONTENT: &str = "NOMIFUN_BEFORE_TOOL_SYNTHETIC_DATA\n";
const ALLOW_MARKER: &str = "NOMIFUN_BEFORE_TOOL_ALLOW_OK";
const DENY_MARKER: &str = "NOMIFUN_BEFORE_TOOL_DENY_CONSUMED";
// Exact ordinary-template output: receipt hashing proves the real Service decision.
const DENY_REASON: &str = "敏感文件检查已阻止此操作。请使用不含凭据的示例文件，或先调整 Agent 中已选择的检查规则。目标工具未执行。";
const TEMPLATE: &str = include_str!("../../src/router/plugin_product/templates/before-tool.mjs");

fn fail(phase: &'static str, code: &'static str) -> SmokeFailure {
    SmokeFailure::new(phase, code, 422)
}

pub(super) fn emit_stage_pass(phase: &str) -> Result<(), SmokeFailure> {
    let line = match phase {
        "before_tool.publish_select" => {
            "NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.publish_select status=pass"
        }
        "before_tool.allow" => "NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.allow status=pass",
        "before_tool.deny" => "NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.deny status=pass",
        "before_tool.continuation" => {
            "NOMIFUN_LIVE_SMOKE_STAGE phase=before_tool.continuation status=pass"
        }
        _ => return Err(fail("before_tool.evidence", "BEFORE_TOOL_STAGE_INVALID")),
    };
    eprintln!("{line}");
    Ok(())
}

struct PublishConfirmation {
    revision: u64,
    release_digest: String,
    receipt_id: String,
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_uuid_v7(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == value)
}

fn publish_confirmation(
    response: &Value,
    draft: &Value,
) -> Result<PublishConfirmation, SmokeFailure> {
    const PHASE: &str = "before_tool.publish";
    if response["code"] != "PLUGIN_SERVICE_TEST_INPUT_REQUIRED" {
        return Err(fail(PHASE, "BEFORE_TOOL_PUBLICATION_CONFIRMATION_REQUIRED"));
    }
    let details = &response["details"];
    let revision = details["expected_revision"]
        .as_u64()
        .filter(|revision| {
            *revision <= i64::MAX as u64
                && draft["revision"]
                    .as_u64()
                    .is_some_and(|prior| *revision > prior)
        })
        .ok_or_else(|| fail(PHASE, "BEFORE_TOOL_CONFIRMATION_REVISION_INVALID"))?;
    let release_digest = required_string(
        PHASE,
        details,
        "/release_digest",
        "BEFORE_TOOL_CONFIRMATION_RELEASE_MISSING",
    )?;
    let receipt_id = required_string(
        PHASE,
        details,
        "/receipt_id",
        "BEFORE_TOOL_CONFIRMATION_RECEIPT_MISSING",
    )?;
    if !object_has_only_keys(
        details,
        &[
            "draft_id",
            "expected_revision",
            "release_digest",
            "receipt_id",
            "display_name",
        ],
    ) || details["draft_id"] != draft["id"]
        || details["display_name"] != draft["name"]
        || draft["id"].as_str().is_none_or(|id| !valid_uuid_v7(id))
        || !valid_digest(&release_digest)
        || !valid_uuid_v7(&receipt_id)
    {
        return Err(fail(PHASE, "BEFORE_TOOL_CONFIRMATION_IDENTITY_INVALID"));
    }
    Ok(PublishConfirmation {
        revision,
        release_digest,
        receipt_id,
    })
}

async fn publish_template(router: &Router) -> Result<Value, SmokeFailure> {
    const PHASE: &str = "before_tool.publish";
    let response = successful_json(
        router,
        PHASE,
        Method::POST,
        "/api/plugins/drafts/from-template/before-tool",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let draft = envelope_data(PHASE, response)?;
    if draft["status"] != "ready"
        || draft["service_source"].as_str() != Some(TEMPLATE)
        || !draft["plugin_id"].is_null()
    {
        return Err(fail(PHASE, "BEFORE_TOOL_TEMPLATE_NOT_ORDINARY_DRAFT"));
    }
    let id = required_string(PHASE, &draft, "/id", "BEFORE_TOOL_DRAFT_ID_MISSING")?;
    let revision = draft["revision"]
        .as_u64()
        .ok_or_else(|| fail(PHASE, "BEFORE_TOOL_DRAFT_REVISION_MISSING"))?;
    // The live test is explicitly authorized to publish this synthetic plugin.
    // Exercise the same specific user confirmation as the ordinary UI; never
    // turn a failed business test into Passed or acknowledge another release.
    let (status, confirmation_response) = dispatch_json(
        router,
        PHASE,
        Method::POST,
        format!("/api/plugins/drafts/{id}/save"),
        Some(json!({"expected_revision":revision})),
        TURN_COMMAND_DEADLINE,
    )
    .await?;
    if status != StatusCode::CONFLICT {
        return Err(fail(PHASE, "BEFORE_TOOL_EXPECTED_TEST_INPUT_CONFIRMATION"));
    }
    let confirmation = publish_confirmation(&confirmation_response, &draft)?;
    let pending = successful_json(
        router,
        PHASE,
        Method::GET,
        format!("/api/plugins/drafts/{id}"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let pending = envelope_data(PHASE, pending)?;
    if pending["id"] != draft["id"]
        || pending["status"] != "ready"
        || pending["revision"].as_u64() != Some(confirmation.revision)
        || [
            "name",
            "description",
            "html",
            "service_source",
            "source_manifest",
        ]
        .iter()
        .any(|field| pending[*field] != draft[*field])
    {
        return Err(fail(PHASE, "BEFORE_TOOL_DRAFT_CHANGED_BEFORE_CONFIRMATION"));
    }
    let plugin = required_string(
        PHASE,
        &pending,
        "/plugin_id",
        "BEFORE_TOOL_PENDING_PLUGIN_MISSING",
    )?;
    if !valid_uuid_v7(&plugin) {
        return Err(fail(PHASE, "BEFORE_TOOL_PENDING_PLUGIN_INVALID"));
    }
    let pending_workshop = successful_json(
        router,
        PHASE,
        Method::GET,
        format!("/api/plugins/runtimes/{plugin}/workshop"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let pending_workshop = envelope_data(PHASE, pending_workshop)?;
    let ready = &pending_workshop["ready"];
    if pending_workshop
        .pointer("/plugin/plugin_id")
        .and_then(Value::as_str)
        != Some(plugin.as_str())
        || !pending_workshop["plugin"]["releases"]["active"].is_null()
        || ready
            .pointer("/release/release_digest")
            .and_then(Value::as_str)
            != Some(confirmation.release_digest.as_str())
        || ready.pointer("/test/status").and_then(Value::as_str) != Some("needs_test_input")
        || ready.pointer("/test/receipt_id").and_then(Value::as_str)
            != Some(confirmation.receipt_id.as_str())
        || ready
            .pointer("/test/expected_release_digest")
            .and_then(Value::as_str)
            != Some(confirmation.release_digest.as_str())
        || ready["test"]["release_id"] != ready["release"]["release_id"]
        || ready["service"]["lifecycle"] != "on_demand"
        || ready["service"]["service_contract_digest"]
            .as_str()
            .is_none_or(|digest| !valid_digest(digest))
    {
        return Err(fail(PHASE, "BEFORE_TOOL_PENDING_RELEASE_OR_TEST_CHANGED"));
    }
    let acknowledged_release = ready["release"].clone();
    let acknowledged_service = ready["service"].clone();
    // Use only the frozen revision and identity from the specific 409. A newer
    // draft/release/test cannot be silently substituted after this check.
    let saved = successful_json(
        router,
        PHASE,
        Method::POST,
        format!("/api/plugins/drafts/{id}/save"),
        Some(
            json!({"expected_revision":confirmation.revision,"acknowledge_service_test":{
                "release_digest":confirmation.release_digest,"receipt_id":confirmation.receipt_id
            }}),
        ),
        TURN_COMMAND_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let saved = envelope_data(PHASE, saved)?;
    if saved.pointer("/plugin/lifecycle").and_then(Value::as_str) != Some("enabled")
        || saved.pointer("/plugin/plugin_id").and_then(Value::as_str) != Some(plugin.as_str())
        || saved.pointer("/plugin/releases/active") != Some(&acknowledged_release)
        || (!saved["active_service"].is_null() && saved["active_service"] != acknowledged_service)
        || (!saved["ready"].is_null()
            && (saved["ready"]["test"]["status"] != "needs_test_input"
                || saved["ready"]["test"]["receipt_id"].as_str()
                    != Some(confirmation.receipt_id.as_str())))
    {
        return Err(fail(PHASE, "BEFORE_TOOL_ACKNOWLEDGED_RELEASE_NOT_ACTIVE"));
    }
    // active_service is a manifest descriptor, not running-host evidence. The
    // exact Ready declaration above belongs to the now-active release; no
    // readiness assertion starts or requires the on-demand production Service.
    let catalog = successful_json(
        router,
        PHASE,
        Method::GET,
        "/api/agent-catalog",
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let catalog = envelope_data(PHASE, catalog)?;
    let entries = catalog["capabilities"]
        .as_array()
        .ok_or_else(|| fail(PHASE, "BEFORE_TOOL_CATALOG_INVALID"))?;
    let hooks: Vec<_> = entries
        .iter()
        .filter(|item| item["middleware_phase"] == "before_tool")
        .collect();
    let [hook] = hooks.as_slice() else {
        return Err(fail(PHASE, "BEFORE_TOOL_PUBLICATION_MISSING_OR_AMBIGUOUS"));
    };
    if hook["source_kind"] != "plugin_product_active_release"
        || hook["materialization_state"] != "materialized"
    {
        return Err(fail(PHASE, "BEFORE_TOOL_PUBLICATION_NOT_MATERIALIZED"));
    }
    require_value(
        PHASE,
        hook,
        "/capability",
        "BEFORE_TOOL_EXACT_CAPABILITY_MISSING",
    )
}

async fn select_hook(
    router: &Router,
    preset: &str,
    capability: &Value,
    provider: &str,
    model: &str,
) -> Result<(), SmokeFailure> {
    const PHASE: &str = "before_tool.select";
    let editor = successful_json(
        router,
        PHASE,
        Method::GET,
        format!("/api/agent-presets/{preset}/editor"),
        None,
        LOCAL_API_DEADLINE,
        &[StatusCode::OK],
    )
    .await?;
    let editor = envelope_data(PHASE, editor)?;
    let revision = require_value(
        PHASE,
        &editor,
        "/revision/reference",
        "BEFORE_TOOL_PRESET_REVISION_MISSING",
    )?;
    let mut draft = require_value(PHASE, &editor, "/draft", "BEFORE_TOOL_PRESET_DRAFT_MISSING")?;
    let selected = draft
        .pointer_mut("/document/enabled_capabilities")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| fail(PHASE, "BEFORE_TOOL_SELECTION_INVALID"))?;
    selected.push(json!({"capability":capability,"action_allowlist":[]}));
    draft["document"]["middleware_order"] = json!([capability["id"]]);
    let saved = successful_json(router, PHASE, Method::POST, format!("/api/agent-presets/{preset}/revisions"),
        Some(json!({"expected_current_revision":revision,"draft":draft,"reason":"live before_tool smoke"})),
        LOCAL_API_DEADLINE, &[StatusCode::OK]).await?;
    let saved = envelope_data(PHASE, saved)?;
    let document = &saved["revision"]["document"];
    ensure_single_model_route(document, provider, model)?;
    if document["middleware_order"] != json!([capability["id"]])
        || !document["enabled_capabilities"]
            .as_array()
            .is_some_and(|entries| {
                entries.len() == 2
                    && entries
                        .iter()
                        .any(|entry| entry["capability"] == *capability)
            })
    {
        return Err(fail(PHASE, "BEFORE_TOOL_SELECTION_NOT_SAVED"));
    }
    Ok(())
}

fn target_matches(args: &Value, workspace: &Path, filename: &str) -> bool {
    object_has_only_keys(args, &["file_path", "content"])
        && args["content"].as_str() == Some(CONTENT)
        && target_path_matches(args["file_path"].as_str(), workspace, filename)
}

fn target_path_matches(raw: Option<&str>, workspace: &Path, filename: &str) -> bool {
    let Some(raw) = raw else {
        return false;
    };
    // A denied file must stay absent: canonicalize its parent only.
    let path = Path::new(raw);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace.join(path)
    };
    path.file_name().is_some_and(|name| name == filename)
        && path
            .parent()
            .is_some_and(|parent| canonical_path_eq(parent, workspace))
}

/// Diagnostic categories only: none of these variants is accepted as equal.
/// Never include actual content, a preview, byte values, or an input hash in
/// live diagnostics. The synthetic fixture still requires exact UTF-8 bytes.
fn content_difference(value: &Value) -> Option<&'static str> {
    let Some(actual) = value.as_str() else {
        return Some("BEFORE_TOOL_TARGET_CONTENT_NOT_STRING");
    };
    if actual.as_bytes() == CONTENT.as_bytes() {
        return None;
    }
    let line = CONTENT
        .strip_suffix('\n')
        .expect("fixed fixture ends in LF");
    Some(if actual == line {
        "BEFORE_TOOL_TARGET_CONTENT_MISSING_FINAL_LF"
    } else if actual == format!("{line}\\n") {
        "BEFORE_TOOL_TARGET_CONTENT_LITERAL_BACKSLASH_N"
    } else if actual == format!("{line}\r\n") {
        "BEFORE_TOOL_TARGET_CONTENT_CRLF"
    } else if actual == format!("{CONTENT}\n") {
        "BEFORE_TOOL_TARGET_CONTENT_EXTRA_FINAL_LF"
    } else {
        "BEFORE_TOOL_TARGET_CONTENT_OTHER_BYTES"
    })
}

/// Return the host-authored turn identity only after actual tool and subsequent
/// assistant projections agree. Assistant words alone can never prove denial.
fn inspect_turn(
    messages: &[Value],
    workspace: &Path,
    denied: bool,
) -> Result<String, SmokeFailure> {
    let phase = if denied {
        "before_tool.deny"
    } else {
        "before_tool.allow"
    };
    if messages.iter().any(|message| {
        message
            .pointer("/projection/source")
            .and_then(Value::as_str)
            == Some("model_failover")
    }) {
        return Err(fail(phase, "BEFORE_TOOL_MODEL_FALLBACK_OBSERVED"));
    }
    let tools: Vec<_> = messages
        .iter()
        .filter(|m| {
            m.pointer("/projection/name")
                .and_then(Value::as_str)
                .is_some()
        })
        .collect();
    let [tool_message] = tools.as_slice() else {
        return Err(fail(phase, "BEFORE_TOOL_EXACTLY_ONE_WRITE_REQUIRED"));
    };
    let tool = &tool_message["projection"];
    let expected_status = if denied { "error" } else { "completed" };
    if tool_message["presentation_intent"] != "left" {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_PRESENTATION_INVALID"));
    }
    if tool["name"] != "Write" {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_NAME_INVALID"));
    }
    if tool["status"] != expected_status {
        return Err(fail(
            phase,
            if tool["status"] == "error" {
                "BEFORE_TOOL_TARGET_STATUS_ERROR"
            } else if tool["status"] == "running" {
                "BEFORE_TOOL_TARGET_STATUS_RUNNING"
            } else {
                "BEFORE_TOOL_TARGET_STATUS_INVALID"
            },
        ));
    }
    if tool.get("input").is_none() {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_INPUT_MISSING"));
    }
    if tool["input"] != tool["args"] {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_INPUT_MISMATCH"));
    }
    if !tool["args"].is_object() || !object_has_only_keys(&tool["args"], &["file_path", "content"])
    {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_ARGS_INVALID"));
    }
    if !target_path_matches(
        tool["args"]["file_path"].as_str(),
        workspace,
        if denied { DENY_FILE } else { ALLOW_FILE },
    ) {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_PATH_INVALID"));
    }
    if let Some(code) = content_difference(&tool["args"]["content"]) {
        if !denied && std::fs::read(workspace.join(ALLOW_FILE)).ok().as_deref() == Some(CONTENT.as_bytes()) {
            return Err(fail(phase, "BEFORE_TOOL_TARGET_CONTENT_PROJECTION_DIFFERS_FROM_FILE"));
        }
        return Err(fail(phase, code));
    }
    if tool["call_id"].as_str().is_none_or(str::is_empty) {
        return Err(fail(phase, "BEFORE_TOOL_TARGET_CALL_ID_INVALID"));
    }
    if denied
        && tool["output"].as_str()
            != Some(
                format!(
                    "Blocked by before_tool hook: {DENY_REASON}. Target tool was not executed."
                )
                .as_str(),
            )
    {
        return Err(fail(phase, "BEFORE_TOOL_HOST_DENIAL_MISSING"));
    }
    let turn = required_string(phase, tool, "/turn_id", "BEFORE_TOOL_TURN_ID_MISSING")?;
    let final_messages: Vec<_> = messages
        .iter()
        .filter_map(|message| {
            assistant_text_projection(message).map(|projection| (message, projection))
        })
        .collect();
    let [(assistant_message, assistant)] = final_messages.as_slice() else {
        return Err(fail(phase, "BEFORE_TOOL_MODEL_CONTINUATION_MISSING"));
    };
    if assistant["turn_id"].as_str() != Some(turn.as_str())
        || assistant["content"].as_str().map(str::trim)
            != Some(if denied { DENY_MARKER } else { ALLOW_MARKER })
        || assistant_message["last_seq"]
            .as_u64()
            .zip(tool_message["last_seq"].as_u64())
            .is_none_or(|(after, before)| after <= before)
    {
        return Err(fail(phase, "BEFORE_TOOL_MODEL_CONTINUATION_INVALID"));
    }
    Ok(turn)
}

async fn wait_turn(
    router: &Router,
    session: &str,
    cursor: u64,
    workspace: &Path,
    denied: bool,
) -> Result<String, SmokeFailure> {
    let phase = if denied {
        "before_tool.deny"
    } else {
        "before_tool.allow"
    };
    hard_deadline(
        phase,
        "BEFORE_TOOL_TURN_DEADLINE_EXCEEDED",
        CODING_STAGE_DEADLINE,
        async {
            loop {
                let (messages, _) = session_messages_after(router, phase, session, cursor).await?;
                let observation = successful_json(
                    router,
                    phase,
                    Method::GET,
                    format!("/api/agent-sessions/{session}"),
                    None,
                    LOCAL_API_DEADLINE,
                    &[StatusCode::OK],
                )
                .await?;
                let observation = envelope_data(phase, observation)?;
                if observation.pointer("/head/status").and_then(Value::as_str) == Some("ready")
                    && !messages.is_empty()
                {
                    // The first read can precede terminal publication while
                    // the later head read already sees ready. Read final
                    // projections after that observation; running never counts
                    // as completed and every original assertion still applies.
                    let (final_messages, _) =
                        session_messages_after(router, phase, session, cursor).await?;
                    return inspect_turn(&final_messages, workspace, denied);
                }
                // Do not forward error bodies. The default deny tool result is
                // expected; all other durable errors become one fixed diagnostic.
                if messages.iter().any(|m| {
                    m.pointer("/projection/name").is_none()
                        && (m.pointer("/projection/type").and_then(Value::as_str) == Some("error")
                            || m["message_status"] == "error")
                }) {
                    return Err(fail(phase, "BEFORE_TOOL_TURN_FAILED"));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        },
    )
    .await
}

struct RequestedTurn {
    api_operation: String,
    idempotency_key: String,
    prompt: String,
}

type ReceiptRow = (i64, String, String, String, String);
async fn verify_receipt(
    root: &Path,
    session: &str,
    capability: &str,
    after: i64,
    turn: &RequestedTurn,
    denied: bool,
) -> Result<i64, SmokeFailure> {
    let phase = if denied {
        "before_tool.deny"
    } else {
        "before_tool.allow"
    };
    // The API's nominal operation identity is not the Conversation receiver's
    // durable receipt key. Preserve the request token and verify both scopes.
    if turn.api_operation != format!("nomi-core-turn:{session}:{}", turn.idempotency_key) {
        return Err(fail(phase, "BEFORE_TOOL_API_TURN_SCOPE_MISMATCH"));
    }
    let options = SqliteConnectOptions::new()
        .filename(root.join("data/nomifun-backend.db"))
        .read_only(true)
        .create_if_missing(false);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|_| fail(phase, "BEFORE_TOOL_RECEIPT_OPEN_FAILED"))?;
    let evidence: Result<(String, Vec<ReceiptRow>), SmokeFailure> = async {
        // public_turn_operation_id is owner/session/token scoped by the real
        // Conversation receiver. Verify the persisted raw user request and its
        // source message too; an arbitrary newest receipt is not acceptable.
        let delivery = nomifun_db::sqlx::query_as::<_, (String, i64, i64, String, Option<i64>)>(
            "SELECT r.operation_id,CASE WHEN json_extract(r.request_payload,'$.content')=? THEN 1 ELSE 0 END,CASE WHEN EXISTS(SELECT 1 FROM messages m WHERE m.message_id=r.message_id AND m.conversation_id=r.conversation_id AND m.position='right' AND json_extract(m.content,'$.content')=?) THEN 1 ELSE 0 END,r.status,r.result_ok FROM conversation_delivery_receipts r JOIN conversations c ON c.conversation_id=r.conversation_id AND c.user_id=r.user_id WHERE r.conversation_id=? AND r.kind='turn' AND r.operation_id=('public-turn:v1:' || r.user_id || ':' || r.conversation_id || ':' || ?) LIMIT 2")
            .bind(&turn.prompt).bind(&turn.prompt).bind(session).bind(&turn.idempotency_key)
            .fetch_all(&mut connection).await.map_err(|_| fail(phase, "BEFORE_TOOL_DELIVERY_RECEIPT_READ_FAILED"))?;
        let [(operation, request_matches, source_matches, status, result_ok)] = delivery.as_slice() else {
            return Err(fail(phase, "BEFORE_TOOL_DELIVERY_RECEIPT_IDENTITY_MISSING"));
        };
        if *request_matches != 1 { return Err(fail(phase, "BEFORE_TOOL_DELIVERY_REQUEST_MISMATCH")); }
        if *source_matches != 1 { return Err(fail(phase, "BEFORE_TOOL_DELIVERY_SOURCE_MESSAGE_MISMATCH")); }
        if status != "completed" || *result_ok != Some(1) { return Err(fail(phase, "BEFORE_TOOL_DELIVERY_NOT_COMPLETED")); }
        let rows = nomifun_db::sqlx::query_as::<_, ReceiptRow>(
            "SELECT e.id,e.state,COALESCE(e.observation_json,''),e.operation_id,e.turn_operation_id FROM conversation_hosted_effects e JOIN conversation_delivery_receipts r ON r.operation_id=e.turn_operation_id AND r.conversation_id=e.conversation_id AND r.user_id=e.user_id WHERE e.conversation_id=? AND e.capability_id=? AND e.action_name='agent.before_tool' AND e.id>? AND r.kind='turn' AND r.status='completed' AND r.result_ok=1 ORDER BY e.id LIMIT 8")
            .bind(session).bind(capability).bind(after).fetch_all(&mut connection).await
            .map_err(|_| fail(phase, "BEFORE_TOOL_RECEIPT_READ_FAILED"))?;
        Ok((operation.clone(), rows))
    }.await;
    let closed = connection.close().await;
    let (durable_operation, rows) = evidence?;
    closed.map_err(|_| fail(phase, "BEFORE_TOOL_RECEIPT_CLOSE_FAILED"))?;
    let [(id, state, observation, operation, receipt_turn)] = rows.as_slice() else {
        return Err(fail(phase, "BEFORE_TOOL_EXACTLY_ONE_RECEIPT_REQUIRED"));
    };
    if state != "returned" {
        return Err(fail(phase, "BEFORE_TOOL_RECEIPT_STATE_NOT_RETURNED"));
    }
    if !operation.starts_with("nomi-before-tool:") {
        return Err(fail(phase, "BEFORE_TOOL_RECEIPT_OPERATION_PREFIX_INVALID"));
    }
    if receipt_turn != &durable_operation {
        return Err(fail(phase, "BEFORE_TOOL_RECEIPT_TURN_IDENTITY_MISMATCH"));
    }
    let observation: Value = serde_json::from_str(observation)
        .map_err(|_| fail(phase, "BEFORE_TOOL_RECEIPT_INVALID"))?;
    verify_decision_digest(&observation, denied)?;
    Ok(*id)
}

fn verify_decision_digest(observation: &Value, denied: bool) -> Result<(), SmokeFailure> {
    let expected = if denied {
        json!({"decision":"deny","reason":DENY_REASON})
    } else {
        json!({"decision":"allow"})
    };
    let bytes = serde_json::to_vec(&expected).map_err(|_| {
        fail(
            "before_tool.evidence",
            "BEFORE_TOOL_DECISION_ENCODING_FAILED",
        )
    })?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if observation["content_omitted"] != true
        || observation["serialized_bytes"].as_u64() != Some(bytes.len() as u64)
        || observation["sha256"].as_str() != Some(digest.as_str())
        || !object_has_only_keys(
            observation,
            &["content_omitted", "serialized_bytes", "sha256"],
        )
    {
        return Err(fail(
            "before_tool.evidence",
            "BEFORE_TOOL_REAL_DECISION_DIGEST_MISMATCH",
        ));
    }
    Ok(())
}

const WRITE_ARGUMENTS_PREFIX: &str = "WRITE_ARGUMENTS_JSON=";
fn stage_prompt(denied: bool) -> Result<String, SmokeFailure> {
    let file = if denied { DENY_FILE } else { ALLOW_FILE };
    let marker = if denied { DENY_MARKER } else { ALLOW_MARKER };
    let arguments = serde_json::to_string(&json!({"file_path":file,"content":CONTENT}))
        .map_err(|_| fail("before_tool.prompt", "BEFORE_TOOL_PROMPT_ENCODING_FAILED"))?;
    let note = if denied {
        "This requested filename intentionally exercises the selected sensitive-file business check. It is a synthetic filename, not a credential source. "
    } else {
        ""
    };
    Ok(format!(
        "This is a disposable business-rule integration test. All content is synthetic test data, with no credentials. Call the native Write tool exactly once using this exact argument object:\n{WRITE_ARGUMENTS_PREFIX}{arguments}\nThe content value is one line terminated by exactly one LF newline (U+000A). The Write tool does not add a newline for you. Its content argument must itself contain the final LF. Preserve that final newline in the tool argument and file; do not trim it or write a literal backslash followed by n. 请特别注意：content 参数本身必须包含最后一个换行符，工具不会自动补换行。 Do not read files, run shell commands, use other tools, retry, rename the target, or bypass a selected check. {note}If the business check rejects the attempted write, consume that tool error and stop. After the tool result arrives, reply with exactly {marker}; no earlier prose or other text."
    ))
}

pub(super) async fn run(
    router: &Router,
    api_key: &str,
    model: &str,
    root: &Path,
    stages: &mut Vec<&'static str>,
) -> Result<(), SmokeFailure> {
    let provider = configure_stepfun(router, api_key, STEPFUN_PLAN_BASE_URL, model).await?;
    run_with_provider(router, &provider, model, root, stages).await
}

async fn run_with_provider(
    router: &Router,
    provider: &str,
    model: &str,
    root: &Path,
    stages: &mut Vec<&'static str>,
) -> Result<(), SmokeFailure> {
    let capability = publish_template(router).await?;
    let capability_id = required_string(
        "before_tool.select",
        &capability,
        "/id",
        "BEFORE_TOOL_CAPABILITY_ID_MISSING",
    )?;
    let (preset, _) =
        create_agent_preset(
            router,
            &provider,
            model,
            &[(
                "workspace.files",
                &["workspace.files/write"],
                "workspace",
            )],
        )
        .await?;
    select_hook(router, &preset, &capability, &provider, model).await?;
    let (session, _) = create_session(
        router,
        &preset,
        &provider,
        model,
        json!([{"resource_kind":"workspace","resource_id":"default-workspace"}]),
    )
    .await?;
    let workspace = root.join("before-tool-workspace");
    std::fs::create_dir(&workspace).map_err(|_| {
        fail(
            "before_tool.workspace",
            "BEFORE_TOOL_WORKSPACE_CREATE_FAILED",
        )
    })?;
    let workspace = workspace.canonicalize().map_err(|_| {
        fail(
            "before_tool.workspace",
            "BEFORE_TOOL_WORKSPACE_CANONICALIZE_FAILED",
        )
    })?;
    bind_session_workspace(router, &session, &workspace).await?;
    assert_session_runtime(router, &session).await?;
    stages.push("before_tool.publish_select");
    let mut receipt_cursor = 0;
    for denied in [false, true] {
        let phase = if denied {
            "before_tool.deny"
        } else {
            "before_tool.allow"
        };
        let file = if denied { DENY_FILE } else { ALLOW_FILE };
        if workspace.join(file).exists() {
            return Err(fail(phase, "BEFORE_TOOL_TARGET_PREEXISTS"));
        }
        let prompt = stage_prompt(denied)?;
        let cursor = session_message_cursor(router, phase, &session).await?;
        let idempotency_key = uuid::Uuid::now_v7().to_string();
        let operation =
            start_session_turn(router, phase, &session, &idempotency_key, prompt.clone()).await?;
        wait_turn(router, &session, cursor, &workspace, denied).await?;
        receipt_cursor = verify_receipt(
            root,
            &session,
            &capability_id,
            receipt_cursor,
            &RequestedTurn {
                api_operation: operation,
                idempotency_key,
                prompt,
            },
            denied,
        )
        .await?;
        if denied {
            if workspace.join(DENY_FILE).exists() {
                return Err(fail(phase, "BEFORE_TOOL_DENIED_TARGET_WAS_WRITTEN"));
            }
        } else if std::fs::read_to_string(workspace.join(ALLOW_FILE))
            .ok()
            .as_deref()
            != Some(CONTENT)
        {
            return Err(fail(phase, "BEFORE_TOOL_ALLOWED_WRITE_MISSING"));
        }
        assert_session_runtime(router, &session).await?;
        stages.push(phase);
        if denied {
            stages.push("before_tool.continuation");
        }
    }
    if std::fs::read_to_string(workspace.join(ALLOW_FILE))
        .ok()
        .as_deref()
        != Some(CONTENT)
    {
        return Err(fail(
            "before_tool.evidence",
            "BEFORE_TOOL_PRIOR_WRITE_CHANGED",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn before_tool_receipt_requires_exact_allow_or_deny_decision_digest() {
        for denied in [false, true] {
            let decision = if denied {
                json!({"decision":"deny","reason":DENY_REASON})
            } else {
                json!({"decision":"allow"})
            };
            let bytes = serde_json::to_vec(&decision).unwrap();
            let evidence = json!({"content_omitted":true,"serialized_bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))});
            assert!(verify_decision_digest(&evidence, denied).is_ok());
            assert!(verify_decision_digest(&evidence, !denied).is_err());
        }
        assert!(
            verify_decision_digest(&json!({"decision":"deny","reason":DENY_REASON}), true).is_err()
        );
    }
    #[test]
    fn before_tool_model_claim_alone_is_not_execution_evidence() {
        let root = tempfile::tempdir().unwrap();
        let claim = json!({"presentation_intent":"left","last_seq":2,"projection":{"content":DENY_MARKER,"turn_id":"fixture-turn"}});
        assert!(inspect_turn(&[claim], root.path(), true).is_err());
        assert!(target_matches(
            &json!({"file_path":DENY_FILE,"content":CONTENT}),
            root.path(),
            DENY_FILE
        ));
        assert!(!root.path().join(DENY_FILE).exists());
        assert!(!target_matches(
            &json!({"file_path":"../.env.hook-smoke","content":CONTENT}),
            root.path(),
            DENY_FILE
        ));
    }
    #[test]
    fn before_tool_denial_needs_host_error_and_later_assistant_in_same_turn() {
        let root = tempfile::tempdir().unwrap();
        let mut tool = json!({"presentation_intent":"left","last_seq":3,"projection":{
            "name":"Write","status":"error","call_id":"fixture-call","turn_id":"fixture-turn",
            "args":{"file_path":DENY_FILE,"content":CONTENT},"input":{"file_path":DENY_FILE,"content":CONTENT},
            "output":format!("Blocked by before_tool hook: {DENY_REASON}. Target tool was not executed.")
        }});
        let mut assistant = json!({"presentation_intent":"left","last_seq":4,
            "projection":{"content":DENY_MARKER,"turn_id":"fixture-turn"}});
        assert!(inspect_turn(&[tool.clone(), assistant.clone()], root.path(), true).is_ok());
        assistant["last_seq"] = json!(2);
        assert!(inspect_turn(&[tool.clone(), assistant.clone()], root.path(), true).is_err());
        assistant["last_seq"] = json!(4);
        tool["projection"]["output"] = json!("model said denied");
        assert!(inspect_turn(&[tool, assistant], root.path(), true).is_err());
    }
    #[test]
    fn before_tool_publish_confirmation_is_bound_to_exact_draft_revision_and_receipt() {
        let draft = json!({"id":uuid::Uuid::now_v7().to_string(),"revision":1,"name":"fixture"});
        let confirmation = json!({"code":"PLUGIN_SERVICE_TEST_INPUT_REQUIRED","details":{
            "draft_id":draft["id"],"expected_revision":3,"release_digest":"a".repeat(64),
            "receipt_id":uuid::Uuid::now_v7().to_string(),"display_name":"fixture"
        }});
        assert!(publish_confirmation(&confirmation, &draft).is_ok());
        for (field, invalid) in [
            ("draft_id", json!(uuid::Uuid::now_v7().to_string())),
            ("expected_revision", json!(1)),
            ("release_digest", json!("invalid")),
            ("receipt_id", json!("invalid")),
            ("display_name", json!("other")),
        ] {
            let mut changed = confirmation.clone();
            changed["details"][field] = invalid;
            assert!(publish_confirmation(&changed, &draft).is_err());
        }
        let mut unrelated = confirmation;
        unrelated["code"] = json!("BAD_REQUEST");
        assert!(publish_confirmation(&unrelated, &draft).is_err());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn before_tool_ordinary_node_template_gates_native_write_via_loopback() {
        use futures_util::FutureExt;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        let upstream = wiremock::MockServer::start().await;
        let calls = Arc::new(AtomicUsize::new(0));
        let consumption = Arc::new(AtomicUsize::new(0));
        let failure = Arc::new(Mutex::new(None::<&'static str>));
        let (count, consumed, observed_failure) =
            (calls.clone(), consumption.clone(), failure.clone());
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |request: &wiremock::Request| {
                let step = count.fetch_add(1, Ordering::SeqCst);
                let body = request.body_json::<Value>().unwrap_or(Value::Null);
                let reject = |code: &'static str| {
                    *observed_failure.lock().unwrap() = Some(code);
                    wiremock::ResponseTemplate::new(400).set_body_json(json!({"error":{"message":"LOCAL_FIXTURE_REJECTED"}}))
                };
                if body["model"] != STEPFUN_PLAN_MODEL { return reject("LOOPBACK_MODEL_MISMATCH"); }
                if step >= 4 { return reject("LOOPBACK_EXTRA_MODEL_REQUEST"); }
                let frame = if step % 2 == 0 {
                    let advertised = body["tools"].as_array().is_some_and(|tools| tools.iter()
                        .any(|tool| tool.pointer("/function/name").and_then(Value::as_str) == Some("Write")));
                    if !advertised { return reject("LOOPBACK_NATIVE_WRITE_NOT_ADVERTISED"); }
                    let user = body["messages"].as_array().and_then(|messages| messages.iter().rev().find(|message| message["role"] == "user"));
                    let content = match user.and_then(|message| message.get("content")) {
                        Some(Value::String(text)) => text.clone(),
                        Some(Value::Array(parts)) => parts.iter().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
                        _ => return reject("LOOPBACK_USER_INPUT_MISSING"),
                    };
                    let argument_lines: Vec<_> = content.lines().filter_map(|line| line.strip_prefix(WRITE_ARGUMENTS_PREFIX)).collect();
                    let [line] = argument_lines.as_slice() else { return reject("LOOPBACK_EXACT_ARGUMENT_LINE_MISSING"); };
                    let arguments = serde_json::from_str::<Value>(line).unwrap_or(Value::Null);
                    let denied = match arguments["file_path"].as_str() {
                        Some(DENY_FILE) => true,
                        Some(ALLOW_FILE) => false,
                        _ => return reject("LOOPBACK_REQUESTED_TARGET_INVALID"),
                    };
                    if denied != (step == 2) || arguments["content"].as_str() != Some(CONTENT)
                        || !object_has_only_keys(&arguments, &["file_path", "content"]) {
                        return reject("LOOPBACK_REQUESTED_ARGUMENTS_INVALID");
                    }
                    let id = if denied { "loopback-native-deny" } else { "loopback-native-allow" };
                    json!({"id":"local-before-tool","choices":[{"index":0,"delta":{"role":"assistant","tool_calls":[{
                        "index":0,"id":id,"type":"function","function":{"name":"Write",
                        "arguments":arguments.to_string()}
                    }]},"finish_reason":null}]})
                } else {
                    let denied = step == 3;
                    let id = if denied { "loopback-native-deny" } else { "loopback-native-allow" };
                    let result = body["messages"].as_array().and_then(|messages| messages.iter().rev()
                        .find(|message| message["role"] == "tool"));
                    if result.is_none_or(|message| message["tool_call_id"] != id || message["content"].as_str()
                        .is_none_or(|text| text.contains(DENY_REASON) != denied)) {
                        return reject("LOOPBACK_MODEL_DID_NOT_CONSUME_TOOL_RESULT");
                    }
                    consumed.fetch_add(1, Ordering::SeqCst);
                    json!({"id":"local-before-tool","choices":[{"index":0,"delta":{"role":"assistant",
                        "content":if denied { DENY_MARKER } else { ALLOW_MARKER }},"finish_reason":null}]})
                };
                let done = json!({"id":"local-before-tool","choices":[{"index":0,"delta":{},
                    "finish_reason":if step % 2 == 0 { "tool_calls" } else { "stop" }}]});
                wiremock::ResponseTemplate::new(200).insert_header("content-type","text/event-stream")
                    .set_body_string(format!("data: {frame}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
            }).mount(&upstream).await;
        let root = tempfile::tempdir().expect("local fixture root");
        let fixture = build_fixture(&root).await.expect("local fixture bootstrap");
        let router = fixture.application.router();
        let mut stages = Vec::new();
        let result = std::panic::AssertUnwindSafe(hard_deadline(
            "before_tool.loopback",
            "LOOPBACK_DEADLINE_EXCEEDED",
            ENGINE_SMOKE_DEADLINE,
            async {
                // Fixed synthetic authentication text for the local mock only; no
                // real credential/environment input and no external model request.
                let provider = configure_stepfun(
                    &router,
                    "local-before-tool-placeholder",
                    &format!("{}/v1", upstream.uri()),
                    STEPFUN_PLAN_MODEL,
                )
                .await?;
                run_with_provider(
                    &router,
                    &provider,
                    STEPFUN_PLAN_MODEL,
                    root.path(),
                    &mut stages,
                )
                .await
            },
        ))
        .catch_unwind()
        .await;
        drop(router);
        let LiveFixture {
            _environment: environment,
            application,
        } = fixture;
        let closed = tokio::time::timeout(SHUTDOWN_DEADLINE, application.close()).await;
        drop(environment);
        assert!(
            closed.is_ok_and(|result| result.is_ok()),
            "LOOPBACK_APPLICATION_SHUTDOWN_FAILED"
        );
        if let Some(code) = *failure.lock().unwrap() {
            panic!("{code}");
        }
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => panic!("LOOPBACK_BEFORE_TOOL_FAILED {error}"),
            Err(_) => panic!("LOOPBACK_UNEXPECTED_PANIC"),
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            4,
            "LOOPBACK_REQUEST_COUNT_INVALID"
        );
        assert_eq!(
            consumption.load(Ordering::SeqCst),
            2,
            "LOOPBACK_RESULT_CONSUMPTION_MISSING"
        );
        assert_eq!(
            stages,
            [
                "before_tool.publish_select",
                "before_tool.allow",
                "before_tool.deny",
                "before_tool.continuation"
            ]
        );
    }

    #[tokio::test]
    async fn before_tool_wait_turn_refetches_messages_after_ready_head() {
        use axum::extract::Query;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let ready_seen = Arc::new(AtomicBool::new(false));
        let root = tempfile::tempdir().unwrap();
        let observed = ready_seen.clone();
        let head_observed = ready_seen.clone();
        let router = Router::new()
            .route("/api/agent-sessions/fixture/messages", axum::routing::get(move |Query(query): Query<std::collections::HashMap<String,String>>| {
                let ready = observed.load(Ordering::SeqCst);
                async move {
                    let after = query.get("after_seq").and_then(|value| value.parse::<u64>().ok()).unwrap_or(0);
                    let tail = if ready { 3 } else { 1 };
                    let messages = if after == 0 {
                        let tool = json!({"session_id":"fixture","presentation_intent":"left","last_seq":if ready {2} else {1},"projection":{
                            "name":"Write","status":if ready {"error"} else {"running"},"call_id":"fixture-call","turn_id":"fixture-turn",
                            "args":{"file_path":DENY_FILE,"content":CONTENT},"input":{"file_path":DENY_FILE,"content":CONTENT},
                            "output":format!("Blocked by before_tool hook: {DENY_REASON}. Target tool was not executed.")}});
                        if ready { vec![tool, json!({"session_id":"fixture","presentation_intent":"left","last_seq":3,
                            "projection":{"content":DENY_MARKER,"turn_id":"fixture-turn"}})] } else { vec![tool] }
                    } else { vec![] };
                    axum::Json(json!({"success":true,"data":{"agent_session_id":"fixture","messages":messages,
                        "next_cursor":{"agent_session_id":"fixture","seq":tail}}}))
                }
            }))
            .route("/api/agent-sessions/fixture", axum::routing::get(move || {
                head_observed.store(true, Ordering::SeqCst);
                async { axum::Json(json!({"success":true,"data":{"head":{"status":"ready"}}})) }
            }));
        let turn = wait_turn(&router, "fixture", 0, root.path(), true)
            .await
            .unwrap();
        assert_eq!(turn, "fixture-turn");
        assert!(ready_seen.load(Ordering::SeqCst));
    }
    #[test]
    fn before_tool_stage_prompt_has_one_exact_target_without_cross_stage_filename() {
        for denied in [false, true] {
            let prompt = stage_prompt(denied).unwrap();
            let lines: Vec<_> = prompt
                .lines()
                .filter_map(|line| line.strip_prefix(WRITE_ARGUMENTS_PREFIX))
                .collect();
            assert_eq!(lines.len(), 1);
            let arguments: Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(
                arguments,
                json!({"file_path":if denied { DENY_FILE } else { ALLOW_FILE },"content":CONTENT})
            );
            assert!(!prompt.contains(if denied { ALLOW_FILE } else { DENY_FILE }));
        }
    }
    #[test]
    fn before_tool_content_evidence_classifies_without_accepting_byte_differences() {
        let line = CONTENT.strip_suffix('\n').unwrap();
        assert_eq!(content_difference(&json!(CONTENT)), None);
        for (value, code) in [
            (Value::Null, "BEFORE_TOOL_TARGET_CONTENT_NOT_STRING"),
            (json!(42), "BEFORE_TOOL_TARGET_CONTENT_NOT_STRING"),
            (json!(line), "BEFORE_TOOL_TARGET_CONTENT_MISSING_FINAL_LF"),
            (
                json!(format!("{line}\\n")),
                "BEFORE_TOOL_TARGET_CONTENT_LITERAL_BACKSLASH_N",
            ),
            (
                json!(format!("{line}\r\n")),
                "BEFORE_TOOL_TARGET_CONTENT_CRLF",
            ),
            (
                json!(format!("{CONTENT}\n")),
                "BEFORE_TOOL_TARGET_CONTENT_EXTRA_FINAL_LF",
            ),
            (
                json!("different synthetic data"),
                "BEFORE_TOOL_TARGET_CONTENT_OTHER_BYTES",
            ),
        ] {
            assert_eq!(content_difference(&value), Some(code));
            assert_ne!(value.as_str(), Some(CONTENT));
        }
    }
}
