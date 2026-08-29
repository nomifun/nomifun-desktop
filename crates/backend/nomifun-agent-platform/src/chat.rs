use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    AgentSessionDeletedState, AgentSessionId, CompactionCompletedPayload, DigestHex,
    OfficialPresetKey, PrincipalRef, ResolvedSnapshotContent, RuntimeProfileKind,
    SESSION_DELETED, SessionEventPayloadRef, SessionEventRecord,
    official_preset_seed_manifest_payload,
};
use nomifun_agent_session::{
    DeleteResult, SessionObservation, SessionRehydrationInput,
};
use nomifun_api_types::{
    AgentPresetEditorResponse, AgentPresetSourceDto, OfficialPresetKeyDto,
    OfficialPresetTemplateDto, PreviewStatusDto, ResolveAgentPresetPreviewResponse,
};
use nomifun_chat_model_broker::{
    ChatContentPart, ChatFinishReason, ChatModelEvent, ChatModelFeature, ChatModelRequest,
    ChatModality, ChatRole, ChatToolChoice,
};
use nomifun_codex_runtime::{PinnedRuntimeProfile, RuntimeProfileLaunchPolicy};
use thiserror::Error;

pub const CHAT_MINIMAL_TEMPLATE_KEY: &str = "chat.minimal";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMinimalContract {
    pub target_contribution_manifest_digest: DigestHex,
    pub runtime_feature_inventory_digest: DigestHex,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChatMinimalHiddenInitialization {
    pub capability_provider: u64,
    pub skill_catalog: u64,
    pub mcp: u64,
    pub workspace: u64,
    pub agents_instructions: u64,
    pub git: u64,
    pub shell: u64,
    pub patch: u64,
    pub memory: u64,
    pub knowledge: u64,
    pub business_context: u64,
    pub browser: u64,
    pub computer: u64,
    pub ssh: u64,
    pub office: u64,
    pub worker: u64,
    pub watcher: u64,
    pub resource_handle: u64,
    pub coding_context: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMinimalEventEvidence {
    pub event_kinds: BTreeSet<String>,
    pub projection_intents: BTreeSet<String>,
    pub streamed_text: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ChatMinimalError {
    #[error("frozen chat.minimal contract is invalid: {0}")]
    FrozenContract(String),
    #[error("chat.minimal template mismatch: {0}")]
    Template(String),
    #[error("chat.minimal ordinary Revision mismatch: {0}")]
    Revision(String),
    #[error("chat.minimal Preview mismatch: {0}")]
    Preview(String),
    #[error("chat.minimal Snapshot mismatch: {0}")]
    Snapshot(String),
    #[error("managed_minimal RuntimeProfile mismatch: {0}")]
    RuntimeProfile(String),
    #[error("chat.minimal hidden initialization mismatch: {0}")]
    HiddenInitialization(String),
    #[error("chat.minimal model request mismatch: {0}")]
    ModelRequest(String),
    #[error("chat.minimal model stream mismatch: {0}")]
    ModelStream(String),
    #[error("chat.minimal event/projection evidence mismatch: {0}")]
    Evidence(String),
    #[error("chat.minimal compaction or resume mismatch: {0}")]
    Rehydration(String),
    #[error("chat.minimal projection rebuild mismatch: {0}")]
    Rebuild(String),
    #[error("D-024 delete result mismatch: {0}")]
    Delete(String),
}

impl ChatMinimalContract {
    pub fn frozen() -> Result<Self, ChatMinimalError> {
        let manifest = official_preset_seed_manifest_payload();
        manifest
            .validate()
            .map_err(|error| ChatMinimalError::FrozenContract(error.message))?;
        let seed = manifest
            .templates
            .get(&OfficialPresetKey::ChatMinimal)
            .ok_or_else(|| {
                ChatMinimalError::FrozenContract(
                    "OfficialPresetSeedManifest has no chat.minimal seed".to_owned(),
                )
            })?;
        let coverage = manifest
            .role_coverage
            .get(&OfficialPresetKey::ChatMinimal)
            .ok_or_else(|| {
                ChatMinimalError::FrozenContract(
                    "OfficialPresetSeedManifest has no chat.minimal role coverage".to_owned(),
                )
            })?;
        if !seed.initial_capabilities.is_empty()
            || !seed.on_demand_capabilities.is_empty()
            || !seed.skill_bindings.is_empty()
            || !seed.typed_resource_defaults.is_empty()
            || !seed.required_runtime_features.is_empty()
            || !coverage.required_capability_categories.is_empty()
            || !coverage.required_capability_ids.is_empty()
            || !coverage.required_runtime_features.is_empty()
            || !coverage.required_resource_kinds.is_empty()
        {
            return Err(ChatMinimalError::FrozenContract(
                "chat.minimal seed and role coverage must be exact-empty".to_owned(),
            ));
        }
        Ok(Self {
            target_contribution_manifest_digest: manifest
                .target_first_party_contribution_digest,
            runtime_feature_inventory_digest: manifest
                .target_runtime_feature_inventory_digest,
        })
    }

    pub fn validate_template(
        &self,
        template: &OfficialPresetTemplateDto,
    ) -> Result<(), ChatMinimalError> {
        if template.template_key != OfficialPresetKeyDto::ChatMinimal
            || !template.immutable
            || !template.forkable
        {
            return Err(ChatMinimalError::Template(
                "official template identity or lifecycle flags differ".to_owned(),
            ));
        }
        if !template.seed.initial_capabilities.is_empty()
            || !template.seed.on_demand_capabilities.is_empty()
            || !template.seed.skill_bindings.is_empty()
            || !template.seed.typed_resource_defaults.is_empty()
            || !template.seed.required_runtime_features.is_empty()
            || !template
                .role_coverage
                .required_capability_categories
                .is_empty()
            || !template.role_coverage.required_capability_ids.is_empty()
            || !template.role_coverage.required_runtime_features.is_empty()
            || !template.role_coverage.required_resource_kinds.is_empty()
        {
            return Err(ChatMinimalError::Template(
                "official chat.minimal template is not exact-empty".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_ordinary_revision(
        &self,
        editor: &AgentPresetEditorResponse,
    ) -> Result<(), ChatMinimalError> {
        let revision = editor.revision.as_ref().ok_or_else(|| {
            ChatMinimalError::Revision(
                "official template did not create an immutable Revision".to_owned(),
            )
        })?;
        if editor.preset.source != AgentPresetSourceDto::User
            || editor.preset.owner_user_id.is_none()
            || editor.preset.current_stable_revision.as_ref() != Some(&revision.reference)
            || editor.draft.current_revision.as_ref() != Some(&revision.reference)
            || editor.draft.preset_id != revision.reference.preset_id
            || editor.draft.document != revision.document
        {
            return Err(ChatMinimalError::Revision(
                "template expansion did not produce the ordinary user-owned stable Revision"
                    .to_owned(),
            ));
        }
        let document = &revision.document;
        if !document.initial_capabilities.is_empty()
            || !document.on_demand_capabilities.is_empty()
            || !document.skill_bindings.is_empty()
            || !document.resource_bindings.is_empty()
            || !document.persona.is_empty()
            || !document.instructions.is_empty()
        {
            return Err(ChatMinimalError::Revision(
                "official chat.minimal Revision contains capability, resource, or hidden context"
                    .to_owned(),
            ));
        }
        if document.model_route_refs.len() != 1 {
            return Err(ChatMinimalError::Revision(
                "chat.minimal Revision must freeze exactly one model route".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_preview(
        &self,
        preview: &ResolveAgentPresetPreviewResponse,
    ) -> Result<(), ChatMinimalError> {
        if preview.status != PreviewStatusDto::Ready
            || !preview.can_save_revision
            || !preview.can_create_session
            || !preview.diagnostics.is_empty()
            || preview.resolved_snapshot_ref.is_none()
        {
            return Err(ChatMinimalError::Preview(
                "chat.minimal Preview is not a ready executable Snapshot".to_owned(),
            ));
        }
        let summary = &preview.summary;
        if summary.initial_count != 0
            || summary.on_demand_count != 0
            || summary.active_at_start_count != 0
            || summary.model_tool_count != 0
            || summary.context_contributor_count != 0
            || summary.on_demand_index_count != 0
            || summary.skill_count != 0
            || summary.mcp_count != 0
            || summary.resource_binding_count != 0
            || summary.provider_initialization_count != 1
        {
            return Err(ChatMinimalError::Preview(
                "Preview summary is not the exact zero-tool shape".to_owned(),
            ));
        }
        let inspector = &preview.inspector;
        if inspector.runtime_profile.as_deref() != Some("managed_minimal")
            || !inspector.required_runtime_features.is_empty()
            || !inspector.initial_capabilities.is_empty()
            || !inspector.on_demand_capabilities.is_empty()
            || !inspector.compact_on_demand_index.is_empty()
            || !inspector.tool_schema_refs.is_empty()
            || !inspector.context_schema_refs.is_empty()
            || !inspector.mcp_materializations.is_empty()
            || !inspector.typed_resource_bindings.is_empty()
            || !inspector.service_key_diagnostics.is_empty()
        {
            return Err(ChatMinimalError::Preview(
                "Preview inspector contains a hidden capability, index, tool, context, MCP, or resource"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_snapshot(
        &self,
        snapshot: &ResolvedSnapshotContent,
    ) -> Result<(), ChatMinimalError> {
        if snapshot.required_runtime_profile != RuntimeProfileKind::ManagedMinimal {
            return Err(ChatMinimalError::Snapshot(
                "chat.minimal must compile to managed_minimal".to_owned(),
            ));
        }
        if snapshot.runtime_feature_inventory_digest != self.runtime_feature_inventory_digest
            || snapshot.target_contribution_manifest_digest
                != self.target_contribution_manifest_digest
        {
            return Err(ChatMinimalError::Snapshot(
                "Snapshot is not bound to the frozen runtime and contribution inventories"
                    .to_owned(),
            ));
        }
        if snapshot.model_route_refs.len() != 1
            || !snapshot.required_runtime_features.is_empty()
            || !snapshot.initial_capabilities.is_empty()
            || !snapshot.on_demand_capabilities.is_empty()
            || !snapshot.on_demand_activation_plans.is_empty()
            || !snapshot.compact_on_demand_index.is_empty()
            || !snapshot.capability_allowlist.is_empty()
            || !snapshot.skill_locks.is_empty()
            || !snapshot.mcp_tool_locks.is_empty()
            || !snapshot.typed_resource_bindings.is_empty()
        {
            return Err(ChatMinimalError::Snapshot(
                "Snapshot is not exact-empty outside its one frozen model route".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_runtime_profile(
        &self,
        profile: &PinnedRuntimeProfile,
    ) -> Result<(), ChatMinimalError> {
        if profile.kind != RuntimeProfileKind::ManagedMinimal
            || !profile.enabled_runtime_features.is_empty()
            || !profile.initial_capabilities.is_empty()
            || !profile.on_demand_capabilities.is_empty()
            || !profile.typed_resource_bindings.is_empty()
        {
            return Err(ChatMinimalError::RuntimeProfile(
                "PinnedRuntimeProfile is not the exact-empty managed_minimal profile".to_owned(),
            ));
        }
        if profile.launch_policy() != RuntimeProfileLaunchPolicy::managed_minimal() {
            return Err(ChatMinimalError::RuntimeProfile(
                "managed_minimal launch policy enabled a Coding surface".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_hidden_initialization(
        &self,
        evidence: &ChatMinimalHiddenInitialization,
    ) -> Result<(), ChatMinimalError> {
        let nonzero = [
            ("capability_provider", evidence.capability_provider),
            ("skill_catalog", evidence.skill_catalog),
            ("mcp", evidence.mcp),
            ("workspace", evidence.workspace),
            ("agents_instructions", evidence.agents_instructions),
            ("git", evidence.git),
            ("shell", evidence.shell),
            ("patch", evidence.patch),
            ("memory", evidence.memory),
            ("knowledge", evidence.knowledge),
            ("business_context", evidence.business_context),
            ("browser", evidence.browser),
            ("computer", evidence.computer),
            ("ssh", evidence.ssh),
            ("office", evidence.office),
            ("worker", evidence.worker),
            ("watcher", evidence.watcher),
            ("resource_handle", evidence.resource_handle),
            ("coding_context", evidence.coding_context),
        ]
        .into_iter()
        .filter(|(_, count)| *count != 0)
        .collect::<Vec<_>>();
        if !nonzero.is_empty() {
            return Err(ChatMinimalError::HiddenInitialization(format!(
                "unselected surface initialization counts are nonzero: {nonzero:?}"
            )));
        }
        Ok(())
    }

    pub fn validate_model_request(
        &self,
        request: &ChatModelRequest,
        session_id: &AgentSessionId,
        snapshot_digest: &DigestHex,
    ) -> Result<(), ChatMinimalError> {
        request
            .validate()
            .map_err(|error| ChatMinimalError::ModelRequest(error.to_string()))?;
        if &request.causality.agent_session_id != session_id
            || &request
                .causality
                .resolved_snapshot_ref
                .snapshot_digest
                != snapshot_digest
            || request.causality.model_route_revision
                != request.route.model_route_revision
        {
            return Err(ChatMinimalError::ModelRequest(
                "request causality does not match the persistent Session and frozen Snapshot"
                    .to_owned(),
            ));
        }
        if !request.input.tools.is_empty()
            || request.input.tool_choice != ChatToolChoice::None
            || request.input.reasoning.is_some()
            || !matches!(
                request.input.response_format,
                nomifun_chat_model_broker::ChatResponseFormat::Text
            )
            || request.input.requested_output_modalities
                != BTreeSet::from([ChatModality::Text])
            || request.input.provider_round_parent.is_some()
            || request.input.preserve_native_responses_items
        {
            return Err(ChatMinimalError::ModelRequest(
                "final Broker request is not the exact zero-tool text request".to_owned(),
            ));
        }
        let expected_features = BTreeSet::from([
            ChatModelFeature::TextInput,
            ChatModelFeature::TextOutput,
        ]);
        if request.input.required_features() != expected_features {
            return Err(ChatMinimalError::ModelRequest(
                "model request requires features outside plain text input/output".to_owned(),
            ));
        }
        if request.input.messages.iter().any(|message| {
            matches!(message.role, ChatRole::System | ChatRole::Tool)
                || message.content.iter().any(|content| {
                    !matches!(content, ChatContentPart::Text { .. })
                })
        }) {
            return Err(ChatMinimalError::ModelRequest(
                "model request contains system, tool, media, or native Coding history".to_owned(),
            ));
        }
        if request
            .input
            .instructions
            .iter()
            .any(|instruction| contains_hidden_coding_context(instruction))
        {
            return Err(ChatMinimalError::ModelRequest(
                "model instructions contain a hidden Coding context marker".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_model_stream(
        &self,
        events: &[ChatModelEvent],
    ) -> Result<String, ChatMinimalError> {
        if events.is_empty()
            || !events.last().is_some_and(ChatModelEvent::is_terminal)
            || events[..events.len().saturating_sub(1)]
                .iter()
                .any(ChatModelEvent::is_terminal)
        {
            return Err(ChatMinimalError::ModelStream(
                "model stream has no single terminal event at the end".to_owned(),
            ));
        }
        if events.iter().any(|event| {
            matches!(
                event,
                ChatModelEvent::ToolCallDelta { .. }
                    | ChatModelEvent::ToolCallCompleted { .. }
                    | ChatModelEvent::OutputAudioDelta { .. }
                    | ChatModelEvent::NativeResponsesItem { .. }
            )
        }) {
            return Err(ChatMinimalError::ModelStream(
                "zero-tool stream emitted a tool, audio, or native Coding item".to_owned(),
            ));
        }
        if !matches!(
            events.last(),
            Some(ChatModelEvent::Completed {
                finish_reason: ChatFinishReason::Completed
            })
        ) {
            return Err(ChatMinimalError::ModelStream(
                "successful chat.minimal stream did not complete normally".to_owned(),
            ));
        }
        let text = events
            .iter()
            .filter_map(|event| match event {
                ChatModelEvent::OutputTextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        if text.is_empty() {
            return Err(ChatMinimalError::ModelStream(
                "model stream committed no text output".to_owned(),
            ));
        }
        Ok(text)
    }

    pub fn validate_success_observation(
        &self,
        observation: &SessionObservation,
        expected_text: &str,
    ) -> Result<ChatMinimalEventEvidence, ChatMinimalError> {
        validate_strict_event_sequence(&observation.events)?;
        let event_kinds = observation
            .events
            .iter()
            .map(|event| event.kind.0.clone())
            .collect::<BTreeSet<_>>();
        for required in [
            "session/opening",
            "capability/active-set-committed",
            "runtime/bound",
            "session/ready",
            "turn/started",
            "message/content-part",
            "message/completed",
            "turn/completed",
        ] {
            if !event_kinds.contains(required) {
                return Err(ChatMinimalError::Evidence(format!(
                    "missing canonical event {required}"
                )));
            }
        }
        if observation.events.iter().any(|event| {
            matches!(
                event.kind.0.as_str(),
                "tool/call-started"
                    | "tool/result-recorded"
                    | "effect/started"
                    | "effect/succeeded"
                    | "effect/failed"
                    | "effect/uncertain"
                    | "capability/activation-requested"
                    | "capability/activation-failed"
            )
        }) {
            return Err(ChatMinimalError::Evidence(
                "zero-tool Session persisted a tool, effect, or activation diagnostic".to_owned(),
            ));
        }
        validate_empty_active_set(&observation.events)?;
        if observation.head.status != "ready"
            || observation.head.active_turn_id.is_some()
            || observation.head.active_set_generation != 0
            || observation.head.last_seq != observation.next_cursor.seq
        {
            return Err(ChatMinimalError::Evidence(
                "final Session head is not the committed ready generation zero".to_owned(),
            ));
        }
        let projection_intents = observation
            .messages
            .iter()
            .map(|projection| projection.presentation_intent.clone())
            .collect::<BTreeSet<_>>();
        let message = observation
            .messages
            .iter()
            .find(|projection| {
                projection.presentation_intent == "message"
                    && projection.projection["state"] == "completed"
            })
            .ok_or_else(|| {
                ChatMinimalError::Evidence(
                    "completed message projection is missing".to_owned(),
                )
            })?;
        let streamed_text = message.projection["content"]
            .as_str()
            .ok_or_else(|| {
                ChatMinimalError::Evidence(
                    "message projection has no bounded text content".to_owned(),
                )
            })?
            .to_owned();
        if streamed_text != expected_text {
            return Err(ChatMinimalError::Evidence(
                "message projection differs from the Broker stream".to_owned(),
            ));
        }
        Ok(ChatMinimalEventEvidence {
            event_kinds,
            projection_intents,
            streamed_text,
        })
    }

    pub fn validate_cancel_observation(
        &self,
        observation: &SessionObservation,
        turn_correlation_id: &str,
    ) -> Result<(), ChatMinimalError> {
        validate_strict_event_sequence(&observation.events)?;
        let turn_events = observation
            .events
            .iter()
            .filter(|event| event.correlation_id.as_ref() == turn_correlation_id)
            .collect::<Vec<_>>();
        if !turn_events
            .iter()
            .any(|event| event.kind.0 == "turn/started")
            || !turn_events
                .iter()
                .any(|event| event.kind.0 == "turn/cancelled")
            || turn_events.iter().any(|event| {
                matches!(
                    event.kind.0.as_str(),
                    "turn/completed"
                        | "turn/failed"
                        | "tool/call-started"
                        | "effect/started"
                        | "effect/succeeded"
                )
            })
        {
            return Err(ChatMinimalError::Evidence(
                "cancelled turn did not terminate only through turn/cancelled".to_owned(),
            ));
        }
        if observation.head.status != "ready" || observation.head.active_turn_id.is_some() {
            return Err(ChatMinimalError::Evidence(
                "cancelled turn did not return the Session head to ready".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_rehydration(
        &self,
        rehydration: &SessionRehydrationInput,
        session_id: &AgentSessionId,
        snapshot_digest: &DigestHex,
    ) -> Result<(), ChatMinimalError> {
        if &rehydration.agent_session_id != session_id
            || &rehydration.resolved_snapshot_ref.snapshot_digest != snapshot_digest
            || &rehydration.through_cursor.agent_session_id != session_id
        {
            return Err(ChatMinimalError::Rehydration(
                "resume input changed Session or frozen Snapshot identity".to_owned(),
            ));
        }
        let compaction = rehydration.completed_compaction.as_ref().ok_or_else(|| {
            ChatMinimalError::Rehydration(
                "resume input has no latest completed compaction".to_owned(),
            )
        })?;
        if &compaction.agent_session_id != session_id {
            return Err(ChatMinimalError::Rehydration(
                "compaction belongs to another AgentSession".to_owned(),
            ));
        }
        if rehydration
            .subsequent_events
            .iter()
            .any(|event| event.seq <= compaction.through_seq)
        {
            return Err(ChatMinimalError::Rehydration(
                "resume input retained pre-compaction events".to_owned(),
            ));
        }
        let compaction_event = rehydration
            .subsequent_events
            .iter()
            .find(|event| event.kind.0 == "compaction/completed")
            .ok_or_else(|| {
                ChatMinimalError::Rehydration(
                    "completed compaction fact is absent from canonical replay".to_owned(),
                )
            })?;
        let SessionEventPayloadRef::InlineJson(payload) = &compaction_event.payload else {
            return Err(ChatMinimalError::Rehydration(
                "compaction/completed did not carry its canonical payload".to_owned(),
            ));
        };
        let event_compaction: CompactionCompletedPayload =
            serde_json::from_value(payload.0.clone()).map_err(|error| {
                ChatMinimalError::Rehydration(format!(
                    "invalid compaction/completed payload: {error}"
                ))
            })?;
        if &event_compaction != compaction {
            return Err(ChatMinimalError::Rehydration(
                "rehydration base differs from the committed compaction event".to_owned(),
            ));
        }
        let expected_cursor = rehydration
            .subsequent_events
            .last()
            .map_or(compaction.through_seq, |event| event.seq);
        if rehydration.through_cursor.seq != expected_cursor {
            return Err(ChatMinimalError::Rehydration(
                "rehydration cursor does not cover the canonical replay suffix".to_owned(),
            ));
        }
        if rehydration.subsequent_events.iter().any(|event| {
            matches!(
                event.kind.0.as_str(),
                "tool/call-started"
                    | "tool/result-recorded"
                    | "effect/started"
                    | "effect/succeeded"
                    | "effect/failed"
                    | "effect/uncertain"
            )
        }) {
            return Err(ChatMinimalError::Rehydration(
                "chat.minimal rehydration contains tool or effect facts".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_rebuild(
        &self,
        before: &SessionObservation,
        rebuilt: &SessionObservation,
    ) -> Result<(), ChatMinimalError> {
        if before != rebuilt {
            return Err(ChatMinimalError::Rebuild(
                "projection drop/rebuild changed the observable Session".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_delete(
        &self,
        result: &DeleteResult,
        session_id: &AgentSessionId,
        owner: &PrincipalRef,
    ) -> Result<(), ChatMinimalError> {
        if &result.tombstone.agent_session_id != session_id
            || &result.tombstone.owner_ref != owner
            || result.tombstone.state != AgentSessionDeletedState::Deleted
        {
            return Err(ChatMinimalError::Delete(
                "delete did not produce the exact D-024 tombstone identity".to_owned(),
            ));
        }
        let value = serde_json::to_value(&result.tombstone)
            .map_err(|error| ChatMinimalError::Delete(error.to_string()))?;
        let fields = value
            .as_object()
            .ok_or_else(|| {
                ChatMinimalError::Delete("tombstone is not a JSON object".to_owned())
            })?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected = BTreeSet::from([
            "agent_session_id",
            "deleted_at",
            "owner_ref",
            "state",
        ]);
        if fields != expected {
            return Err(ChatMinimalError::Delete(format!(
                "tombstone fields differ: expected {expected:?}, actual {fields:?}"
            )));
        }
        Ok(())
    }

    pub fn validate_deleted_error(
        &self,
        code: Option<&str>,
    ) -> Result<(), ChatMinimalError> {
        if code != Some(SESSION_DELETED) {
            return Err(ChatMinimalError::Delete(format!(
                "late operation returned {code:?}, expected {SESSION_DELETED}"
            )));
        }
        Ok(())
    }
}

fn validate_strict_event_sequence(
    events: &[SessionEventRecord],
) -> Result<(), ChatMinimalError> {
    if events
        .windows(2)
        .any(|pair| pair[1].seq != pair[0].seq.saturating_add(1))
    {
        return Err(ChatMinimalError::Evidence(
            "SessionEvent sequence has a gap or duplicate".to_owned(),
        ));
    }
    Ok(())
}

fn validate_empty_active_set(
    events: &[SessionEventRecord],
) -> Result<(), ChatMinimalError> {
    let active_events = events
        .iter()
        .filter(|event| event.kind.0 == "capability/active-set-committed")
        .collect::<Vec<_>>();
    if active_events.len() != 1 {
        return Err(ChatMinimalError::Evidence(format!(
            "chat.minimal must have exactly one committed generation-zero active set, found {}",
            active_events.len()
        )));
    }
    let SessionEventPayloadRef::InlineJson(payload) = &active_events[0].payload else {
        return Err(ChatMinimalError::Evidence(
            "active-set commit has no inline canonical payload".to_owned(),
        ));
    };
    if payload.0["generation"].as_u64() != Some(0)
        || !payload.0["active_capability_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)
        || !payload.0["delta"].as_array().is_some_and(Vec::is_empty)
    {
        return Err(ChatMinimalError::Evidence(
            "chat.minimal active set is not exact-empty generation zero".to_owned(),
        ));
    }
    Ok(())
}

fn contains_hidden_coding_context(instruction: &str) -> bool {
    let instruction = instruction.to_ascii_lowercase();
    [
        "agents.md",
        "workspace discovery",
        "worktree",
        "git status",
        "shell tool",
        "patch tool",
        "code mode",
        "review workflow",
        "sub-agent",
        "subagent",
        "skill catalog",
        "mcp warmup",
        "coding context",
        "codex coding",
    ]
    .into_iter()
    .any(|marker| instruction.contains(marker))
}
