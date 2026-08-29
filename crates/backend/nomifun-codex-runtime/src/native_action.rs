use async_trait::async_trait;
use nomifun_agent_contracts::{
    NativeActionStart, NativeActionStartAck, RuntimeEventWireAck, RuntimeEventWireEnvelope,
};

use crate::error::RuntimeError;

#[async_trait]
pub trait RuntimeIngressPort: Send + Sync {
    async fn append_runtime_event(
        &self,
        event: RuntimeEventWireEnvelope,
    ) -> Result<RuntimeEventWireAck, RuntimeError>;

    async fn commit_native_action_start(
        &self,
        start: NativeActionStart,
    ) -> Result<NativeActionStartAck, RuntimeError>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AckedNativeAction {
    start: NativeActionStart,
    ack: NativeActionStartAck,
}

impl AckedNativeAction {
    pub fn start(&self) -> &NativeActionStart {
        &self.start
    }

    pub fn ack(&self) -> &NativeActionStartAck {
        &self.ack
    }

    pub(crate) fn after_durable_commit(
        start: NativeActionStart,
        ack: NativeActionStartAck,
    ) -> Result<Self, RuntimeError> {
        validate_native_action_ack(&start, &ack)?;
        Ok(Self { start, ack })
    }
}

pub fn validate_runtime_event_ack(
    event: &RuntimeEventWireEnvelope,
    ack: &RuntimeEventWireAck,
) -> Result<(), RuntimeError> {
    if ack.runtime_binding_id != event.runtime_binding_id
        || ack.committed_producer_seq != event.producer_seq
        || ack.session_event_ack.event_id != event.event_id
    {
        return Err(RuntimeError::Protocol(
            "runtime event ACK does not match the exact inbound event".to_owned(),
        ));
    }
    Ok(())
}

pub fn validate_native_action_ack(
    start: &NativeActionStart,
    ack: &NativeActionStartAck,
) -> Result<(), RuntimeError> {
    if ack.agent_session_id != start.agent_session_id
        || ack.runtime_binding_id != start.runtime_binding_id
        || ack.turn_operation_id != start.turn_operation_id
        || ack.action_id != start.action_id
        || ack.effect_id != start.effect_id
        || ack.idempotency_key != start.idempotency_key
        || ack.capability_id != start.capability_id
        || ack.active_set_generation != start.active_set_generation
        || ack.snapshot_digest != start.snapshot_digest
    {
        return Err(RuntimeError::NativeActionAck(
            "ACK identity differs from native_action/start".to_owned(),
        ));
    }
    if ack.effect_started_event_id.as_ref().is_empty() || ack.committed_session_seq == 0 {
        return Err(RuntimeError::NativeActionAck(
            "ACK must reference a durable effect/started event and committed session sequence"
                .to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use nomifun_agent_contracts::NativeActionStartAckExchange;

    use super::*;

    #[test]
    fn frozen_ack_fixture_is_accepted() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/native-action-start-ack.json"
        );
        let exchange: NativeActionStartAckExchange =
            serde_json::from_str(fixture).unwrap();
        validate_native_action_ack(&exchange.start, &exchange.ack).unwrap();
        AckedNativeAction::after_durable_commit(exchange.start, exchange.ack).unwrap();
    }

    #[test]
    fn mismatched_generation_never_produces_execution_permit() {
        let fixture = include_str!(
            "../../nomifun-agent-contracts/contracts/runtime/native-action-start-ack.json"
        );
        let mut exchange: NativeActionStartAckExchange =
            serde_json::from_str(fixture).unwrap();
        exchange.ack.active_set_generation += 1;
        assert!(
            AckedNativeAction::after_durable_commit(exchange.start, exchange.ack).is_err()
        );
    }
}
