//! Catalog deletion removes mutable selections, never frozen execution facts.
use nomifun_agent_contracts::{AgentBindingValue, AgentPresetRevision, PresetRevisionRef, ResolvedSnapshotEnvelope};
use serde_json::Value;
use sqlx::{Sqlite, Transaction};
use crate::{DbError, ProviderModelCleanupPlan};
use crate::repository::{provider_model_preference_delete_action,provider_preference_delete_action,ProviderPreferenceDeleteAction,prune_missing_provider_preference};

fn invalid(message: impl Into<String>) -> DbError {DbError::Conflict(message.into())}
fn wire(value: &impl serde::Serialize) -> Result<String,DbError> {
    serde_json::to_string(value).map_err(|error| invalid(format!("serialize catalog cleanup: {error}")))
}
fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T,DbError> {
    serde_json::from_str(value).map_err(|error| invalid(format!("invalid stored catalog reference: {error}")))
}
fn revision_no(value: u64) -> Result<i64,DbError> {
    i64::try_from(value).map_err(|_| invalid("Agent revision is outside SQLite integer range"))
}

/// The single writer for a new immutable Revision, its contribution locks and
/// Snapshot. The caller owns the transaction and current-revision CAS.
pub async fn insert_agent_preset_revision_tx(
    tx: &mut Transaction<'_,Sqlite>, revision: &AgentPresetRevision, snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(),DbError> {
    revision.validate().map_err(|error| invalid(error.message))?;
    snapshot.validate().map_err(|error| invalid(error.message))?;
    if snapshot.content.preset_revision_ref != revision.reference
        || snapshot.content.model_route_refs != revision.payload.model_route_refs
        || snapshot.content.chat_route_identity != revision.chat_route_identity().map_err(|error| invalid(error.message))? {
        return Err(invalid("Snapshot does not bind the appended Agent revision"));
    }
    let id = revision.reference.revision_id();
    sqlx::query("INSERT INTO agent_preset_revisions (revision_id,preset_id,revision_no,schema_version,payload_json,revision_digest,created_by,created_at,reason) VALUES (?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(revision.reference.preset_id.as_ref()).bind(revision_no(revision.reference.revision)?)
        .bind(revision.payload.schema_version.as_ref()).bind(wire(&revision.payload)?)
        .bind(revision.reference.revision_digest.as_ref()).bind(revision.created_by.as_ref())
        .bind(revision.created_at_ms).bind(&revision.reason).execute(&mut **tx).await?;
    for lock in &revision.contribution_locks {
        sqlx::query("INSERT INTO agent_preset_contribution_locks (revision_id,contribution_id,lock_json) VALUES (?,?,?)")
            .bind(&id).bind(lock.contribution_id.as_ref()).bind(wire(lock)?).execute(&mut **tx).await?;
    }
    sqlx::query("INSERT INTO agent_runtime_snapshots (snapshot_id,snapshot_digest,content_json,envelope_json) VALUES (?,?,?,?)")
        .bind(snapshot.snapshot_ref.snapshot_id.as_ref()).bind(snapshot.snapshot_ref.snapshot_digest.as_ref())
        .bind(wire(&snapshot.content)?).bind(wire(snapshot)?).execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn revision_artifact_tx(tx: &mut Transaction<'_,Sqlite>, expected: &PresetRevisionRef)
    -> Result<AgentPresetRevision,DbError> {
    let row: Option<(String,String,String,i64,Option<String>)> = sqlx::query_as(
        "SELECT revision.payload_json,revision.revision_digest,revision.created_by,revision.created_at,revision.reason \
         FROM agent_preset_revisions revision WHERE revision.preset_id=? AND revision.revision_no=?")
        .bind(expected.preset_id.as_ref()).bind(revision_no(expected.revision)?).fetch_optional(&mut **tx).await?;
    let Some((payload,digest,creator,created_at,reason)) = row else {return Err(invalid("exact Agent revision is missing during catalog cleanup"));};
    if digest != expected.revision_digest.as_ref() {return Err(invalid("Agent source revision digest changed during catalog cleanup"));}
    let mut locks = sqlx::query_scalar::<_,String>("SELECT lock_json FROM agent_preset_contribution_locks WHERE revision_id=? ORDER BY contribution_id")
        .bind(expected.revision_id()).fetch_all(&mut **tx).await?.into_iter()
        .map(|value| decode(&value)).collect::<Result<Vec<_>,_>>()?;
    locks.sort();
    let revision = AgentPresetRevision {reference:expected.clone(),payload:decode(&payload)?,contribution_locks:locks,
        created_by:creator.into(),created_at_ms:created_at,reason};
    revision.validate().map_err(|error| invalid(error.message))?;
    Ok(revision)
}

async fn source_artifacts(tx: &mut Transaction<'_,Sqlite>, expected: &PresetRevisionRef)
    -> Result<(AgentPresetRevision,ResolvedSnapshotEnvelope),DbError> {
    let current: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_presets WHERE preset_id=? AND current_stable_revision=? AND retired_at_ms IS NULL)")
        .bind(expected.preset_id.as_ref()).bind(revision_no(expected.revision)?).fetch_one(&mut **tx).await?;
    if !current {return Err(invalid("Agent current revision changed during catalog cleanup"));}
    let revision = revision_artifact_tx(tx,expected).await?;
    let envelope: Option<String> = sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE json_extract(content_json,'$.preset_revision_ref.preset_id')=? AND json_extract(content_json,'$.preset_revision_ref.revision')=? AND json_extract(content_json,'$.preset_revision_ref.revision_digest')=?")
        .bind(expected.preset_id.as_ref()).bind(revision_no(expected.revision)?).bind(expected.revision_digest.as_ref())
        .fetch_optional(&mut **tx).await?;
    let snapshot: ResolvedSnapshotEnvelope = decode(&envelope.ok_or_else(|| invalid("Agent source Snapshot is missing during catalog cleanup"))?)?;
    snapshot.validate().map_err(|error| invalid(error.message))?;
    if snapshot.content.preset_revision_ref != *expected {return Err(invalid("Agent source Snapshot revision mismatch"));}
    Ok((revision,snapshot))
}

/// Existing Cron jobs execute the exact current Session binding, regardless of
/// any authoring model selection on the job. Only the mutable schedule stops.
async fn disable_bound_session_cron_tx(
    tx: &mut Transaction<'_,Sqlite>, provider_id: &str, model: Option<&str>, now: i64,
) -> Result<(),DbError> {
    let rows: Vec<(String,String)> = sqlx::query_as(
        "SELECT job.cron_job_id,session.agent_binding_json FROM cron_jobs job \
         JOIN agent_sessions session ON session.agent_session_id=job.conversation_id \
         WHERE job.enabled=1 AND job.execution_mode='existing' AND session.state='live' \
           AND session.remote_binding_id IS NULL \
           AND json_extract(session.owner_ref_json,'$.principal_kind')='user' \
           AND json_extract(session.owner_ref_json,'$.principal_id')=job.user_id")
        .fetch_all(&mut **tx).await?;
    for (job_id,binding) in rows {
        let binding: AgentBindingValue = decode(&binding)?;
        let revision = revision_artifact_tx(tx,&binding.preset_revision_ref).await?;
        let matches = revision.payload.chat_route_records
            .get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
            .is_some_and(|route| route.primary.provider_id==provider_id
                && model.is_none_or(|model| route.primary.model==model));
        if matches {
            sqlx::query("UPDATE cron_jobs SET enabled=0,schedule_revision=schedule_revision+1,next_run_at=NULL,updated_at=MAX(updated_at,?) WHERE cron_job_id=?")
                .bind(now).bind(job_id).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

/// Launch-mode Cron authority is the frozen binding, independently of its
/// editable model selection. Catalog deletion never edits immutable artifacts.
async fn disable_launch_binding_cron_tx(tx:&mut Transaction<'_,Sqlite>,provider_id:&str,model:Option<&str>,now:i64)->Result<(),DbError> {
    let rows:Vec<(String,String)>=sqlx::query_as("SELECT cron_job_id,agent_binding FROM cron_jobs WHERE enabled=1 AND conversation_id IS NULL AND agent_binding IS NOT NULL")
        .fetch_all(&mut **tx).await?;
    for (job,raw) in rows {
        let binding:AgentBindingValue=decode(&raw)?;
        let revision=revision_artifact_tx(tx,&binding.preset_revision_ref).await?;
        if revision.payload.chat_route_records.get(nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT)
            .is_some_and(|route|route.primary.provider_id==provider_id && model.is_none_or(|model|route.primary.model==model)) {
            sqlx::query("UPDATE cron_jobs SET enabled=0,schedule_revision=schedule_revision+1,next_run_at=NULL,configuration_error='CRON_MODEL_RESELECTION_REQUIRED',updated_at=MAX(updated_at,?) WHERE cron_job_id=?")
                .bind(now).bind(job).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

pub(crate) async fn apply_cleanup_plan_tx(
    tx: &mut Transaction<'_,Sqlite>, provider_id: &str, model: Option<&str>, plan: &ProviderModelCleanupPlan,
) -> Result<(),DbError> {
    for patch in &plan.agent_revisions {
        let (source,source_snapshot) = source_artifacts(tx,&patch.expected_current).await?;
        nomifun_agent_contracts::validate_catalog_model_detachment(&source,&source_snapshot,&patch.revision,provider_id,model)
            .map_err(|error| invalid(error.message))?;
        let mut expected_content = source_snapshot.content.clone();
        expected_content.preset_revision_ref = patch.revision.reference.clone();
        expected_content.model_route_refs = patch.revision.payload.model_route_refs.clone();
        expected_content.chat_route_identity = patch.snapshot.content.chat_route_identity.clone();
        // Kernel recompiles this digest from the detached model identity. The
        // remaining frozen profile contract still must match byte for byte.
        expected_content.compiled_runtime_profile_digest = patch.snapshot.content.compiled_runtime_profile_digest.clone();
        if patch.snapshot.content != expected_content || patch.snapshot.actor != source_snapshot.actor
            || patch.snapshot.scene != source_snapshot.scene || patch.snapshot.surface != source_snapshot.surface
            || patch.snapshot.audience != source_snapshot.audience
            || patch.snapshot.availability_evidence_revision != source_snapshot.availability_evidence_revision {
            return Err(invalid("catalog cleanup must preserve the Agent's non-model frozen Snapshot contract"));
        }
        insert_agent_preset_revision_tx(tx,&patch.revision,&patch.snapshot).await?;
        let updated = sqlx::query("UPDATE agent_presets SET current_stable_revision=? WHERE preset_id=? AND current_stable_revision=? AND retired_at_ms IS NULL")
            .bind(revision_no(patch.revision.reference.revision)?).bind(patch.expected_current.preset_id.as_ref())
            .bind(revision_no(patch.expected_current.revision)?).execute(&mut **tx).await?;
        if updated.rows_affected()!=1 {return Err(invalid("Agent current revision changed during catalog cleanup"));}
    }
    // A raw repository caller cannot silently leave a mutable authoring
    // default behind. Historical revisions and hidden Session variants are
    // deliberately outside this completeness check.
    let missing_agent_patch: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM agent_presets preset JOIN agent_preset_revisions revision \
         ON revision.preset_id=preset.preset_id AND revision.revision_no=preset.current_stable_revision, \
         json_tree(revision.payload_json,'$.chat_route_records') route \
         WHERE preset.retired_at_ms IS NULL AND COALESCE(json_extract(preset.display_json,'$.session_only'),0)=0 \
           AND route.type='object' AND json_extract(route.value,'$.provider_id')=?1 \
           AND (?2 IS NULL OR json_extract(route.value,'$.model')=?2))")
        .bind(provider_id).bind(model).fetch_one(&mut **tx).await?;
    if missing_agent_patch {return Err(invalid("catalog cleanup plan is missing a current Agent model detachment"));}
    for cleanup in &plan.projects {
        let updated = sqlx::query("UPDATE creative_studio_projects SET document_json=?,node_count=?,connection_count=?,revision=revision+1,updated_at=? WHERE project_id=? AND revision=?")
            .bind(&cleanup.document_json).bind(cleanup.node_count).bind(cleanup.connection_count).bind(cleanup.updated_at)
            .bind(&cleanup.project_id).bind(cleanup.expected_revision).execute(&mut **tx).await?;
        if updated.rows_affected()!=1 {return Err(invalid(format!("creative studio project '{}' changed during provider model cleanup; expected revision {}",cleanup.project_id,cleanup.expected_revision)));}
    }
    for cleanup in &plan.templates {
        let row = &cleanup.replacement;
        if row.template_id!=cleanup.template_id || row.revision!=cleanup.expected_revision+1 {
            return Err(invalid("creative studio template cleanup must preserve its ID and increment revision once"));
        }
        let updated = sqlx::query("UPDATE creative_studio_templates SET revision=?,name=?,description=?,category=?,visibility=?,definition_json=?,updated_at=? WHERE template_id=? AND revision=?")
            .bind(row.revision).bind(&row.name).bind(&row.description).bind(&row.category).bind(&row.visibility)
            .bind(&row.definition_json).bind(row.updated_at).bind(&cleanup.template_id).bind(cleanup.expected_revision)
            .execute(&mut **tx).await?;
        if updated.rows_affected()!=1 {return Err(invalid(format!("creative studio template '{}' changed during provider model cleanup; expected revision {}",cleanup.template_id,cleanup.expected_revision)));}
    }
    cleanup_mutable_references_tx(tx,provider_id,model).await
}

fn selection_matches(value: &Value, provider_id: &str, model: Option<&str>) -> bool {
    value.get("provider_id").and_then(Value::as_str)==Some(provider_id)
        && model.is_none_or(|model| value.get("model").and_then(Value::as_str)==Some(model))
}

/// New mutable selections serialize with catalog deletion through the same
/// SQLite writer. Existing frozen execution facts do not use this writer.
pub(crate) async fn lock_provider_model_choice_tx(
    tx: &mut Transaction<'_,Sqlite>, provider_id: &str, model: Option<&str>,
) -> Result<(),DbError> {
    let parent=sqlx::query("UPDATE providers SET config_revision=config_revision WHERE provider_id=?")
        .bind(provider_id).execute(&mut **tx).await?;
    if parent.rows_affected()!=1 {return Err(invalid("selected provider no longer exists; select a model again"));}
    if let Some(model)=model {
        let exists:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM provider_models WHERE provider_id=? AND model=?)")
            .bind(provider_id).bind(model).fetch_one(&mut **tx).await?;
        if !exists {return Err(invalid("selected provider model no longer exists; select a model again"));}
    }
    Ok(())
}

async fn cleanup_mutable_references_tx(tx: &mut Transaction<'_,Sqlite>, provider_id: &str, model: Option<&str>) -> Result<(),DbError> {
    let now = nomifun_common::now_ms();
    let malformed_cron: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE model_selection IS NOT NULL AND NOT json_valid(model_selection))")
        .fetch_one(&mut **tx).await?;
    if malformed_cron {return Err(invalid("invalid cron model_selection JSON during catalog cleanup"));}
    let rows: Vec<(String,String)> = sqlx::query_as("SELECT key,value FROM client_preferences").fetch_all(&mut **tx).await?;
    for (key,value) in rows {
        let action = if model.is_none() {provider_preference_delete_action(&key,&value,provider_id)?}
            else {provider_model_preference_delete_action(&key,&value,provider_id,model)?};
        match action {
            ProviderPreferenceDeleteAction::Keep => {}
            ProviderPreferenceDeleteAction::Delete => {sqlx::query("DELETE FROM client_preferences WHERE key=?").bind(&key).execute(&mut **tx).await?;}
            ProviderPreferenceDeleteAction::Update(value) => {
                let value = if model.is_none() {prune_missing_provider_preference(&key,value,tx).await?} else {value};
                sqlx::query("UPDATE client_preferences SET value=?,updated_at=MAX(updated_at,?) WHERE key=?")
                    .bind(value).bind(now).bind(&key).execute(&mut **tx).await?;
            }
        }
        if key.starts_with("agent_session.idmm.") {
            let mut state: Value = decode(&value)?;
            if !state.pointer("/config/bypass_model").is_some_and(|selection| selection_matches(selection,provider_id,model)) {continue;}
            let config = state.get_mut("config").and_then(Value::as_object_mut).ok_or_else(|| invalid("invalid IDMM configuration"))?;
            config.insert("bypass_model".into(),serde_json::json!({}));
            if config.get("mode").and_then(Value::as_str)==Some("rule_plus_model") {config.insert("mode".into(),serde_json::json!("rule_only"));}
            if let Some(revision) = state.get("revision").and_then(Value::as_u64) {state["revision"]=serde_json::json!(revision.saturating_add(1));}
            sqlx::query("UPDATE client_preferences SET value=?,updated_at=MAX(updated_at,?) WHERE key=?")
                .bind(wire(&state)?).bind(now).bind(&key).execute(&mut **tx).await?;
        }
    }
    let rows: Vec<(String,String)> = sqlx::query_as("SELECT terminal_id,idmm FROM terminal_sessions WHERE idmm IS NOT NULL AND json_valid(idmm)").fetch_all(&mut **tx).await?;
    for (id,raw) in rows {
        let mut value: Value = decode(&raw)?;
        let mut changed = false;
        for watch in ["fault_watch","decision_watch"] {
            if value.get(watch).and_then(|value| value.get("bypass_model")).is_some_and(|value| selection_matches(value,provider_id,model)) {
                value.get_mut(watch).and_then(Value::as_object_mut).expect("selected watch object").remove("bypass_model");changed=true;
            }
        }
        if changed {sqlx::query("UPDATE terminal_sessions SET idmm=?,updated_at=MAX(updated_at,?) WHERE terminal_id=?")
            .bind(wire(&value)?).bind(now).bind(id).execute(&mut **tx).await?;}
    }
    sqlx::query("UPDATE cs_agents SET provider_id=NULL,model=NULL,updated_at=MAX(updated_at,?) WHERE provider_id=? AND (? IS NULL OR model=?)")
        .bind(now).bind(provider_id).bind(model).bind(model).execute(&mut **tx).await?;
    sqlx::query("UPDATE agent_execution_templates SET updated_at=MAX(updated_at,?) WHERE execution_template_id IN (SELECT template_id FROM agent_execution_template_participants WHERE provider_id=? AND (? IS NULL OR model=?))")
        .bind(now).bind(provider_id).bind(model).bind(model).execute(&mut **tx).await?;
    sqlx::query("UPDATE agent_execution_template_participants SET provider_id=NULL,model=NULL,updated_at=MAX(updated_at,?) WHERE provider_id=? AND (? IS NULL OR model=?)")
        .bind(now).bind(provider_id).bind(model).bind(model).execute(&mut **tx).await?;
    sqlx::query("UPDATE cron_jobs SET enabled=0,schedule_revision=schedule_revision+1,next_run_at=NULL,model_selection=NULL,configuration_error='CRON_MODEL_RESELECTION_REQUIRED',updated_at=MAX(updated_at,?) WHERE json_valid(model_selection) AND json_extract(model_selection,'$.provider_id')=? AND (? IS NULL OR json_extract(model_selection,'$.model')=?)")
        .bind(now).bind(provider_id).bind(model).bind(model).execute(&mut **tx).await?;
    disable_bound_session_cron_tx(tx,provider_id,model,now).await?;
    disable_launch_binding_cron_tx(tx,provider_id,model,now).await?;
    Ok(())
}
