//! SQLite-backed control-plane store for the current Nomi-core composition.
//!
//! Fresh-v4 has its own normalized AgentPlatform store.  The current product
//! still runs on the v3 Nomi Conversation graph, so it must not open the
//! Fresh-v4 session database or reuse its Remote table shape.  This adapter
//! persists the canonical preset/revision/snapshot/binding facts in the
//! Nomi-core tables added by migration 060, while keeping the public
//! `AgentControlPlane` contract unchanged.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use axum::http::StatusCode;
use nomifun_agent_contracts::{
    AgentBindingValue, AgentPreset, AgentPresetId, AgentPresetRevision, AgentPresetSource,
    PresetRevisionRef, RemoteBinding, RemoteBindingId, ResolvedSnapshotEnvelope, UserId,
    canonical_json_bytes, digest_payload,
};
use nomifun_agent_control_plane::{
    AgentBindingTarget, ControlPlaneError, ControlPlaneStore, StoredAgentBinding, StoredPreset,
};
use nomifun_common::UserId as CommonUserId;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqlitePool, Transaction};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[derive(Clone)]
pub(crate) struct NomiCoreControlPlaneStore {
    pool: SqlitePool,
}

impl NomiCoreControlPlaneStore {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn wire<T: Serialize>(value: &T) -> Result<String, ControlPlaneError> {
    String::from_utf8(
        canonical_json_bytes(value)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?,
    )
    .map_err(|error| ControlPlaneError::Wire(error.to_string()))
}

fn decode<T: for<'de> Deserialize<'de>>(
    value: &str,
    subject: &str,
) -> Result<T, ControlPlaneError> {
    serde_json::from_str(value)
        .map_err(|error| ControlPlaneError::Wire(format!("{subject} is invalid JSON: {error}")))
}

fn sql(error: sqlx::Error) -> ControlPlaneError {
    ControlPlaneError::Wire(error.to_string())
}

fn conflict(message: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "PRESET_REVISION_DIGEST_MISMATCH",
        StatusCode::CONFLICT,
        message,
    )
}

fn not_found(subject: &str) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "PRESET_REVISION_DIGEST_MISMATCH",
        StatusCode::NOT_FOUND,
        format!("{subject} was not found"),
    )
}

fn internal(message: impl Into<String>) -> ControlPlaneError {
    ControlPlaneError::canonical(
        "PRESET_REVISION_SAVE_FAILED",
        StatusCode::INTERNAL_SERVER_ERROR,
        message,
    )
}

fn i64_from_u64(value: u64, field: &str) -> Result<i64, ControlPlaneError> {
    i64::try_from(value)
        .map_err(|_| ControlPlaneError::Wire(format!("{field} exceeds SQLite i64 range")))
}

fn u64_from_i64(value: i64, field: &str) -> Result<u64, ControlPlaneError> {
    u64::try_from(value)
        .map_err(|_| ControlPlaneError::Wire(format!("{field} is negative in SQLite")))
}

fn parse_owner(value: &str) -> Result<UserId, ControlPlaneError> {
    CommonUserId::parse(value.to_owned())
        .map(|owner| UserId::from(owner.into_string()))
        .map_err(|error| ControlPlaneError::Wire(format!("invalid persisted owner: {error}")))
}

fn parse_source(value: &str) -> Result<AgentPresetSource, ControlPlaneError> {
    match value {
        "user" => Ok(AgentPresetSource::User),
        "official" => Ok(AgentPresetSource::Official),
        other => Err(ControlPlaneError::Wire(format!(
            "unsupported persisted AgentPreset source {other:?}"
        ))),
    }
}

