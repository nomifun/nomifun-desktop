//! In-session companion summon lifecycle (spec §设计 B, plan Task 1).
//!
//! "召唤伙伴" loads one companion's skills and hand-picked memories (read-only)
//! into an ordinary work conversation. The durable marker is
//! `conversation.extra.summon` ([`SummonConfig`]); the nomi factory reads it at
//! build time, so any change here must recycle the cached runtime for the
//! change to take effect on the next message — the same contract as a
//! knowledge-binding change (`service.rs::apply_knowledge_mounts`).
//!
//! Lifecycle invariant (spec §B4): summon set/adjust/clear requires an idle
//! conversation — no process-local active turn and no durable `running`
//! generation — otherwise 409 Conflict. Both checks run under the runtime
//! preparation gate so a send admitted concurrently cannot interleave a build
//! between the idle check and the extra write.
//!
//! Kept in a child module of `service` (separate file) so it can use the
//! service's private repos without widening their visibility.

use nomifun_api_types::SummonConfig;
use nomifun_common::{
    AgentKillReason, AppError, CompanionId, CompanionMemoryId, ConversationSource, now_ms,
};
use nomifun_db::ConversationRowUpdate;
use nomifun_db::models::ConversationRow;
use serde::Deserialize;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::convert::string_to_enum;
use crate::service::{ConversationService, parse_conv_id};

/// `PUT /api/conversations/{id}/summon` body. `summoned_at` is deliberately
/// absent — the server stamps it so clients cannot forge summon age.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetSummonRequest {
    pub companion_id: String,
    #[serde(default)]
    pub memory_ids: Vec<String>,
    #[serde(default)]
    pub skill_exclusions: Vec<String>,
}

fn validated_config(req: SetSummonRequest) -> Result<SummonConfig, AppError> {
    CompanionId::parse(req.companion_id.as_str())
        .map_err(|error| AppError::BadRequest(format!("invalid summon companion_id: {error}")))?;
    let mut memory_ids = Vec::with_capacity(req.memory_ids.len());
    for id in req.memory_ids {
        CompanionMemoryId::parse(id.as_str())
            .map_err(|error| AppError::BadRequest(format!("invalid summon memory id: {error}")))?;
        if !memory_ids.contains(&id) {
            memory_ids.push(id);
        }
    }
    let mut skill_exclusions = Vec::with_capacity(req.skill_exclusions.len());
    for name in req.skill_exclusions {
        let name = name.trim().to_owned();
        if !name.is_empty() && !skill_exclusions.contains(&name) {
            skill_exclusions.push(name);
        }
    }
    Ok(SummonConfig {
        companion_id: req.companion_id,
        memory_ids,
        skill_exclusions,
        summoned_at: now_ms(),
    })
}

fn parse_extra(row: &ConversationRow) -> Result<serde_json::Value, AppError> {
    let extra: serde_json::Value = serde_json::from_str(&row.extra).map_err(|error| {
        AppError::Internal(format!(
            "Conversation {} has invalid extra JSON: {error}",
            row.conversation_id
        ))
    })?;
    if !extra.is_object() {
        return Err(AppError::Internal(format!(
            "Conversation {} extra must be a JSON object",
            row.conversation_id
        )));
    }
    Ok(extra)
}

fn row_source(row: &ConversationRow) -> Option<ConversationSource> {
    row.source
        .as_deref()
        .and_then(|source| string_to_enum(source).ok())
}

