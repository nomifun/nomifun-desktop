use serde_json::Value;
use sqlx::{Sqlite, Transaction};

use crate::error::DbError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AgentPresetLineageAuthority {
    Absent,
    LivePreset,
    FrozenSnapshot,
}

pub(crate) fn validate_agent_preset_lineage(
    preset_id: Option<&str>,
    preset_revision: Option<i64>,
    agent_snapshot: Option<&str>,
    label: &str,
) -> Result<AgentPresetLineageAuthority, DbError> {
    match (preset_id, preset_revision, agent_snapshot) {
        (None, None, None) => Ok(AgentPresetLineageAuthority::Absent),
        (Some(preset_id), Some(preset_revision), snapshot) if preset_revision > 0 => {
            nomifun_common::validate_uuidv7(preset_id).map_err(|error| {
                DbError::Conflict(format!(
                    "{label} preset_id is not a canonical UUIDv7: {error}"
                ))
            })?;
            let Some(snapshot) = snapshot else {
                return Ok(AgentPresetLineageAuthority::LivePreset);
            };
            let snapshot: Value = serde_json::from_str(snapshot).map_err(|error| {
                DbError::Conflict(format!("{label} agent_snapshot must be valid JSON: {error}"))
            })?;
            let object = snapshot.as_object().ok_or_else(|| {
                DbError::Conflict(format!("{label} agent_snapshot must be a JSON object"))
            })?;
            if object.get("preset_id").and_then(Value::as_str) != Some(preset_id)
                || object.get("preset_revision").and_then(Value::as_i64)
                    != Some(preset_revision)
            {
                return Err(DbError::Conflict(format!(
                    "{label} AgentPreset lineage and agent_snapshot are inconsistent"
                )));
            }
            Ok(AgentPresetLineageAuthority::FrozenSnapshot)
        }
        _ => Err(DbError::Conflict(format!(
            "{label} AgentPreset lineage must be absent or include preset_id and a positive preset_revision"
        ))),
    }
}

pub(crate) async fn validate_and_lock_agent_preset_lineage(
    tx: &mut Transaction<'_, Sqlite>,
    preset_id: Option<&str>,
    preset_revision: Option<i64>,
    agent_snapshot: Option<&str>,
    label: &str,
) -> Result<AgentPresetLineageAuthority, DbError> {
    let authority =
        validate_agent_preset_lineage(preset_id, preset_revision, agent_snapshot, label)?;
    if authority != AgentPresetLineageAuthority::LivePreset {
        return Ok(authority);
    }

    let preset_id = preset_id.expect("live AgentPreset authority requires preset_id");
    let locked = sqlx::query(
        "UPDATE nomi_agent_presets SET display_name = display_name WHERE preset_id = ?",
    )
    .bind(preset_id)
    .execute(&mut **tx)
    .await?;
    if locked.rows_affected() == 0 {
        return Err(DbError::Conflict(format!(
            "{label} AgentPreset '{preset_id}' does not exist"
        )));
    }
    Ok(authority)
}
