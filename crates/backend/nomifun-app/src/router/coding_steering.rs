//! Receipt-backed, turn-local inbox. A queued acknowledgement is not proof
//! that a model consumed or followed the text. No automatic cross-turn replay.
use std::collections::BTreeMap;
use std::sync::Weak;

use async_trait::async_trait;
use nomifun_ai_agent::RuntimeSteerDelivery;
use nomifun_chat_model_broker::ChatCausality;
use nomifun_coding_engine::{
    CodingEngineError, CodingEngineEvent, CodingInputPort, CodingSteeringInput,
};
use nomifun_common::AppError;
use nomifun_db::sqlx;

use super::{ActiveTurn, ConversationCodingHost, error};

pub(super) struct Inbox {
    open: bool,
    generation: Option<u64>,
    seen: BTreeMap<String, CodingSteeringInput>,
    pending: Vec<CodingSteeringInput>,
    prepared_image_bytes: usize,
    prepared_image_count: usize,
}
impl Default for Inbox {
    fn default() -> Self {
        Self {
            open: false,
            generation: None,
            seen: BTreeMap::new(),
            pending: Vec::new(),
            prepared_image_bytes: 0,
            prepared_image_count: 0,
        }
    }
}

impl Inbox {
    pub(super) fn permits_resource_dispatch(&self) -> bool {
        self.open && self.pending.is_empty()
    }
}

pub(super) struct HostPort(pub(super) Weak<ConversationCodingHost>);
impl std::fmt::Debug for HostPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConversationInputPort")
    }
}
fn engine_error(value: impl std::fmt::Display) -> CodingEngineError {
    CodingEngineError::InvalidContract(format!("Coding steering: {value}"))
}
fn admitted(
    turn: &ActiveTurn,
    host: &ConversationCodingHost,
    causality: &ChatCausality,
) -> Result<(), CodingEngineError> {
    if turn.cleanup_started
        || turn.cancellation.is_cancelled()
        || turn.journal.sequence() == 0
        || causality.agent_session_id.as_ref() != host.options.conversation_id
        || causality.turn_operation_id.as_ref() != turn.operation
        || causality.causation_event_id.as_ref() != turn.root
        || causality.resolved_snapshot_ref != host.snapshot_ref
        || causality.route_identity != host.route
    {
        return Err(engine_error("input request differs from the admitted turn"));
    }
    Ok(())
}

