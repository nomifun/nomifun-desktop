//! A returned value can violate the observation contract without making its
//! native resource immortal. Record a bounded rejection on the original call;
//! its summaries identify rejected bytes and never authorize their use.
use super::*;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum Violation {
    OwnerCallMismatch,
    OwnerResultTooLarge,
    OwnerResultInvalid,
    ObservationCallMismatch,
    ObservationResultTooLarge,
    ObservationResultInvalid,
    ObservationClassificationChanged,
    ObservationPolicyPanicked,
}

impl Violation {
    fn message(self) -> &'static str {
        match self {
            Self::OwnerCallMismatch => "the original owner returned another call identity",
            Self::OwnerResultTooLarge => "the original owner return exceeded the result contract",
            Self::OwnerResultInvalid => "the original owner return violated the result contract",
            Self::ObservationCallMismatch => "the observation policy changed the original call identity",
            Self::ObservationResultTooLarge => "the observation exceeded the bounded journal result contract",
            Self::ObservationResultInvalid => "the observation policy produced an invalid result",
            Self::ObservationClassificationChanged => "the observation policy changed the original result classification",
            Self::ObservationPolicyPanicked => "the observation policy panicked after the original owner returned",
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
    #[serde(skip_serializing_if = "Option::is_none")]
    rejected_observation: Option<ReturnSummary>,
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
    outcome: &Result<EngineToolResult, EngineToolError>, recorded: Option<&EngineToolResult>,
    policy_panicked: bool, owner_invoked: bool) -> (String, Option<ToolReturnRejection>) {
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
        if policy_panicked { return Some(Violation::ObservationPolicyPanicked); }
        if let Some(observed) = recorded {
            if observed.call_id != original.call_id { return Some(Violation::ObservationCallMismatch); }
            if observed.is_error != original.is_error { return Some(Violation::ObservationClassificationChanged); }
            if let Err(error) = observed.validate_for(&invocation.call.call_id) {
                return Some(if matches!(error, EngineToolError::ToolResultTooLarge {..}) {
                    Violation::ObservationResultTooLarge
                } else { Violation::ObservationResultInvalid });
            }
        }
        (payload.to_string().len() > 64 * 1024).then_some(Violation::ObservationResultTooLarge)
    });
    let rejection = violation.map(|kind| ToolReturnRejection { kind, owner_returned: true,
        original_return: ReturnSummary::of(outcome.as_ref().expect("a rejected return is an actual owner result")),
        rejected_observation: recorded.map(ReturnSummary::of) });
    if let Some(rejection) = &rejection {
        payload["result"] = serde_json::Value::Null;
        payload["error"] = rejection.message().into();
        payload["contract_rejection"] = serde_json::to_value(rejection)
            .expect("a bounded typed rejection has a JSON representation");
    }
    (payload.to_string(),rejection)
}