async fn current_revision_ref_tx(
    tx: &mut Transaction<'_, Sqlite>,
    preset_id: &AgentPresetId,
) -> Result<Option<PresetRevisionRef>, ControlPlaneError> {
    let revision: Option<i64> = sqlx::query_scalar(
        "SELECT current_revision FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(preset_id.as_ref())
    .fetch_optional(&mut **tx)
    .await
    .map_err(sql)?
    .flatten();
    let Some(revision) = revision else {
        return Ok(None);
    };
    let digest: String = sqlx::query_scalar(
        "SELECT revision_digest FROM nomi_agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = ?",
    )
    .bind(preset_id.as_ref())
    .bind(revision)
    .fetch_one(&mut **tx)
    .await
    .map_err(sql)?;
    Ok(Some(PresetRevisionRef {
        preset_id: preset_id.clone(),
        revision: u64_from_i64(revision, "current_revision")?,
        revision_digest: digest.into(),
    }))
}

async fn load_preset(
    pool: &SqlitePool,
    preset_id: &AgentPresetId,
) -> Result<Option<StoredPreset>, ControlPlaneError> {
    let row: Option<(String, String, String, String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT preset_id, owner_user_id, source_kind, display_name, description, current_revision \
         FROM nomi_agent_presets WHERE preset_id = ?",
    )
    .bind(preset_id.as_ref())
    .fetch_optional(pool)
    .await
    .map_err(sql)?;
    let Some((id, owner, source, display_name, description, current_revision)) = row else {
        return Ok(None);
    };
    let preset_id = AgentPresetId::from(id);
    let current_stable_revision = match current_revision {
        Some(revision) => {
            let digest: String = sqlx::query_scalar(
                "SELECT revision_digest FROM nomi_agent_preset_revisions \
                 WHERE preset_id = ? AND revision_no = ?",
            )
            .bind(preset_id.as_ref())
            .bind(revision)
            .fetch_one(pool)
            .await
            .map_err(sql)?;
            Some(PresetRevisionRef {
                preset_id: preset_id.clone(),
                revision: u64_from_i64(revision, "current_revision")?,
                revision_digest: digest.into(),
            })
        }
        None => None,
    };
    Ok(Some(StoredPreset {
        preset: AgentPreset {
            preset_id,
            owner_user_id: Some(parse_owner(&owner)?),
            source: parse_source(&source)?,
            display_name,
            description,
            current_stable_revision,
        },
    }))
}

async fn load_revision(
    pool: &SqlitePool,
    preset_id: &AgentPresetId,
    revision_no: u64,
) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
    let row: Option<(String, i64, String, String, String, i64, String, String, String)> =
        sqlx::query_as(
            "SELECT revision_id, revision_no, editor_document_json, revision_digest, \
                    created_by, created_at, reason, snapshot_json, contribution_locks_json \
             FROM nomi_agent_preset_revisions \
             WHERE preset_id = ? AND revision_no = ?",
        )
        .bind(preset_id.as_ref())
        .bind(i64_from_u64(revision_no, "revision")?)
        .fetch_optional(pool)
        .await
        .map_err(sql)?;
    let Some((
        _revision_id,
        persisted_no,
        document,
        digest,
        created_by,
        created_at,
        reason,
        _snapshot,
        contribution_locks_json,
    )) =
        row
    else {
        return Ok(None);
    };
    let revision = AgentPresetRevision {
        reference: PresetRevisionRef {
            preset_id: preset_id.clone(),
            revision: u64_from_i64(persisted_no, "revision_no")?,
            revision_digest: digest.into(),
        },
        payload: decode(&document, "AgentPreset revision document")?,
        contribution_locks: decode(
            &contribution_locks_json,
            "AgentPreset contribution locks",
        )?,
        created_by: parse_owner(&created_by)?,
        created_at_ms: created_at,
        reason: (!reason.is_empty()).then_some(reason),
    };
    revision.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::INTERNAL_SERVER_ERROR,
            violation.message,
        )
    })?;
    Ok(Some(revision))
}

async fn load_snapshot(
    pool: &SqlitePool,
    reference: &PresetRevisionRef,
) -> Result<Option<ResolvedSnapshotEnvelope>, ControlPlaneError> {
    let row: Option<String> = sqlx::query_scalar(
        "SELECT snapshot_json FROM nomi_agent_preset_revisions \
         WHERE preset_id = ? AND revision_no = ? AND revision_digest = ?",
    )
    .bind(reference.preset_id.as_ref())
    .bind(i64_from_u64(reference.revision, "revision")?)
    .bind(reference.revision_digest.as_ref())
    .fetch_optional(pool)
    .await
    .map_err(sql)?;
    let Some(value) = row else {
        return Ok(None);
    };
    let snapshot: ResolvedSnapshotEnvelope = decode(&value, "ResolvedSnapshot")?;
    snapshot.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::INTERNAL_SERVER_ERROR,
            violation.message,
        )
    })?;
    if snapshot.content.preset_revision_ref != *reference {
        return Err(internal(
            "persisted ResolvedSnapshot references a different Preset revision",
        ));
    }
    Ok(Some(snapshot))
}