impl ConversationService {
    /// Summon a companion into (or adjust the summon of) an idle work
    /// conversation. Persists the server-stamped [`SummonConfig`] at
    /// `extra.summon` and recycles the cached runtime so the next message
    /// rebuilds with the companion's skills, memories and read-only tools.
    pub async fn set_summon(
        &self,
        user_id: &str,
        conversation_id: &str,
        req: SetSummonRequest,
    ) -> Result<SummonConfig, AppError> {
        let config = validated_config(req)?;
        let (row, mut extra) = self
            .begin_idle_summon_mutation(user_id, conversation_id)
            .await?;
        if extra
            .get("companion_session")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return Err(AppError::BadRequest(
                "companion conversations cannot summon a companion (persona boundary)".into(),
            ));
        }
        extra["summon"] = serde_json::to_value(&config)
            .map_err(|error| AppError::Internal(format!("Failed to serialize summon: {error}")))?;
        self.commit_summon_extra(user_id, &row, extra, "companion summon change")
            .await?;
        Ok(config)
    }

    /// Remove the summon marker from an idle conversation and recycle the
    /// cached runtime. Idempotent: clearing an un-summoned conversation is a
    /// no-op success that leaves the runtime alone.
    pub async fn clear_summon(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<(), AppError> {
        let (row, mut extra) = self
            .begin_idle_summon_mutation(user_id, conversation_id)
            .await?;
        let Some(map) = extra.as_object_mut() else {
            return Err(AppError::Internal(format!(
                "Conversation {conversation_id} extra must be a JSON object"
            )));
        };
        if map.remove("summon").is_none() {
            return Ok(());
        }
        self.commit_summon_extra(user_id, &row, extra, "companion summon release")
            .await
    }

    /// Shared admission for summon mutations: ownership, execution-attempt
    /// retention, preparation-gate serialization and the idle invariant.
    /// Returns the re-validated row plus its parsed extra. The preparation
    /// gate is deliberately released before the extra write: the idle checks
    /// already prove no turn owns the runtime, and both mutation paths finish
    /// with a result-bearing teardown that any interleaved build would follow.
    async fn begin_idle_summon_mutation(
        &self,
        user_id: &str,
        conversation_id: &str,
    ) -> Result<(ConversationRow, serde_json::Value), AppError> {
        let conv_id = parse_conv_id(conversation_id)?;
        self.conversation_repo
            .get(conv_id)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| AppError::NotFound(format!("Conversation {conversation_id} not found")))?;
        self.ensure_not_retained_execution_attempt(user_id, conv_id)
            .await?;

        // Serialize with runtime preparation so no build can interleave
        // between the idle proof and the runtime recycle.
        let preparation_token = CancellationToken::new();
        let _preparation_guard = self
            .runtime_state
            .acquire_preparation_gate(conversation_id, &preparation_token)
            .await?;

        let row = self
            .conversation_repo
            .get(conv_id)
            .await?
            .filter(|row| row.user_id == user_id)
            .ok_or_else(|| AppError::NotFound(format!("Conversation {conversation_id} not found")))?;
        if row.status.as_deref() == Some("running")
            || self.runtime_state.has_active_turn(conversation_id)
        {
            return Err(AppError::Conflict(format!(
                "Conversation {conversation_id} is running; summon changes require an idle conversation"
            )));
        }
        let extra = parse_extra(&row)?;
        Ok((row, extra))
    }

    /// Persist the mutated extra, then recycle the cached runtime with proof
    /// so the summon change is guaranteed to take effect on the next message.
    async fn commit_summon_extra(
        &self,
        user_id: &str,
        row: &ConversationRow,
        extra: serde_json::Value,
        operation: &'static str,
    ) -> Result<(), AppError> {
        let updates = ConversationRowUpdate {
            extra: Some(serde_json::to_string(&extra).map_err(|error| {
                AppError::Internal(format!("Failed to serialize merged extra: {error}"))
            })?),
            updated_at: Some(now_ms()),
            ..Default::default()
        };
        self.conversation_repo
            .update(parse_conv_id(&row.conversation_id)?, &updates)
            .await?;
        Self::terminate_runtime_with_proof(
            &self.runtime_registry,
            &row.conversation_id,
            AgentKillReason::ConfigurationChanged,
            operation,
        )
        .await?;
        info!(conversation_id = %row.conversation_id, operation, "companion summon updated");
        self.broadcast_list_changed(
            user_id,
            &row.conversation_id,
            "updated",
            row_source(row).as_ref(),
        );
        Ok(())
    }
}
