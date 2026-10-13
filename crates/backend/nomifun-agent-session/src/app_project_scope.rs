//! Application affiliation is an immutable opening fact. Current-generation
//! openings from before the explicit field are recognized by their complete
//! canonical creation receipt, never by a live workspace or navigation pointer.
use nomifun_agent_contracts::{AgentBindingValue, PrincipalRef};
use serde::Deserialize;
use serde_json::Value;

use super::{SessionStoreError, StoredSessionRow, validate_uuidv7};

#[derive(Deserialize)]
struct OpeningProvenance {
    event_id: String,
    producer_id: String,
    idempotency_key: String,
    correlation_id: String,
    causation_event_id: Option<String>,
}

pub(super) fn current_generation_app_opening(row: &StoredSessionRow, binding: &AgentBindingValue)
    -> Result<Option<String>, SessionStoreError> {
    let owner: PrincipalRef = serde_json::from_str(&row.owner_ref_json)?;
    if owner.principal_kind != "user" { return Ok(None); }
    let Some(provenance) = row.opening_provenance_json.as_deref() else { return Ok(None); };
    let provenance: OpeningProvenance = serde_json::from_str(provenance)?;
    let prefix = format!("user:{}:open:app-development:", owner.principal_id);
    let Some(command) = provenance.idempotency_key.strip_prefix(&prefix) else { return Ok(None); };
    let Some((app, suffix)) = command.split_once(':') else { return Ok(None); };
    if validate_uuidv7(app, "app_project_id").is_err()
        || !(suffix == "initial" || validate_uuidv7(suffix, "prior_app_session_id").is_ok()
            || suffix.strip_prefix("new:").is_some_and(|key| validate_uuidv7(key, "app_creation_key").is_ok())) {
        return Ok(None);
    }
    let key = &provenance.idempotency_key;
    if provenance.producer_id != "session_api" || provenance.event_id != format!("opening:{key}")
        || provenance.correlation_id != format!("open:{key}") || provenance.causation_event_id.is_some() {
        return Ok(None);
    }
    let payload: Value = serde_json::from_str(row.opening_json.as_deref().ok_or_else(||
        SessionStoreError::InvalidSession("application opening lost its payload".into()))?)?;
    if payload.get("operation_id").and_then(Value::as_str) != Some(format!("open:{key}").as_str())
        || !["parent_session_id", "fork_base_payload_id", "remote_binding_provenance"].iter()
            .all(|field| payload.get(field).is_some_and(Value::is_null)) {
        return Ok(None);
    }
    let mut workspaces = binding.typed_resource_bindings.iter()
        .filter(|resource| resource.resource_kind.as_ref() == "workspace");
    let Some(workspace) = workspaces.next() else { return Ok(None); };
    if workspaces.next().is_some() || workspace.owner_id != owner.principal_id
        || !workspace.resource_id.as_ref().starts_with("selected-workspace-")
        || workspace.typed_parameters.contains_key("workspace_location") {
        return Ok(None);
    }
    let Some(root) = workspace.typed_parameters.get("workspace_root") else { return Ok(None); };
    let normalized = root.replace('\\', "/");
    let absolute = normalized.starts_with('/') || normalized.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && normalized.as_bytes().get(1..3) == Some(b":/");
    if !absolute || root.trim() != root || normalized.split('/').any(|segment| matches!(segment, "." | ".."))
        || !normalized.ends_with(&format!("/app-hosting/apps/{app}")) {
        return Ok(None);
    }
    Ok(Some(app.to_owned()))
}

fn uuidv7_sql(value: &str) -> String {
    format!("(length({value})=36 AND substr({value},9,1)='-' AND substr({value},14,1)='-' \
        AND substr({value},19,1)='-' AND substr({value},24,1)='-' AND substr({value},15,1)='7' \
        AND length(replace({value},'-',''))=32 \
        AND replace({value},'-','') NOT GLOB '*[^0-9a-f]*')")
}

/// SQL counterpart of the typed opening reader, shared by canonical list and
/// message-search projections so filtering happens before count and pagination.
/// The alias is a trusted identifier from the host adapter, never request data.
pub(super) fn app_project_scope_sql(alias: &str) -> String {
    assert!(!alias.is_empty() && alias.bytes().all(|value| value.is_ascii_alphanumeric() || value == b'_'));
    let owner = format!("json_extract({alias}.owner_ref_json,'$.principal_id')");
    let prefix = format!("('user:'||{owner}||':open:app-development:')");
    let app = format!("substr(e.idempotency_key,length({prefix})+1,36)");
    let suffix = format!("substr(e.idempotency_key,length({prefix})+38)");
    let root = "replace(json_extract(workspace.value,'$.typed_parameters.workspace_root'),char(92),'/')";
    let valid_suffix = format!("({suffix}='initial' OR {} OR (substr({suffix},1,4)='new:' AND {}))",
        uuidv7_sql(&suffix), uuidv7_sql(&format!("substr({suffix},5)")));
    let explicit = "json_extract(e.inline_json,'$.metadata.app_project_id')";
    format!("(SELECT CASE WHEN json_type(e.inline_json,'$.metadata.app_project_id') IS NOT NULL \
        AND json_type(e.inline_json,'$.metadata.app_project_id')<>'null' \
        THEN CASE WHEN json_type(e.inline_json,'$.metadata.app_project_id')='text' AND {} \
            THEN {explicit} ELSE NULL END \
        WHEN json_extract({alias}.owner_ref_json,'$.principal_kind')='user' \
        AND COALESCE(json_extract(e.inline_json,'$.metadata.purpose'),'conversation')='conversation' \
        AND e.producer_id='session_api' AND substr(e.idempotency_key,1,length({prefix}))={prefix} \
        AND substr(e.idempotency_key,length({prefix})+37,1)=':' AND {} AND {valid_suffix} \
        AND e.event_id='opening:'||e.idempotency_key AND e.correlation_id='open:'||e.idempotency_key \
        AND e.causation_event_id IS NULL AND json_extract(e.inline_json,'$.operation_id')='open:'||e.idempotency_key \
        AND json_type(e.inline_json,'$.parent_session_id')='null' \
        AND json_type(e.inline_json,'$.fork_base_payload_id')='null' \
        AND json_type(e.inline_json,'$.remote_binding_provenance')='null' \
        AND (SELECT COUNT(*) FROM json_each(e.inline_json,'$.agent_binding.typed_resource_bindings') workspace \
            WHERE json_extract(workspace.value,'$.resource_kind')='workspace')=1 \
        AND EXISTS(SELECT 1 FROM json_each(e.inline_json,'$.agent_binding.typed_resource_bindings') workspace \
            WHERE json_extract(workspace.value,'$.resource_kind')='workspace' \
            AND substr(json_extract(workspace.value,'$.resource_id'),1,19)='selected-workspace-' \
            AND json_extract(workspace.value,'$.owner_id')={owner} \
            AND json_type(workspace.value,'$.typed_parameters.workspace_location') IS NULL \
            AND (substr({root},1,1)='/' OR (substr({root},1,1) GLOB '[A-Za-z]' AND substr({root},2,2)=':/')) \
            AND instr('/'||{root}||'/','/./')=0 AND instr('/'||{root}||'/','/../')=0 \
            AND substr({root},-54)='/app-hosting/apps/'||{app}) THEN {app} ELSE NULL END \
        FROM agent_events e WHERE e.session_id={alias}.agent_session_id AND e.seq=1 \
        AND e.kind='session/opening' AND e.kind_version=1)", uuidv7_sql(explicit), uuidv7_sql(&app))
}