fn validate_revision_snapshot(
    preset: &StoredPreset,
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), ControlPlaneError> {
    revision.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    snapshot.validate().map_err(|violation| {
        ControlPlaneError::canonical(
            violation.code,
            StatusCode::UNPROCESSABLE_ENTITY,
            violation.message,
        )
    })?;
    if preset.preset.owner_user_id.as_ref() != Some(&revision.created_by)
        || revision.reference.preset_id != preset.preset.preset_id
        || snapshot.content.preset_revision_ref != revision.reference
        || preset.preset.current_stable_revision.as_ref() != Some(&revision.reference)
    {
        return Err(conflict(
            "Nomi-core Preset/Revision/Snapshot identities do not match",
        ));
    }
    Ok(())
}

async fn insert_preset_tx(
    tx: &mut Transaction<'_, Sqlite>,
    preset: &StoredPreset,
    created_at: i64,
) -> Result<(), ControlPlaneError> {
    let Some(owner) = preset.preset.owner_user_id.as_ref() else {
        return Err(conflict("Nomi-core user Preset requires an owner"));
    };
    if preset.preset.source != AgentPresetSource::User {
        return Err(conflict(
            "Nomi-core control-plane store only persists user Presets",
        ));
    }
    sqlx::query(
        "INSERT INTO nomi_agent_presets \
         (preset_id, owner_user_id, source_kind, display_name, description, current_revision, created_at) \
         VALUES (?, ?, 'user', ?, ?, ?, ?)",
    )
    .bind(preset.preset.preset_id.as_ref())
    .bind(owner.as_ref())
    .bind(&preset.preset.display_name)
    .bind(&preset.preset.description)
    .bind(
        preset
            .preset
            .current_stable_revision
            .as_ref()
            .map(|reference| i64_from_u64(reference.revision, "revision"))
            .transpose()?,
    )
    .bind(created_at.max(0))
    .execute(&mut **tx)
    .await
    .map_err(sql)?;
    Ok(())
}

