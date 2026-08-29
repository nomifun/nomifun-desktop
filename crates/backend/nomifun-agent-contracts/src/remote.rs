use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::event::{SessionEventCursor, SessionEventRecord};
use crate::preset::AgentBindingValue;
use crate::{
    AgentSessionId, CanonicalErrorCode, DigestHex, IdempotencyKey, RemoteBindingId,
    StrictJsonValue, UserId, VersionString,
};

pub const REMOTE_AUTH_REQUIRED: &str = "REMOTE_AUTH_REQUIRED";
pub const REMOTE_BINDING_NOT_FOUND: &str = "REMOTE_BINDING_NOT_FOUND";
pub const REMOTE_BINDING_VERSION_CONFLICT: &str = "REMOTE_BINDING_VERSION_CONFLICT";
pub const REMOTE_BINDING_DIGEST_CONFLICT: &str = "REMOTE_BINDING_DIGEST_CONFLICT";
pub const REMOTE_SESSION_NOT_FOUND: &str = "REMOTE_SESSION_NOT_FOUND";
pub const REMOTE_SESSION_OPENING: &str = "REMOTE_SESSION_OPENING";
pub const REMOTE_OPEN_FAILED: &str = "REMOTE_OPEN_FAILED";
pub const REMOTE_SESSION_BUSY: &str = "REMOTE_SESSION_BUSY";
pub const REMOTE_IDEMPOTENCY_CONFLICT: &str = "REMOTE_IDEMPOTENCY_CONFLICT";
pub const SESSION_DELETED: &str = "SESSION_DELETED";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteBinding {
    pub remote_binding_id: RemoteBindingId,
    pub owner_user_id: UserId,
    pub name: String,
    pub agent_binding: AgentBindingValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteBindingVersionRef {
    pub remote_binding_id: RemoteBindingId,
    pub binding_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RemoteOpenState {
    Opening,
    Ready,
    Failed {
        code: CanonicalErrorCode,
        recoverable: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteOpenRequest {
    pub binding_id: RemoteBindingId,
    pub idempotency_key: IdempotencyKey,
    pub initial_input: Option<StrictJsonValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteOpenResponse {
    pub agent_session_id: AgentSessionId,
    pub agent_binding: AgentBindingValue,
    pub open_state: RemoteOpenState,
    pub cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteTurnRequest {
    pub agent_session_id: AgentSessionId,
    pub input: StrictJsonValue,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteObserveRequest {
    pub agent_session_id: AgentSessionId,
    pub after_cursor: SessionEventCursor,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteCancelRequest {
    pub agent_session_id: AgentSessionId,
    pub idempotency_key: IdempotencyKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteMutationResponse {
    pub agent_session_id: AgentSessionId,
    pub cursor: SessionEventCursor,
    pub session_status: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteObserveResponse {
    pub agent_session_id: AgentSessionId,
    pub events: Vec<SessionEventRecord>,
    pub messages: Vec<StrictJsonValue>,
    pub next_cursor: SessionEventCursor,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteOpenFixture {
    pub request: RemoteOpenRequest,
    pub opening_response: RemoteOpenResponse,
    pub ready_state: RemoteOpenState,
    pub failed_state: RemoteOpenState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteProtocolInvariants {
    pub binding_update_affects_existing_session: bool,
    pub binding_delete_cancels_existing_session: bool,
    pub network_disconnect_changes_session_fact: bool,
    pub transport_session_id_is_product_identity: bool,
    pub direct_capability_requires_agent_session_id: bool,
    pub full_auto_only: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteBindingProtocolFixture {
    pub contract_version: VersionString,
    pub remote_binding: RemoteBinding,
    pub open: RemoteOpenFixture,
    pub turn: RemoteTurnRequest,
    pub observe: RemoteObserveRequest,
    pub cancel: RemoteCancelRequest,
    pub invariants: RemoteProtocolInvariants,
    pub forbidden_remote_binding_fields: BTreeSet<String>,
}

pub const REMOTE_BINDING_PROTOCOL_FIXTURE_JSON: &str =
    include_str!("../contracts/remote/remote-binding-and-protocol.fixture.json");

pub fn remote_binding_protocol_fixture() -> RemoteBindingProtocolFixture {
    serde_json::from_str(REMOTE_BINDING_PROTOCOL_FIXTURE_JSON)
        .expect("Remote binding/protocol fixture must match RemoteBindingProtocolFixture")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InstallationAuthStatus {
    Active,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallationAuthRecord {
    pub owner_user_id: UserId,
    pub current_verifier_hash: Option<DigestHex>,
    pub auth_revision: u64,
    pub status: InstallationAuthStatus,
    pub updated_at_ms: i64,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RemoteOperation {
    Open,
    Turn,
    Observe,
    Cancel,
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RemoteAuthMutation {
    Rotate,
    Revoke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RequestAdmissionOrdering {
    RequestAdmissionCommittedFirst,
    AuthMutationCommittedFirst,
    AfterAuthMutationFence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RequestAdmissionOutcome {
    AcceptedToOrdinaryFiniteBoundary,
    Rejected { code: CanonicalErrorCode },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D026RequestAdmissionFixtureCase {
    pub case_id: String,
    pub applies_to_operations: BTreeSet<RemoteOperation>,
    pub applies_to_auth_mutations: BTreeSet<RemoteAuthMutation>,
    pub ordering: RequestAdmissionOrdering,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_outcome: Option<RequestAdmissionOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lookup_binding_or_session: Option<bool>,
    pub expected_agent_session_mutations: u32,
    pub expected_remote_binding_mutations: u32,
    pub expected_effect_replays: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_credential_requires_same_owner: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replacement_credential_requires_explicit_agent_session_id: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implicit_lookup_by_token_connection_or_recent_session: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D026RequestAdmissionFixturePayload {
    pub schema_version: VersionString,
    pub operation_exact_set: BTreeSet<RemoteOperation>,
    pub auth_mutation_exact_set: BTreeSet<RemoteAuthMutation>,
    pub cases: Vec<D026RequestAdmissionFixtureCase>,
    pub forbidden_auth_state: BTreeSet<String>,
}

pub const D026_REQUEST_ADMISSION_ORDERING_FIXTURE_JSON: &str =
    include_str!("../contracts/remote/d026-request-admission-ordering.fixture.json");

pub fn d026_request_admission_fixture() -> D026RequestAdmissionFixturePayload {
    serde_json::from_str(D026_REQUEST_ADMISSION_ORDERING_FIXTURE_JSON)
        .expect("D-026 fixture must match D026RequestAdmissionFixturePayload")
}

pub fn remote_canonical_error_codes() -> BTreeSet<CanonicalErrorCode> {
    [
        REMOTE_AUTH_REQUIRED,
        REMOTE_BINDING_NOT_FOUND,
        REMOTE_BINDING_VERSION_CONFLICT,
        REMOTE_BINDING_DIGEST_CONFLICT,
        REMOTE_SESSION_NOT_FOUND,
        REMOTE_SESSION_OPENING,
        REMOTE_OPEN_FAILED,
        REMOTE_SESSION_BUSY,
        REMOTE_IDEMPOTENCY_CONFLICT,
        SESSION_DELETED,
        crate::runtime::SNAPSHOT_EXECUTOR_UNAVAILABLE,
        crate::preset::CAPABILITY_NOT_MATERIALIZED,
        crate::preset::CAPABILITY_UNAVAILABLE_ON_PLATFORM,
        crate::preset::CAPABILITY_NOT_IN_PRESET,
        crate::preset::CAPABILITY_NOT_ACTIVE,
        crate::preset::PRESET_RESOURCE_NOT_BOUND,
        crate::preset::RESOURCE_OWNER_MISMATCH,
    ]
    .into_iter()
    .map(CanonicalErrorCode::from)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_binding_serializes_only_the_exact_four_fields() {
        let fixture = remote_binding_protocol_fixture();
        let json = serde_json::to_value(fixture.remote_binding).unwrap();
        let keys = json.as_object().unwrap().keys().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "agent_binding".into(),
                "name".into(),
                "owner_user_id".into(),
                "remote_binding_id".into(),
            ])
        );
    }

    #[test]
    fn remote_open_fixture_covers_opening_ready_and_failed() {
        let fixture = remote_binding_protocol_fixture();
        assert_eq!(fixture.open.opening_response.open_state, RemoteOpenState::Opening);
        assert_eq!(fixture.open.ready_state, RemoteOpenState::Ready);
        assert!(matches!(
            fixture.open.failed_state,
            RemoteOpenState::Failed {
                ref code,
                recoverable: true
            } if code.as_ref() == REMOTE_OPEN_FAILED
        ));
    }

    #[test]
    fn d026_covers_every_operation_and_both_linearization_orders() {
        let fixture = d026_request_admission_fixture();
        assert_eq!(
            fixture.operation_exact_set,
            BTreeSet::from([
                RemoteOperation::Open,
                RemoteOperation::Turn,
                RemoteOperation::Observe,
                RemoteOperation::Cancel,
            ])
        );
        assert_eq!(
            fixture.auth_mutation_exact_set,
            BTreeSet::from([RemoteAuthMutation::Rotate, RemoteAuthMutation::Revoke])
        );
        assert_eq!(fixture.cases.len(), 3);

        let request_first = fixture
            .cases
            .iter()
            .find(|case| {
                case.ordering == RequestAdmissionOrdering::RequestAdmissionCommittedFirst
            })
            .unwrap();
        assert_eq!(
            request_first.expected_outcome.as_ref(),
            Some(&RequestAdmissionOutcome::AcceptedToOrdinaryFiniteBoundary)
        );

        let auth_first = fixture
            .cases
            .iter()
            .find(|case| case.ordering == RequestAdmissionOrdering::AuthMutationCommittedFirst)
            .unwrap();
        assert_eq!(
            auth_first.expected_outcome.as_ref(),
            Some(&RequestAdmissionOutcome::Rejected {
                code: CanonicalErrorCode::from(REMOTE_AUTH_REQUIRED),
            })
        );
        assert_eq!(auth_first.lookup_binding_or_session, Some(false));

        let replacement = fixture
            .cases
            .iter()
            .find(|case| case.ordering == RequestAdmissionOrdering::AfterAuthMutationFence)
            .unwrap();
        assert_eq!(
            replacement.applies_to_operations,
            BTreeSet::from([
                RemoteOperation::Turn,
                RemoteOperation::Observe,
                RemoteOperation::Cancel,
            ])
        );
        assert_eq!(
            replacement.replacement_credential_requires_same_owner,
            Some(true)
        );
        assert_eq!(
            replacement.replacement_credential_requires_explicit_agent_session_id,
            Some(true)
        );
        assert_eq!(
            replacement.implicit_lookup_by_token_connection_or_recent_session,
            Some(false)
        );

        for operation in [
            RemoteOperation::Open,
            RemoteOperation::Turn,
            RemoteOperation::Observe,
            RemoteOperation::Cancel,
        ] {
            assert!(request_first.applies_to_operations.contains(&operation));
            assert!(auth_first.applies_to_operations.contains(&operation));
        }
        assert!(fixture.cases.iter().all(|case| {
            case.expected_agent_session_mutations == 0
                && case.expected_remote_binding_mutations == 0
                && case.expected_effect_replays == 0
        }));
    }

    #[test]
    fn every_remote_error_code_is_uppercase() {
        assert!(remote_canonical_error_codes()
            .iter()
            .all(|code| code.as_ref() == code.as_ref().to_ascii_uppercase()));
    }
}
