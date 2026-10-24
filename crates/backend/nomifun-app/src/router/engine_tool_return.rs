//! Admit the original owner result to the canonical journal before the Runtime
//! consumes it. Unsupported sizes get an explicit rejection on the same call;
//! successful results are never silently converted into excerpts.
use super::*;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Violation {
    OwnerCallMismatch,
    OwnerResultTooLarge,
    OwnerResultInvalid,
    CanonicalResultTooLarge,
}

impl Violation {
    fn message(self) -> &'static str {
        match self {
            Self::OwnerCallMismatch => "the original owner returned another call identity",
            Self::OwnerResultTooLarge => "the original owner return exceeded the result contract",
            Self::OwnerResultInvalid => "the original owner return violated the result contract",
            Self::CanonicalResultTooLarge => "the original return exceeds the canonical payload contract; the owner must provide bounded pages",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct ReturnSummary { sha256: nomifun_agent_contracts::DigestHex, bytes: usize }

impl ReturnSummary {
    fn of(result: &EngineToolResult) -> Self {
        // EngineToolResult contains only strings, booleans and their vectors;
        // it has no fallible/custom serializer or non-finite numeric value.
        let bytes = nomifun_agent_contracts::canonical_json_bytes(result)
            .expect("the typed EngineToolResult has a canonical JSON representation");
        Self { sha256: nomifun_agent_contracts::digest_bytes(&bytes), bytes: bytes.len() }
    }
}

#[derive(Clone, Debug, Serialize)]
pub(super) struct ToolReturnRejection {
    kind: Violation,
    owner_returned: bool,
    original_return: ReturnSummary,
}

impl ToolReturnRejection {
    pub(super) fn error(&self) -> EngineToolError {
        EngineToolError::InvalidContract(self.message())
    }

    fn message(&self) -> String {
        format!("HOST_TOOL_RETURN_CONTRACT_REJECTED: {}; rejected bytes are identified by the original owner observation and are not an accepted successful result", self.kind.message())
    }
}

pub(super) fn settlement_payload(dispatch: &EngineToolDispatchRecord, invocation: &EngineToolInvocation,
    outcome: &Result<EngineToolResult, EngineToolError>, owner_invoked: bool) -> (String, Option<ToolReturnRejection>) {
    let recorded = outcome.as_ref().ok().map(|result|
        super::super::engine_tool_record::for_call(&invocation.call, result));
    let message = outcome.as_ref().err().map(|error|
        bounded_error(&nomi_redact::redact_secrets_owned(error.to_string())));
    let mut payload = serde_json::json!({"event":"host_tool_settled","operation_id":dispatch.operation_id,
        "call_id":dispatch.call_id,"result":recorded,"error":message});
    if !owner_invoked { payload["owner_invoked"] = serde_json::Value::Bool(false); }
    let violation = outcome.as_ref().ok().and_then(|original| {
        if original.call_id != invocation.call.call_id { return Some(Violation::OwnerCallMismatch); }
        if let Err(error) = original.validate_for(&invocation.call.call_id) {
            return Some(if matches!(error, EngineToolError::ToolResultTooLarge {..}) {
                Violation::OwnerResultTooLarge
            } else { Violation::OwnerResultInvalid });
        }
        // Include both the owner receipt and its possible recovery observation,
        // with the actual Session/operation identities and largest sequence.
        // This also bounds the smaller tool/result-recorded projection. The
        // normal journal selects inline vs stored Payload; 64 KiB is only the
        // inline threshold, never a limit on an otherwise valid owner result.
        let recovered = nomifun_agent_runtime::AgentEngineEvent::ToolOutcomeReconciled {
            step: u16::MAX,
            result: recorded.as_ref().expect("original successful return has a canonical representation").clone(),
            source: nomifun_agent_runtime::AgentReconciliationSource::OwnerReceipt,
            evidence_event_id: Some(format!("runtime-progress:{}:{}:{}",
                invocation.agent_session_id.as_ref(), invocation.turn_operation_id.as_ref(), u64::MAX)),
            owner_operation_id: Some(invocation.operation_id.clone()),
        };
        [&payload, &serde_json::to_value(recovered).expect("typed recovery observation")].into_iter()
            .any(|event| nomifun_agent_contracts::canonical_json_bytes(
                &serde_json::json!({"producer_seq":u64::MAX,"event":event}))
                .expect("typed tool observation has a canonical JSON representation").len()
                > nomifun_agent_session::MAX_SINGLE_PAYLOAD_BYTES)
            .then_some(Violation::CanonicalResultTooLarge)
    });
    let rejection = violation.map(|kind| ToolReturnRejection { kind, owner_returned: true,
        original_return: ReturnSummary::of(outcome.as_ref().expect("a rejected return is an actual owner result")) });
    if let Some(rejection) = &rejection {
        payload["result"] = serde_json::Value::Null;
        payload["error"] = rejection.message().into();
        payload["contract_rejection"] = serde_json::to_value(rejection)
            .expect("a bounded typed rejection has a JSON representation");
    }
    (payload.to_string(),rejection)
}

#[cfg(test)]
#[path = "engine_tool_return_tests.rs"]
mod tests;