async fn insert_revision_tx(
    tx: &mut Transaction<'_, Sqlite>,
    revision: &AgentPresetRevision,
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<(), ControlPlaneError> {
    let revision_id = revision.reference.revision_id();
    sqlx::query(
        "INSERT INTO nomi_agent_preset_revisions \
         (revision_id, preset_id, revision_no, schema_version, editor_document_json, \
          revision_digest, created_by, created_at, reason, snapshot_json, contribution_locks_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(revision_id)
    .bind(revision.reference.preset_id.as_ref())
    .bind(i64_from_u64(revision.reference.revision, "revision")?)
    .bind(revision.payload.schema_version.as_ref())
    .bind(wire(&revision.payload)?)
    .bind(revision.reference.revision_digest.as_ref())
    .bind(revision.created_by.as_ref())
    .bind(revision.created_at_ms)
    .bind(revision.reason.as_deref().unwrap_or_default())
    .bind(wire(snapshot)?)
    .bind(wire(&revision.contribution_locks)?)
    .execute(&mut **tx)
    .await
    .map_err(sql)?;
    Ok(())
}

fn remote_snapshot_placeholder(
    snapshot: &ResolvedSnapshotEnvelope,
) -> Result<String, ControlPlaneError> {
    wire(snapshot)
}

fn remote_provenance(
    binding: &RemoteBinding,
) -> Result<String, ControlPlaneError> {
    wire(&serde_json::json!({
        "source": "nomi_core_control_plane",
        "remote_binding_id": binding.remote_binding_id,
        "binding_version": binding.agent_binding.binding_version,
    }))
}

async fn remote_row(
    pool: &SqlitePool,
    binding_id: &RemoteBindingId,
) -> Result<Option<RemoteBinding>, ControlPlaneError> {
    let row: Option<(String, String, String, String)> = sqlx::query_as(
        "SELECT remote_binding_id, owner_user_id, name, agent_binding_json \
         FROM remote_bindings WHERE remote_binding_id = ?",
    )
    .bind(binding_id.as_ref())
    .fetch_optional(pool)
    .await
    .map_err(sql)?;
    row.map(|(id, owner, name, binding)| {
        Ok(RemoteBinding {
            remote_binding_id: RemoteBindingId::from(id),
            owner_user_id: parse_owner(&owner)?,
            name,
            agent_binding: decode(&binding, "Remote AgentBinding")?,
        })
    })
    .transpose()
}

#[async_trait]
impl ControlPlaneStore for NomiCoreControlPlaneStore {
    async fn list_presets(&self, owner: &UserId) -> Result<Vec<StoredPreset>, ControlPlaneError> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT preset_id FROM nomi_agent_presets WHERE owner_user_id = ? ORDER BY preset_id",
        )
        .bind(owner.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(preset) = load_preset(&self.pool, &AgentPresetId::from(id)).await? {
                result.push(preset);
            }
        }
        Ok(result)
    }

    async fn get_preset(
        &self,
        preset_id: &AgentPresetId,
    ) -> Result<Option<StoredPreset>, ControlPlaneError> {
        load_preset(&self.pool, preset_id).await
    }

    async fn insert_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        let mut tx = self.pool.begin().await.map_err(sql)?;
        insert_preset_tx(&mut tx, &preset, now_ms()).await?;
        tx.commit().await.map_err(sql)
    }

    async fn insert_preset_with_revision(
        &self,
        preset: StoredPreset,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
    ) -> Result<StoredPreset, ControlPlaneError> {
        validate_revision_snapshot(&preset, &revision, &snapshot)?;
        let mut tx = self.pool.begin().await.map_err(sql)?;
        insert_preset_tx(&mut tx, &preset, revision.created_at_ms).await?;
        insert_revision_tx(&mut tx, &revision, &snapshot).await?;
        tx.commit().await.map_err(sql)?;
        Ok(preset)
    }

    async fn update_preset(&self, preset: StoredPreset) -> Result<(), ControlPlaneError> {
        let changed = sqlx::query(
            "UPDATE nomi_agent_presets SET display_name = ?, description = ?, current_revision = ? \
             WHERE preset_id = ? AND owner_user_id = ?",
        )
        .bind(&preset.preset.display_name)
        .bind(&preset.preset.description)
        .bind(
            preset
                .preset
                .current_stable_revision
                .as_ref()
                .map(|reference| i64_from_u64(reference.revision, "revision"))
                .transpose()?,
        )
        .bind(preset.preset.preset_id.as_ref())
        .bind(
            preset
                .preset
                .owner_user_id
                .as_ref()
                .ok_or_else(|| conflict("Preset owner is missing"))?
                .as_ref(),
        )
        .execute(&self.pool)
        .await
        .map_err(sql)?;
        if changed.rows_affected() != 1 {
            return Err(not_found("AgentPreset"));
        }
        Ok(())
    }

    async fn get_revision(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        Ok(load_revision(&self.pool, &reference.preset_id, reference.revision)
            .await?
            .filter(|revision| revision.reference.revision_digest == reference.revision_digest))
    }

    async fn get_revision_number(
        &self,
        preset_id: &AgentPresetId,
        revision: u64,
    ) -> Result<Option<AgentPresetRevision>, ControlPlaneError> {
        load_revision(&self.pool, preset_id, revision).await
    }

    async fn append_revision(
        &self,
        expected_current: Option<&PresetRevisionRef>,
        revision: AgentPresetRevision,
        snapshot: ResolvedSnapshotEnvelope,
        display_name: String,
        description: Option<String>,
    ) -> Result<StoredPreset, ControlPlaneError> {
        revision.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        snapshot.validate().map_err(|violation| {
            ControlPlaneError::canonical(
                violation.code,
                StatusCode::UNPROCESSABLE_ENTITY,
                violation.message,
            )
        })?;
        if snapshot.content.preset_revision_ref != revision.reference {
            return Err(conflict(
                "ResolvedSnapshot does not bind the appended Preset revision",
            ));
        }
        let mut tx = self.pool.begin().await.map_err(sql)?;
        let current = current_revision_ref_tx(&mut tx, &revision.reference.preset_id).await?;
        if current.as_ref() != expected_current {
            return Err(conflict(
                "expected_current_revision does not match the persisted Preset",
            ));
        }
        insert_revision_tx(&mut tx, &revision, &snapshot).await?;
        let changed = sqlx::query(
            "UPDATE nomi_agent_presets SET display_name = ?, description = ?, current_revision = ? \
             WHERE preset_id = ?",
        )
        .bind(display_name)
        .bind(description)
        .bind(i64_from_u64(revision.reference.revision, "revision")?)
        .bind(revision.reference.preset_id.as_ref())
        .execute(&mut *tx)
        .await
        .map_err(sql)?;
        if changed.rows_affected() != 1 {
            return Err(not_found("AgentPreset"));
        }
        tx.commit().await.map_err(sql)?;
        load_preset(&self.pool, &revision.reference.preset_id)
            .await?
            .ok_or_else(|| internal("updated Nomi-core AgentPreset disappeared"))
    }

    async fn get_snapshot(
        &self,
        reference: &PresetRevisionRef,
    ) -> Result<Option<ResolvedSnapshotEnvelope>, ControlPlaneError> {
        load_snapshot(&self.pool, reference).await
    }

    async fn list_agent_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<StoredAgentBinding>, ControlPlaneError> {
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT target_kind, target_id, agent_binding_json FROM nomi_agent_bindings \
             WHERE owner_user_id = ? ORDER BY target_kind, target_id",
        )
        .bind(owner.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        rows.into_iter()
            .map(|(target_kind, target_id, value)| {
                Ok(StoredAgentBinding {
                    target: AgentBindingTarget {
                        target_kind,
                        target_id,
                    },
                    owner_user_id: owner.clone(),
                    value: decode(&value, "AgentBinding")?,
                })
            })
            .collect()
    }

    async fn get_agent_binding(
        &self,
        target: &AgentBindingTarget,
    ) -> Result<Option<StoredAgentBinding>, ControlPlaneError> {
        let row: Option<(String, String)> = sqlx::query_as(
            "SELECT owner_user_id, agent_binding_json FROM nomi_agent_bindings \
             WHERE target_kind = ? AND target_id = ?",
        )
        .bind(&target.target_kind)
        .bind(&target.target_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(sql)?;
        row.map(|(owner, value)| {
            Ok(StoredAgentBinding {
                target: target.clone(),
                owner_user_id: parse_owner(&owner)?,
                value: decode(&value, "AgentBinding")?,
            })
        })
        .transpose()
    }

    async fn put_agent_binding(
        &self,
        binding: StoredAgentBinding,
        expected_binding_version: Option<u64>,
    ) -> Result<StoredAgentBinding, ControlPlaneError> {
        let mut tx = self.pool.begin().await.map_err(sql)?;
        let existing: Option<(String, String)> = sqlx::query_as(
            "SELECT owner_user_id, agent_binding_json FROM nomi_agent_bindings \
             WHERE target_kind = ? AND target_id = ?",
        )
        .bind(&binding.target.target_kind)
        .bind(&binding.target.target_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql)?;
        if let Some((owner, value)) = existing {
            let current: AgentBindingValue = decode(&value, "AgentBinding")?;
            if parse_owner(&owner)? != binding.owner_user_id {
                return Err(not_found("AgentBinding"));
            }
            if expected_binding_version != Some(current.binding_version) {
                return Err(conflict("agent binding version changed"));
            }
        } else if expected_binding_version.is_some() {
            return Err(conflict(
                "agent binding does not exist at the expected version",
            ));
        }
        let preset_owner: Option<String> = sqlx::query_scalar(
            "SELECT owner_user_id FROM nomi_agent_presets WHERE preset_id = ?",
        )
        .bind(binding.value.preset_revision_ref.preset_id.as_ref())
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql)?;
        if preset_owner.as_deref() != Some(binding.owner_user_id.as_ref()) {
            return Err(not_found("AgentPreset"));
        }
        sqlx::query(
            "INSERT INTO nomi_agent_bindings \
             (target_kind, target_id, owner_user_id, agent_binding_json) VALUES (?, ?, ?, ?) \
             ON CONFLICT (target_kind, target_id) DO UPDATE SET \
               owner_user_id = excluded.owner_user_id, agent_binding_json = excluded.agent_binding_json",
        )
        .bind(&binding.target.target_kind)
        .bind(&binding.target.target_id)
        .bind(binding.owner_user_id.as_ref())
        .bind(wire(&binding.value)?)
        .execute(&mut *tx)
        .await
        .map_err(sql)?;
        tx.commit().await.map_err(sql)?;
        Ok(binding)
    }

    async fn list_remote_bindings(
        &self,
        owner: &UserId,
    ) -> Result<Vec<RemoteBinding>, ControlPlaneError> {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT remote_binding_id FROM remote_bindings WHERE owner_user_id = ? \
             ORDER BY remote_binding_id",
        )
        .bind(owner.as_ref())
        .fetch_all(&self.pool)
        .await
        .map_err(sql)?;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(binding) =
                remote_row(&self.pool, &RemoteBindingId::from(id)).await?
            {
                result.push(binding);
            }
        }
        Ok(result)
    }

    async fn get_remote_binding(
        &self,
        binding_id: &RemoteBindingId,
    ) -> Result<Option<RemoteBinding>, ControlPlaneError> {
        remote_row(&self.pool, binding_id).await
    }

    async fn insert_remote_binding(
        &self,
        binding: RemoteBinding,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        let snapshot = load_snapshot(
            &self.pool,
            &binding.agent_binding.preset_revision_ref,
        )
        .await?
        .ok_or_else(|| not_found("ResolvedSnapshot"))?;
        let digest = digest_payload(&binding.agent_binding)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO remote_bindings \
             (remote_binding_id, owner_user_id, name, agent_binding_json, nomi_snapshot_json, \
              provenance_json, agent_binding_digest, binding_version, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(binding.remote_binding_id.as_ref())
        .bind(binding.owner_user_id.as_ref())
        .bind(&binding.name)
        .bind(wire(&binding.agent_binding)?)
        .bind(remote_snapshot_placeholder(&snapshot)?)
        .bind(remote_provenance(&binding)?)
        .bind(digest.as_ref())
        .bind(i64_from_u64(
            binding.agent_binding.binding_version,
            "binding_version",
        )?)
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(sql)?;
        Ok(binding)
    }

    async fn update_remote_binding(
        &self,
        binding: RemoteBinding,
        expected_binding_version: u64,
        expected_agent_binding_digest: &str,
    ) -> Result<RemoteBinding, ControlPlaneError> {
        let existing = remote_row(&self.pool, &binding.remote_binding_id)
            .await?
            .ok_or_else(|| not_found("RemoteBinding"))?;
        if existing.owner_user_id != binding.owner_user_id {
            return Err(not_found("RemoteBinding"));
        }
        let existing_digest = digest_payload(&existing.agent_binding)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        if existing.agent_binding.binding_version != expected_binding_version
            || existing_digest.as_ref() != expected_agent_binding_digest
        {
            return Err(conflict("RemoteBinding version or digest changed"));
        }
        let snapshot = load_snapshot(
            &self.pool,
            &binding.agent_binding.preset_revision_ref,
        )
        .await?
        .ok_or_else(|| not_found("ResolvedSnapshot"))?;
        let digest = digest_payload(&binding.agent_binding)
            .map_err(|error| ControlPlaneError::Wire(error.to_string()))?;
        let changed = sqlx::query(
            "UPDATE remote_bindings SET name = ?, agent_binding_json = ?, nomi_snapshot_json = ?, \
             provenance_json = ?, agent_binding_digest = ?, binding_version = ?, updated_at = ? \
             WHERE remote_binding_id = ? AND owner_user_id = ?",
        )
        .bind(&binding.name)
        .bind(wire(&binding.agent_binding)?)
        .bind(remote_snapshot_placeholder(&snapshot)?)
        .bind(remote_provenance(&binding)?)
        .bind(digest.as_ref())
        .bind(i64_from_u64(
            binding.agent_binding.binding_version,
            "binding_version",
        )?)
        .bind(now_ms())
        .bind(binding.remote_binding_id.as_ref())
        .bind(binding.owner_user_id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(sql)?;
        if changed.rows_affected() != 1 {
            return Err(not_found("RemoteBinding"));
        }
        Ok(binding)
    }

    async fn delete_remote_binding(
        &self,
        owner: &UserId,
        binding_id: &RemoteBindingId,
    ) -> Result<(), ControlPlaneError> {
        let changed = sqlx::query(
            "DELETE FROM remote_bindings WHERE remote_binding_id = ? AND owner_user_id = ?",
        )
        .bind(binding_id.as_ref())
        .bind(owner.as_ref())
        .execute(&self.pool)
        .await
        .map_err(sql)?;
        if changed.rows_affected() != 1 {
            return Err(not_found("RemoteBinding"));
        }
        Ok(())
    }
}
