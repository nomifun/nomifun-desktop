//! Pure current-generation Cron schema normalization. No compilation, catalog
//! lookup, Session mutation, replay, or new authority is available here.
use crate::DbError;
use nomifun_agent_contracts::{AgentBindingValue, ResolvedSnapshotEnvelope};
use sqlx::{Connection, SqliteConnection};

const OPTIONAL_TARGETS: &[&str] = &[
    "knowledge_base",
    "channel",
    "robot",
    "canvas",
    "ssh_host",
    "browser",
    "computer",
];

/// The saved descriptors are the original permission ceiling. A kind alone,
/// a path, or an idle Session never proves the concrete action grants.
pub(crate) fn complete_resources(
    binding: &AgentBindingValue,
    snapshot: &ResolvedSnapshotEnvelope,
    owner: &str,
) -> bool {
    if binding.binding_version == 0
        || binding.typed_resource_bindings.iter().any(|resource| {
            resource.owner_id != owner
                || nomifun_agent_contracts::resource_definition_id(resource)
                    .ok()
                    .as_ref()
                    != Some(&resource.binding_id)
        })
    {
        return false;
    }
    for kind in &snapshot.content.required_resource_kinds {
        let count = binding
            .typed_resource_bindings
            .iter()
            .filter(|resource| &resource.resource_kind == kind)
            .count();
        if count == 0 && !OPTIONAL_TARGETS.contains(&kind.as_ref()) {
            return false;
        }
        if count > 1 && !matches!(kind.as_ref(), "knowledge_base" | "mcp_server") {
            return false;
        }
    }
    for capability in snapshot.content.contributions() {
        for action in capability
            .actions
            .iter()
            .filter(|action| capability.action_allowlist.contains(&action.action_id))
        {
            let Some(requirements) = action.required_resource_operations.as_ref() else {
                if capability
                    .required_resource_kinds
                    .iter()
                    .any(|kind| !OPTIONAL_TARGETS.contains(&kind.as_ref()))
                {
                    return false;
                }
                continue;
            };
            for (kind, operations) in requirements {
                let bound = binding
                    .typed_resource_bindings
                    .iter()
                    .filter(|resource| &resource.resource_kind == kind)
                    .collect::<Vec<_>>();
                if bound.is_empty() && OPTIONAL_TARGETS.contains(&kind.as_ref()) {
                    continue;
                }
                // Knowledge/MCP mounts are narrowed per target. Their actual
                // advertised operations are filtered by the product catalogs;
                // the global Action ceiling must not upgrade or reject a
                // legitimate read-only mount during schema normalization.
                if matches!(kind.as_ref(), "knowledge_base" | "mcp_server") {
                    continue;
                }
                if bound.len() != 1 || !operations.is_subset(&bound[0].operations) {
                    return false;
                }
            }
        }
    }
    true
}

/// The migration leaves a resumable marker until the same database opening
/// proves the exact immutable artifacts. A crash after SQL commit cannot make
/// an unverified candidate eligible for scheduler startup.
pub(crate) async fn finish_pending(conn: &mut SqliteConnection) -> Result<(), DbError> {
    let mut tx = conn.begin().await?;
    let rows:Vec<(String,String,String)>=sqlx::query_as("SELECT cron_job_id,user_id,agent_binding FROM cron_jobs WHERE configuration_error='CRON_BINDING_VALIDATION_PENDING'")
        .fetch_all(&mut *tx).await?;
    for (job, owner, raw) in rows {
        let binding: AgentBindingValue = serde_json::from_str(&raw).map_err(|error| {
            DbError::Init(format!(
                "invalid current Cron binding during normalization: {error}"
            ))
        })?;
        let revision =
            crate::provider_cleanup::revision_artifact_tx(&mut tx, &binding.preset_revision_ref)
                .await?;
        let envelope:Option<String>=sqlx::query_scalar("SELECT envelope_json FROM agent_runtime_snapshots WHERE snapshot_id=? AND snapshot_digest=?")
            .bind(binding.resolved_snapshot_ref.snapshot_id.as_ref()).bind(binding.resolved_snapshot_ref.snapshot_digest.as_ref()).fetch_optional(&mut *tx).await?;
        let snapshot: ResolvedSnapshotEnvelope =
            serde_json::from_str(&envelope.ok_or_else(|| {
                DbError::Init("Cron exact normalization Snapshot is missing".into())
            })?)
            .map_err(|error| DbError::Init(error.to_string()))?;
        snapshot
            .validate()
            .map_err(|error| DbError::Init(error.message))?;
        if revision.created_by.as_ref() != owner
            || snapshot.actor.principal_kind != "user"
            || snapshot.actor.principal_id != owner
            || snapshot.content.preset_revision_ref != binding.preset_revision_ref
            || snapshot.snapshot_ref != binding.resolved_snapshot_ref
        {
            return Err(DbError::Init(
                "Cron normalization immutable artifact identity or owner differs".into(),
            ));
        }
        if complete_resources(&binding, &snapshot, &owner) {
            sqlx::query("UPDATE cron_jobs SET configuration_error=NULL WHERE cron_job_id=? AND configuration_error='CRON_BINDING_VALIDATION_PENDING'")
                .bind(job).execute(&mut *tx).await?;
        } else {
            sqlx::query("UPDATE cron_jobs SET enabled=0,next_run_at=NULL,schedule_revision=schedule_revision+1,agent_binding=NULL,configuration_error='CRON_CONFIGURATION_RESELECTION_REQUIRED' WHERE cron_job_id=? AND configuration_error='CRON_BINDING_VALIDATION_PENDING'")
                .bind(job).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}