impl ConversationCodingHost {
    pub(super) async fn admit_steerable_tool(
        &self,
        message: &nomifun_ai_agent::types::SendMessageData,
        event: &CodingEngineEvent,
    ) -> Result<bool, AppError> {
        if !matches!(event, CodingEngineEvent::ToolStarted { step, .. } if *step > 0) {
            return Err(error(
                "model tool admission requires a positive-step ToolStarted",
            ));
        }
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("tool admission without active turn"))?;
        if turn.root
            != message
                .source_message_id
                .as_deref()
                .unwrap_or(&message.msg_id)
            || turn.wire_id != message.msg_id
            || turn.journal.sequence() < 2
            || turn.cleanup_started
            || turn.cancellation.is_cancelled()
            || !turn.steering.open
            || self
                .activation_failed
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(error("tool admission differs from the live input scope"));
        }
        if !turn.steering.pending.is_empty() {
            return Ok(false);
        }
        // Same lock as accept_steer/take/close, held through durable admission
        // but never through execution. Later steering cannot revoke this call.
        let admission = async {
            self.flush_steering_buffer(turn).await?;
            if turn.cancellation.is_cancelled() {
                return Err(error("turn cancelled before tool admission"));
            }
            self.append_locked_record(
                turn,
                serde_json::to_string(event).map_err(error)?,
                None,
                false,
            )
            .await
        }
        .await;
        if let Err(error) = admission {
            // An uncertain write must not be normalized into a recoverable
            // model tool error followed by further effects in this turn.
            turn.steering.open = false;
            turn.cancellation.cancel();
            return Err(error);
        }
        Ok(true)
    }

    pub(super) async fn open_steering(&self) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| error("no admitted input scope"))?;
        if turn.journal.sequence() != 1 {
            return Err(error("input scope must follow the durable turn root"));
        }
        let payload = serde_json::to_string(&CodingEngineEvent::TurnInputScope {
            wire_turn_id: turn.wire_id.clone(),
        })
        .map_err(error)?;
        self.append_locked_record(turn, payload, None, false)
            .await?;
        turn.steering.open = true;
        Ok(())
    }

    pub(super) async fn accept_steer(
        &self,
        delivery: RuntimeSteerDelivery,
    ) -> Result<bool, AppError> {
        if delivery.receipt_operation_id.len() > 1024 || delivery.text.len() > 16 * 1024 {
            return Err(error(
                "steering requires bounded receipt identity and text <=16 KiB",
            ));
        }
        let mut active = self.active.lock().await;
        let Some(turn) = active.as_mut() else {
            return Ok(false);
        };
        if !turn.steering.open
            || self
                .activation_failed
                .load(std::sync::atomic::Ordering::Acquire)
            || turn.cleanup_started
            || turn.cancellation.is_cancelled()
            || turn.journal.sequence() == 0
            || turn.wire_id != delivery.wire_turn_id
        {
            return Ok(false);
        }
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT r.message_id, CASE WHEN length(CAST(r.request_payload AS BLOB)) <= 65536 THEN r.request_payload ELSE NULL END FROM conversation_delivery_receipts r JOIN conversations c \
             ON c.conversation_id = r.conversation_id AND c.user_id = r.user_id \
             WHERE r.operation_id = ? AND r.kind = 'steer' AND r.user_id = ? AND r.conversation_id = ? \
             AND (r.status = 'accepted' OR (r.status = 'completed' AND r.result_ok = 1)) \
             AND c.status = 'running' AND c.admission_epoch = ? AND c.active_turn_operation_id = ?")
            .bind(&delivery.receipt_operation_id).bind(&self.options.user_id).bind(&self.options.conversation_id)
            .bind(turn.epoch).bind(&turn.operation).fetch_optional(&self.pool).await.map_err(error)?;
        let (message_id, raw) =
            row.ok_or_else(|| error("no admitted steering receipt for this turn"))?;
        let raw = raw.ok_or_else(|| error("oversized steering receipt"))?;
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(error)?;
        if value.get("content").and_then(|v| v.as_str()) != Some(delivery.text.as_str())
            || value
                .pointer("/turn_scope/wire_turn_id")
                .and_then(|v| v.as_str())
                != Some(turn.wire_id.as_str())
            || value
                .pointer("/turn_scope/generation")
                .and_then(|v| v.as_u64())
                != Some(delivery.turn_generation)
            || turn
                .steering
                .generation
                .is_some_and(|generation| generation != delivery.turn_generation)
        {
            return Err(error(
                "steering text/scope differs from its committed receipt",
            ));
        }
        let files = super::super::coding_attachments::references(&value)?;
        let inject_skills = super::super::coding_attachments::selected_skills(&value)?;
        if files != delivery.files || inject_skills != delivery.inject_skills {
            return Err(error("steering context differs from its committed receipt"));
        }
        self.skills.validate_ids(&inject_skills)?;
        let mut input = CodingSteeringInput {
            receipt_operation_id: delivery.receipt_operation_id,
            message_id,
            text: delivery.text,
            files,
            inject_skills,
            image_count: 0,
            prepared_images: Vec::new(),
        };
        input.validate().map_err(error)?;
        if let Some(previous) = turn.steering.seen.get(&input.receipt_operation_id) {
            return if previous.same_delivery(&input) {
                Ok(true)
            } else {
                Err(error("steering identity was reused"))
            };
        }
        if turn.steering.seen.len() >= 16 {
            return Err(error("turn steering limit reached (16 inputs)"));
        }
        let capabilities = self.capability_state.snapshot().map_err(error)?;
        let vision_active = self.primary_image_input
            && capabilities
                .active
                .iter()
                .any(|id| id.as_ref() == "llm.vision");
        // Keep the same inbox lock as tool admission and finish: no tool may
        // race past an input while the platform is preparing its attachments.
        // This is read-only, bounded local preparation, not a new tool grant.
        input.prepared_images = tokio::select! {
            biased;
            _ = turn.cancellation.cancelled() => return Ok(false),
            result = tokio::time::timeout(std::time::Duration::from_secs(10),
                super::super::coding_attachments::prepare_images(&input.files, &self.options.extra, vision_active)) =>
                result.map_err(|_| error("steering image preparation timed out; input was not queued"))??,
        };
        input.image_count = input.prepared_images.len();
        input.validate().map_err(error)?;
        let image_bytes = input
            .prepared_images
            .iter()
            .map(|part| match part {
                nomifun_chat_model_broker::ChatContentPart::Image { data_base64, .. } => {
                    data_base64.len()
                }
                _ => 0,
            })
            .sum::<usize>();
        let next_image_bytes = turn
            .steering
            .prepared_image_bytes
            .saturating_add(image_bytes);
        let next_image_count = turn
            .steering
            .prepared_image_count
            .saturating_add(input.image_count);
        if next_image_bytes > 4 * 1024 * 1024 || next_image_count > 4 {
            return Err(error(
                "turn steering image budget exceeded (4 images / 4 MiB prepared payloads); input was not queued",
            ));
        }
        // Attachment reads may outlive a database-side stop. Recheck the
        // durable receipt/turn authority before the in-memory acknowledgement.
        let (still_admitted,): (i64,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM conversation_delivery_receipts r JOIN conversations c ON c.conversation_id = r.conversation_id AND c.user_id = r.user_id \
             WHERE r.operation_id = ? AND r.kind = 'steer' AND r.user_id = ? AND r.conversation_id = ? \
             AND (r.status = 'accepted' OR (r.status = 'completed' AND r.result_ok = 1)) \
             AND c.status = 'running' AND c.admission_epoch = ? AND c.active_turn_operation_id = ?)")
            .bind(&input.receipt_operation_id).bind(&self.options.user_id).bind(&self.options.conversation_id)
            .bind(turn.epoch).bind(&turn.operation).fetch_one(&self.pool).await.map_err(error)?;
        if still_admitted != 1 {
            return Ok(false);
        }
        if turn.cancellation.is_cancelled() {
            return Ok(false);
        }
        // Receipt was already committed by the Conversation owner. No await
        // between queue insertion and acknowledgement; read failures queued nothing.
        turn.steering.generation = Some(delivery.turn_generation);
        turn.steering.prepared_image_bytes = next_image_bytes;
        turn.steering.prepared_image_count = next_image_count;
        let recorded = input.journal_record();
        turn.steering
            .seen
            .insert(input.receipt_operation_id.clone(), recorded);
        turn.steering.pending.push(input);
        Ok(true)
    }

    pub(super) async fn close_steering(&self) -> Result<(), AppError> {
        let mut active = self.active.lock().await;
        let Some(turn) = active.as_mut() else {
            return Ok(());
        };
        turn.steering.open = false;
        turn.cleanup_started = true;
        if !turn.steering.pending.is_empty() {
            self.flush_steering_buffer(turn).await?;
            let event = CodingEngineEvent::SteeringDeferred { inputs: turn.steering.pending.iter().map(CodingSteeringInput::journal_record).collect(),
                reason: "Turn ended or was cancelled before these queued instructions reached the next model boundary. No automatic retry or new turn was started.".into() };
            self.append_locked_record(
                turn,
                serde_json::to_string(&event).map_err(error)?,
                None,
                false,
            )
            .await?;
            turn.steering.pending.clear();
        }
        Ok(())
    }

    async fn flush_steering_buffer(&self, turn: &mut ActiveTurn) -> Result<(), AppError> {
        let mut records = Vec::new();
        turn.event_buffer.flush(&mut records);
        for event in records {
            self.append_locked_record(
                turn,
                serde_json::to_string(&event).map_err(error)?,
                None,
                false,
            )
            .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl CodingInputPort for HostPort {
    async fn take(
        &self,
        causality: &ChatCausality,
        close_if_empty: bool,
    ) -> Result<Vec<CodingSteeringInput>, CodingEngineError> {
        let host = self
            .0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))?;
        let mut active = host.active.lock().await;
        let turn = active
            .as_mut()
            .ok_or_else(|| engine_error("no active turn"))?;
        admitted(turn, &host, causality)?;
        if turn.steering.pending.is_empty() {
            if close_if_empty {
                turn.steering.open = false;
            }
            return Ok(Vec::new());
        }
        turn.steering.open = false; // remains closed if a journal write fails
        host.flush_steering_buffer(turn)
            .await
            .map_err(engine_error)?;
        let payload = serde_json::to_string(&CodingEngineEvent::SteeringInputs {
            inputs: turn
                .steering
                .pending
                .iter()
                .map(CodingSteeringInput::journal_record)
                .collect(),
        })
        .map_err(engine_error)?;
        host.append_locked_record(turn, payload, None, false)
            .await
            .map_err(engine_error)?;
        let inputs = std::mem::take(&mut turn.steering.pending);
        turn.steering.open = true;
        Ok(inputs)
    }

    async fn has_pending(&self, causality: &ChatCausality) -> Result<bool, CodingEngineError> {
        let host = self
            .0
            .upgrade()
            .ok_or_else(|| engine_error("Session host has shut down"))?;
        let active = host.active.lock().await;
        let turn = active
            .as_ref()
            .ok_or_else(|| engine_error("no active turn"))?;
        admitted(turn, &host, causality)?;
        Ok(!turn.steering.pending.is_empty())
    }
}
