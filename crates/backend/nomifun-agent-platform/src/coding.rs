use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use nomifun_agent_contracts::{
    ActionId, AgentBindingValue, AgentSessionDeletedState, AgentSessionId, CapabilityId,
    CapabilityRef, CheckpointRehydrateSource, CodingRuntimeFeatureInventoryPayload,
    CorrelationId, DeleteAgentSessionCommand, FullAutoExecutionWire, IdempotencyKey,
    NativeActionStart, NativeActionStartAck, OfficialPresetKey, OperationId, PrincipalRef,
    ResolvedSnapshotContent, RuntimeFeatureId, RuntimeProfileKind, SESSION_DELETED, SessionEventAppend,
    SessionEventPayloadRef, SessionEventRecord, SnapshotCompatibilityAdmissionResult,
    StrictJsonValue, UserId, digest_payload, official_preset_seed_manifest_payload,
};
use nomifun_agent_session::{
    CheckpointAdmission, DeleteResult, SessionCreateResult, SessionEventAppendResult,
    SessionObservation, SessionRehydrationInput,
};
use nomifun_api_types::{
    AgentPresetEditorResponse, AgentPresetSourceDto, OfficialPresetKeyDto,
    OfficialPresetTemplateDto, PreviewStatusDto, ResolveAgentPresetPreviewResponse,
};
use nomifun_codex_runtime::{
    CheckpointDisposition, DisposeRpcOutcome, PinnedRuntimeProfile, RuntimeDisposeReport,
    RuntimeIngressPort, RuntimeProfileLaunchPolicy, validate_native_action_ack,
};
use thiserror::Error;

use crate::{
    ActivateCapabilityRequest, AgentPlatform, AgentPlatformError, AgentSessionCommandPort,
    AgentSessionDeletePort, AgentSessionQueryPort, AgentTurnDispatch, OpenAgentSessionRequest,
    InvokeCapabilityCommand, StartAgentTurnRequest, TriadHarness,
};

const CODING_RUNTIME_FEATURE_INVENTORY_JSON: &str = include_str!(
    "../../nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json"
);
pub const CODING_CODEX_TEMPLATE_KEY: &str = "coding.codex";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodingSurface {
    FileSystem,
    Terminal,
    VersionControl,
    Mcp,
    Review,
    Diff,
    Test,
}

impl CodingSurface {
    pub const ALL: [Self; 7] = [
        Self::FileSystem,
        Self::Terminal,
        Self::VersionControl,
        Self::Mcp,
        Self::Review,
        Self::Diff,
        Self::Test,
    ];

    pub const EFFECTFUL: [Self; 5] = [
        Self::FileSystem,
        Self::Terminal,
        Self::VersionControl,
        Self::Mcp,
        Self::Test,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FileSystem => "fs",
            Self::Terminal => "terminal",
            Self::VersionControl => "vcs",
            Self::Mcp => "mcp",
            Self::Review => "review",
            Self::Diff => "diff",
            Self::Test => "test",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodingCodexContract {
    pub initial_capabilities: Vec<CapabilityRef>,
    pub on_demand_capabilities: Vec<CapabilityRef>,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
    pub native_actions: BTreeSet<ActionId>,
    pub responses_semantics: BTreeSet<String>,
    pub full_auto: FullAutoExecutionWire,
    pub target_contribution_manifest_digest: nomifun_agent_contracts::DigestHex,
    pub runtime_feature_inventory_digest: nomifun_agent_contracts::DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodingEventEvidence {
    pub event_kinds: BTreeSet<String>,
    pub tool_surfaces: BTreeSet<CodingSurface>,
    pub started_effect_surfaces: BTreeSet<CodingSurface>,
    pub succeeded_effect_surfaces: BTreeSet<CodingSurface>,
    pub projection_intents: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodingWorkspaceEvidence {
    pub dirty_before: BTreeMap<String, nomifun_agent_contracts::DigestHex>,
    pub content_after: BTreeMap<String, nomifun_agent_contracts::DigestHex>,
    pub agent_touched_paths: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodingReviewEvidence {
    pub diff_reviewed: bool,
    pub review_comment_count: u32,
    pub review_fixes_applied: bool,
    pub tests_run: bool,
    pub tests_passed: bool,
}

pub struct CodingCodexHarness<P> {
    platform: Arc<P>,
    owner: UserId,
    contract: CodingCodexContract,
}

pub type AgentPlatformCodingHarness = CodingCodexHarness<AgentPlatform>;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum CodingCodexError {
    #[error("frozen coding contract is invalid: {0}")]
    FrozenContract(String),
    #[error("coding Snapshot mismatch: {0}")]
    Snapshot(String),
    #[error("coding official template mismatch: {0}")]
    Template(String),
    #[error("coding ordinary Revision mismatch: {0}")]
    Revision(String),
    #[error("coding Preview mismatch: {0}")]
    Preview(String),
    #[error("coding RuntimeProfile mismatch: {0}")]
    RuntimeProfile(String),
    #[error("coding workspace preservation mismatch: {0}")]
    Workspace(String),
    #[error("coding review/diff/test mismatch: {0}")]
    Review(String),
    #[error("native action ACK mismatch: {0}")]
    NativeActionAck(String),
    #[error("coding event/projection evidence mismatch: {0}")]
    Evidence(String),
    #[error("checkpoint behavior mismatch: {0}")]
    Checkpoint(String),
    #[error("coding rehydration mismatch: {0}")]
    Rehydration(String),
    #[error("coding projection rebuild mismatch: {0}")]
    Rebuild(String),
    #[error("runtime dispose behavior mismatch: {0}")]
    Dispose(String),
    #[error("D-024 delete result mismatch: {0}")]
    Delete(String),
}

impl CodingCodexContract {
    pub fn frozen() -> Result<Self, CodingCodexError> {
        let seed_manifest = official_preset_seed_manifest_payload();
        seed_manifest
            .validate()
            .map_err(|error| CodingCodexError::FrozenContract(error.message))?;
        let seed = seed_manifest
            .templates
            .get(&OfficialPresetKey::CodingCodex)
            .ok_or_else(|| {
                CodingCodexError::FrozenContract(
                    "OfficialPresetSeedManifest has no coding.codex seed".to_owned(),
                )
            })?;
        let runtime: CodingRuntimeFeatureInventoryPayload =
            serde_json::from_str(CODING_RUNTIME_FEATURE_INVENTORY_JSON)
                .map_err(|error| CodingCodexError::FrozenContract(error.to_string()))?;
        runtime
            .validate()
            .map_err(|error| CodingCodexError::FrozenContract(error.message))?;
        if !seed
            .required_runtime_features
            .is_subset(&runtime.runtime_features)
        {
            return Err(CodingCodexError::FrozenContract(
                "coding.codex requires runtime features absent from the frozen inventory"
                    .to_owned(),
            ));
        }
        let initial_ids = capability_ids(&seed.initial_capabilities);
        let on_demand_ids = capability_ids(&seed.on_demand_capabilities);
        if !initial_ids.is_disjoint(&on_demand_ids) {
            return Err(CodingCodexError::FrozenContract(
                "coding.codex initial and on-demand sets overlap".to_owned(),
            ));
        }
        Ok(Self {
            initial_capabilities: seed.initial_capabilities.clone(),
            on_demand_capabilities: seed.on_demand_capabilities.clone(),
            required_runtime_features: seed.required_runtime_features.clone(),
            native_actions: runtime.native_actions,
            responses_semantics: runtime.responses_semantics,
            full_auto: runtime.full_auto,
            target_contribution_manifest_digest: seed_manifest
                .target_first_party_contribution_digest,
            runtime_feature_inventory_digest: seed_manifest
                .target_runtime_feature_inventory_digest,
        })
    }

    pub fn initial_ids(&self) -> BTreeSet<CapabilityId> {
        capability_ids(&self.initial_capabilities)
    }

    pub fn on_demand_ids(&self) -> BTreeSet<CapabilityId> {
        capability_ids(&self.on_demand_capabilities)
    }

    pub fn ceiling_ids(&self) -> BTreeSet<CapabilityId> {
        self.initial_ids()
            .union(&self.on_demand_ids())
            .cloned()
            .collect()
    }

    pub fn validate_template(
        &self,
        template: &OfficialPresetTemplateDto,
    ) -> Result<(), CodingCodexError> {
        if template.template_key != OfficialPresetKeyDto::CodingCodex
            || !template.immutable
            || !template.forkable
        {
            return Err(CodingCodexError::Template(
                "official coding.codex identity or lifecycle flags differ".to_owned(),
            ));
        }
        let initial = template
            .seed
            .initial_capabilities
            .iter()
            .map(|reference| CapabilityId::from(reference.id.clone()))
            .collect::<BTreeSet<_>>();
        let on_demand = template
            .seed
            .on_demand_capabilities
            .iter()
            .map(|reference| CapabilityId::from(reference.id.clone()))
            .collect::<BTreeSet<_>>();
        let runtime_features = template
            .seed
            .required_runtime_features
            .iter()
            .map(|feature| RuntimeFeatureId::from(feature.clone()))
            .collect::<BTreeSet<_>>();
        if initial != self.initial_ids()
            || on_demand != self.on_demand_ids()
            || runtime_features != self.required_runtime_features
            || template.role_coverage.required_capability_ids
                != self
                    .ceiling_ids()
                    .iter()
                    .map(|id| id.as_ref().to_owned())
                    .collect()
        {
            return Err(CodingCodexError::Template(
                "official coding.codex exact-set differs from frozen artifacts".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_ordinary_revision(
        &self,
        editor: &AgentPresetEditorResponse,
    ) -> Result<(), CodingCodexError> {
        let revision = editor.revision.as_ref().ok_or_else(|| {
            CodingCodexError::Revision(
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
            return Err(CodingCodexError::Revision(
                "template expansion did not produce the ordinary user-owned stable Revision"
                    .to_owned(),
            ));
        }
        let document = &revision.document;
        let initial = document
            .initial_capabilities
            .iter()
            .map(|selection| CapabilityId::from(selection.capability.id.clone()))
            .collect::<BTreeSet<_>>();
        let on_demand = document
            .on_demand_capabilities
            .iter()
            .map(|selection| CapabilityId::from(selection.capability.id.clone()))
            .collect::<BTreeSet<_>>();
        if initial != self.initial_ids()
            || on_demand != self.on_demand_ids()
            || document.model_route_refs.len() != 1
        {
            return Err(CodingCodexError::Revision(
                "ordinary Revision changed the frozen capability partition or model route"
                    .to_owned(),
            ));
        }
        validate_api_workspace_binding(&document.resource_bindings)?;
        Ok(())
    }

    pub fn validate_preview(
        &self,
        preview: &ResolveAgentPresetPreviewResponse,
    ) -> Result<(), CodingCodexError> {
        if preview.status != PreviewStatusDto::Ready
            || !preview.can_save_revision
            || !preview.can_create_session
            || !preview.diagnostics.is_empty()
            || preview.resolved_snapshot_ref.is_none()
        {
            return Err(CodingCodexError::Preview(
                "coding.codex Preview is not ready and executable".to_owned(),
            ));
        }
        let summary = &preview.summary;
        if summary.initial_count != self.initial_capabilities.len() as u32
            || summary.on_demand_count != self.on_demand_capabilities.len() as u32
            || summary.active_at_start_count != self.initial_capabilities.len() as u32
            || summary.on_demand_index_count != self.on_demand_capabilities.len() as u32
            || summary.model_tool_count == 0
            || summary.resource_binding_count == 0
            || summary.provider_initialization_count != 1
        {
            return Err(CodingCodexError::Preview(
                "Preview summary differs from the frozen Coding shape".to_owned(),
            ));
        }
        let inspector = &preview.inspector;
        let initial = inspector
            .initial_capabilities
            .iter()
            .map(|capability| CapabilityId::from(capability.capability.id.clone()))
            .collect::<BTreeSet<_>>();
        let on_demand = inspector
            .on_demand_capabilities
            .iter()
            .map(|capability| CapabilityId::from(capability.capability.id.clone()))
            .collect::<BTreeSet<_>>();
        let index = inspector
            .compact_on_demand_index
            .iter()
            .cloned()
            .map(CapabilityId::from)
            .collect::<BTreeSet<_>>();
        let runtime_features = inspector
            .required_runtime_features
            .iter()
            .cloned()
            .map(RuntimeFeatureId::from)
            .collect::<BTreeSet<_>>();
        if inspector.runtime_profile.as_deref() != Some("coding_native")
            || initial != self.initial_ids()
            || on_demand != self.on_demand_ids()
            || index != self.on_demand_ids()
            || runtime_features != self.required_runtime_features
            || inspector.tool_schema_refs.is_empty()
            || !inspector.service_key_diagnostics.is_empty()
        {
            return Err(CodingCodexError::Preview(
                "Preview inspector lost Coding capabilities, features, tools, or clean wiring"
                    .to_owned(),
            ));
        }
        validate_api_workspace_binding(&inspector.typed_resource_bindings)?;
        Ok(())
    }

    pub fn validate_snapshot(
        &self,
        snapshot: &ResolvedSnapshotContent,
    ) -> Result<(), CodingCodexError> {
        if snapshot.required_runtime_profile != RuntimeProfileKind::CodingNative {
            return Err(CodingCodexError::Snapshot(
                "coding.codex must compile to coding_native".to_owned(),
            ));
        }
        let actual_initial = snapshot
            .initial_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        let actual_on_demand = snapshot
            .on_demand_capabilities
            .iter()
            .map(|capability| capability.capability.id.clone())
            .collect::<BTreeSet<_>>();
        if actual_initial != self.initial_ids() {
            return Err(CodingCodexError::Snapshot(format!(
                "initial capability partition differs: expected {:?}, actual {:?}",
                self.initial_ids(),
                actual_initial
            )));
        }
        if actual_on_demand != self.on_demand_ids() {
            return Err(CodingCodexError::Snapshot(format!(
                "on-demand capability partition differs: expected {:?}, actual {:?}",
                self.on_demand_ids(),
                actual_on_demand
            )));
        }
        if snapshot.capability_allowlist != self.ceiling_ids() {
            return Err(CodingCodexError::Snapshot(
                "Snapshot capability ceiling differs from the frozen union".to_owned(),
            ));
        }
        if snapshot.required_runtime_features != self.required_runtime_features {
            return Err(CodingCodexError::Snapshot(
                "required runtime feature exact-set differs".to_owned(),
            ));
        }
        if snapshot.runtime_feature_inventory_digest
            != self.runtime_feature_inventory_digest
        {
            return Err(CodingCodexError::Snapshot(
                "runtime feature inventory digest differs".to_owned(),
            ));
        }
        if snapshot.target_contribution_manifest_digest
            != self.target_contribution_manifest_digest
        {
            return Err(CodingCodexError::Snapshot(
                "target contribution manifest digest differs".to_owned(),
            ));
        }
        let plan_ids = snapshot
            .on_demand_activation_plans
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let index_ids = snapshot
            .compact_on_demand_index
            .iter()
            .map(|entry| entry.capability_id.clone())
            .collect::<BTreeSet<_>>();
        if plan_ids != self.on_demand_ids() || index_ids != self.on_demand_ids() {
            return Err(CodingCodexError::Snapshot(
                "activation plans and compact index must exactly match on-demand roots"
                    .to_owned(),
            ));
        }
        for (root, plan) in &snapshot.on_demand_activation_plans {
            if &plan.root_capability_id != root
                || !plan.capability_bundle.contains(root)
            {
                return Err(CodingCodexError::Snapshot(format!(
                    "activation plan for {} is not rooted in its selected capability",
                    root.as_ref()
                )));
            }
        }
        validate_resource_defaults(snapshot, self)?;
        Ok(())
    }

    pub fn validate_runtime_profile(
        &self,
        profile: &PinnedRuntimeProfile,
    ) -> Result<(), CodingCodexError> {
        if profile.kind != RuntimeProfileKind::CodingNative
            || profile.full_auto() != self.full_auto
            || profile.initial_capabilities != self.initial_ids()
            || profile.on_demand_capabilities != self.on_demand_ids()
            || profile.enabled_runtime_features != self.required_runtime_features
        {
            return Err(CodingCodexError::RuntimeProfile(
                "PinnedRuntimeProfile differs from the frozen coding.codex contract"
                    .to_owned(),
            ));
        }
        let launch = profile.launch_policy();
        if launch != RuntimeProfileLaunchPolicy::coding_native()
            || !launch.review_workflow
            || !launch.workspace_discovery
            || !launch.tool_search
            || !launch.code_mode
            || !launch.subagents
        {
            return Err(CodingCodexError::RuntimeProfile(
                "coding_native launch policy lost Coding or Review behavior".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_workspace_preservation(
        &self,
        evidence: &CodingWorkspaceEvidence,
    ) -> Result<(), CodingCodexError> {
        for (path, before_digest) in &evidence.dirty_before {
            let Some(after_digest) = evidence.content_after.get(path) else {
                return Err(CodingCodexError::Workspace(format!(
                    "pre-existing dirty path {path} disappeared"
                )));
            };
            if !evidence.agent_touched_paths.contains(path)
                && after_digest != before_digest
            {
                return Err(CodingCodexError::Workspace(format!(
                    "pre-existing dirty path {path} changed outside the Agent write set"
                )));
            }
        }
        Ok(())
    }

    pub fn validate_review_workflow(
        &self,
        evidence: &CodingReviewEvidence,
    ) -> Result<(), CodingCodexError> {
        if !evidence.diff_reviewed
            || evidence.review_comment_count == 0
            || !evidence.review_fixes_applied
            || !evidence.tests_run
            || !evidence.tests_passed
        {
            return Err(CodingCodexError::Review(
                "code review, diff review, review comments, fixes, and tests must all complete"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_native_action_ack(
        &self,
        start: &NativeActionStart,
        ack: &NativeActionStartAck,
    ) -> Result<(), CodingCodexError> {
        if !self.native_actions.contains(&start.action_id) {
            return Err(CodingCodexError::NativeActionAck(format!(
                "native action {} is outside the frozen inventory",
                start.action_id.as_ref()
            )));
        }
        validate_native_action_ack(start, ack)
            .map_err(|error| CodingCodexError::NativeActionAck(error.to_string()))
    }

    pub fn validate_success_observation(
        &self,
        observation: &SessionObservation,
    ) -> Result<CodingEventEvidence, CodingCodexError> {
        validate_strict_event_sequence(&observation.events)?;
        let event_kinds = observation
            .events
            .iter()
            .map(|event| event.kind.0.clone())
            .collect::<BTreeSet<_>>();
        for required in [
            "session/opening",
            "session/ready",
            "capability/active-set-committed",
            "runtime/bound",
            "turn/started",
            "tool/call-started",
            "tool/result-recorded",
            "effect/started",
            "effect/succeeded",
            "turn/completed",
        ] {
            if !event_kinds.contains(required) {
                return Err(CodingCodexError::Evidence(format!(
                    "missing canonical event {required}"
                )));
            }
        }
        let tool_surfaces = surfaces_for_kind(&observation.events, "tool/result-recorded");
        let started_effect_surfaces =
            surfaces_for_kind(&observation.events, "effect/started");
        let succeeded_effect_surfaces =
            surfaces_for_kind(&observation.events, "effect/succeeded");
        let expected_tools = CodingSurface::ALL.into_iter().collect::<BTreeSet<_>>();
        let expected_effects = CodingSurface::EFFECTFUL
            .into_iter()
            .collect::<BTreeSet<_>>();
        if tool_surfaces != expected_tools {
            return Err(CodingCodexError::Evidence(format!(
                "Coding tool surface differs: expected {expected_tools:?}, actual {tool_surfaces:?}"
            )));
        }
        if started_effect_surfaces != expected_effects
            || succeeded_effect_surfaces != expected_effects
        {
            return Err(CodingCodexError::Evidence(
                "effect receipts do not cover the exact effectful Coding surfaces"
                    .to_owned(),
            ));
        }
        let projection_intents = observation
            .messages
            .iter()
            .map(|projection| projection.presentation_intent.clone())
            .collect::<BTreeSet<_>>();
        for required in ["turn_status", "tool", "effect", "runtime", "capability"] {
            if !projection_intents.contains(required) {
                return Err(CodingCodexError::Evidence(format!(
                    "missing {required} projection"
                )));
            }
        }
        let active_generation = expected_active_generation(&observation.events)?;
        if !self.on_demand_capabilities.is_empty() && active_generation == 0 {
            return Err(CodingCodexError::Evidence(
                "coding.codex on-demand partition was never activated at a turn boundary"
                    .to_owned(),
            ));
        }
        if observation.head.status != "ready"
            || observation.head.active_turn_id.is_some()
            || observation.head.active_set_generation != active_generation
        {
            return Err(CodingCodexError::Evidence(
                "final Session head is not the committed ready generation".to_owned(),
            ));
        }
        Ok(CodingEventEvidence {
            event_kinds,
            tool_surfaces,
            started_effect_surfaces,
            succeeded_effect_surfaces,
            projection_intents,
        })
    }

    pub fn validate_cancel_observation(
        &self,
        observation: &SessionObservation,
    ) -> Result<(), CodingCodexError> {
        validate_strict_event_sequence(&observation.events)?;
        let cancelled = observation
            .events
            .iter()
            .find(|event| event.kind.0 == "turn/cancelled")
            .ok_or_else(|| {
                CodingCodexError::Evidence(
                    "cancel scenario has no turn/cancelled event".to_owned(),
                )
            })?;
        if observation
            .events
            .iter()
            .any(|event| {
                event.seq > cancelled.seq
                    && matches!(
                        event.kind.0.as_str(),
                        "effect/started" | "effect/succeeded"
                    )
            })
        {
            return Err(CodingCodexError::Evidence(
                "effect work continued after turn cancellation".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_checkpoint_admission(
        &self,
        admission: &CheckpointAdmission,
    ) -> Result<(), CodingCodexError> {
        if !admission.checkpoint_reusable
            || !matches!(
                admission.compatibility.as_ref(),
                Some(SnapshotCompatibilityAdmissionResult::CompatibleExact { .. })
            )
        {
            return Err(CodingCodexError::Checkpoint(
                "exact checkpoint was not admitted by complete-ceiling compatibility"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_checkpoint_discard(
        &self,
        disposition: &CheckpointDisposition,
    ) -> Result<(), CodingCodexError> {
        let CheckpointDisposition::Discard {
            rehydrate_from,
            checkpoint_converter_allowed,
            ..
        } = disposition
        else {
            return Err(CodingCodexError::Checkpoint(
                "mismatched checkpoint was resumed".to_owned(),
            ));
        };
        let expected = vec![
            CheckpointRehydrateSource::ExactSnapshot,
            CheckpointRehydrateSource::LatestCompletedCompaction,
            CheckpointRehydrateSource::SubsequentCanonicalEvents,
        ];
        if *checkpoint_converter_allowed || rehydrate_from != &expected {
            return Err(CodingCodexError::Checkpoint(
                "checkpoint discard did not use canonical Event rehydration"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_rehydration(
        &self,
        rehydration: &SessionRehydrationInput,
        session_id: &AgentSessionId,
        snapshot_digest: &nomifun_agent_contracts::DigestHex,
    ) -> Result<(), CodingCodexError> {
        if &rehydration.agent_session_id != session_id
            || &rehydration.resolved_snapshot_ref.snapshot_digest != snapshot_digest
            || &rehydration.through_cursor.agent_session_id != session_id
        {
            return Err(CodingCodexError::Rehydration(
                "rehydration changed Session or frozen Snapshot identity".to_owned(),
            ));
        }
        if let Some(compaction) = &rehydration.completed_compaction
            && rehydration
                .subsequent_events
                .iter()
                .any(|event| event.seq <= compaction.through_seq)
        {
            return Err(CodingCodexError::Rehydration(
                "rehydration retained events already covered by compaction".to_owned(),
            ));
        }
        let expected_cursor = rehydration
            .subsequent_events
            .last()
            .map(|event| event.seq)
            .or_else(|| {
                rehydration
                    .completed_compaction
                    .as_ref()
                    .map(|compaction| compaction.through_seq)
            })
            .unwrap_or(0);
        if rehydration.through_cursor.seq != expected_cursor {
            return Err(CodingCodexError::Rehydration(
                "rehydration cursor does not cover the canonical replay suffix".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_rebuild(
        &self,
        before: &SessionObservation,
        rebuilt: &SessionObservation,
    ) -> Result<(), CodingCodexError> {
        if before != rebuilt {
            return Err(CodingCodexError::Rebuild(
                "projection drop/rebuild changed the observable Coding Session".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_dispose(
        &self,
        report: &RuntimeDisposeReport,
        session_id: &AgentSessionId,
        require_rpc_ack: bool,
    ) -> Result<(), CodingCodexError> {
        if &report.agent_session_id != session_id {
            return Err(CodingCodexError::Dispose(
                "dispose report belongs to another AgentSession".to_owned(),
            ));
        }
        if require_rpc_ack && report.rpc != DisposeRpcOutcome::Acked {
            return Err(CodingCodexError::Dispose(
                "stable dispose RPC was not acknowledged".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_delete(
        &self,
        result: &DeleteResult,
        session_id: &AgentSessionId,
        owner: &PrincipalRef,
    ) -> Result<(), CodingCodexError> {
        if &result.tombstone.agent_session_id != session_id
            || &result.tombstone.owner_ref != owner
            || result.tombstone.state != AgentSessionDeletedState::Deleted
        {
            return Err(CodingCodexError::Delete(
                "delete did not produce the exact D-024 tombstone identity"
                    .to_owned(),
            ));
        }
        let value = serde_json::to_value(&result.tombstone)
            .map_err(|error| CodingCodexError::Delete(error.to_string()))?;
        let fields = value
            .as_object()
            .ok_or_else(|| {
                CodingCodexError::Delete("tombstone is not a JSON object".to_owned())
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
            return Err(CodingCodexError::Delete(format!(
                "tombstone fields differ: expected {expected:?}, actual {fields:?}"
            )));
        }
        Ok(())
    }

    pub fn validate_deleted_error(
        &self,
        code: Option<&str>,
    ) -> Result<(), CodingCodexError> {
        if code != Some(SESSION_DELETED) {
            return Err(CodingCodexError::Delete(format!(
                "late operation returned {code:?}, expected {SESSION_DELETED}"
            )));
        }
        Ok(())
    }
}

impl<P> CodingCodexHarness<P> {
    pub fn new(
        platform: Arc<P>,
        owner: UserId,
    ) -> Result<Self, CodingCodexError> {
        Ok(Self {
            platform,
            owner,
            contract: CodingCodexContract::frozen()?,
        })
    }

    pub fn platform(&self) -> &Arc<P> {
        &self.platform
    }

    pub fn owner(&self) -> &UserId {
        &self.owner
    }

    pub fn contract(&self) -> &CodingCodexContract {
        &self.contract
    }
}

impl<P> CodingCodexHarness<P>
where
    P: AgentSessionCommandPort
        + AgentSessionQueryPort
        + AgentSessionDeletePort
        + RuntimeIngressPort,
{
    pub async fn open_session(
        &self,
        agent_binding: AgentBindingValue,
        title: Option<String>,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Result<SessionCreateResult, AgentPlatformError> {
        let mut request =
            OpenAgentSessionRequest::user(&self.owner, agent_binding, idempotency_key);
        request.metadata.title = title;
        self.platform.open_session(request).await
    }

    pub async fn append_event(
        &self,
        append: &SessionEventAppend,
    ) -> Result<SessionEventAppendResult, AgentPlatformError> {
        self.platform.append_event(append).await
    }

    pub async fn start_turn(
        &self,
        agent_session_id: AgentSessionId,
        input: StrictJsonValue,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Result<AgentTurnDispatch, AgentPlatformError> {
        self.platform
            .start_turn(StartAgentTurnRequest {
                agent_session_id,
                principal: coding_principal(&self.owner),
                input,
                idempotency_key: idempotency_key.into(),
            })
            .await
    }

    pub async fn activate_at_boundary(
        &self,
        agent_session_id: AgentSessionId,
        capability_id: CapabilityId,
        expected_generation: u64,
        completed_turn_operation_id: OperationId,
        idempotency_key: impl Into<IdempotencyKey>,
    ) -> Result<nomifun_agent_kernel::ActivationOutcome, AgentPlatformError> {
        self.platform
            .activate_capability(ActivateCapabilityRequest {
                agent_session_id,
                principal: coding_principal(&self.owner),
                capability_id,
                expected_generation,
                completed_turn_operation_id,
                idempotency_key: idempotency_key.into(),
            })
            .await
    }

    pub async fn invoke_capability(
        &self,
        agent_session_id: &AgentSessionId,
        request: nomifun_agent_kernel::CapabilityInvocationRequest,
    ) -> Result<StrictJsonValue, AgentPlatformError> {
        let request_digest = digest_payload(&serde_json::json!({
            "agent_session_id": agent_session_id,
            "principal": &request.principal,
            "session_owner": &request.session_owner,
            "resolved_snapshot_ref": &request.resolved_snapshot_ref,
            "active_set_generation": request.active_set_generation,
            "capability_id": &request.capability_id,
            "action_id": &request.action_id,
            "resource_binding_ids": &request.resource_binding_ids,
            "state_scope_key": &request.state_scope_key,
            "input": &request.input,
        }))?;
        let invocation_key = format!(
            "capability-invoke:{}:{}",
            agent_session_id.as_ref(),
            request_digest.as_ref()
        );
        self.platform
            .invoke_capability(InvokeCapabilityCommand {
                agent_session_id: agent_session_id.clone(),
                invocation: request,
                operation_id: OperationId::from(invocation_key.clone()),
                idempotency_key: IdempotencyKey::from(invocation_key.clone()),
                correlation_id: CorrelationId::from(invocation_key),
            })
            .await
    }

    pub async fn commit_native_action_start(
        &self,
        start: NativeActionStart,
    ) -> Result<NativeActionStartAck, AgentPlatformError> {
        let ack = self
            .platform
            .commit_native_action_start(start.clone())
            .await?;
        self.contract
            .validate_native_action_ack(&start, &ack)
            .map_err(|error| AgentPlatformError::Contract(error.to_string()))?;
        Ok(ack)
    }

    pub async fn observe(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionObservation, AgentPlatformError> {
        self.platform
            .observe_session(
                &coding_principal(&self.owner),
                session_id,
                None,
                nomifun_agent_session::MAX_EVENT_PAGE_SIZE,
            )
            .await
    }

    pub async fn rehydration_input(
        &self,
        session_id: &AgentSessionId,
    ) -> Result<SessionRehydrationInput, AgentPlatformError> {
        self.platform
            .rehydration_input(&coding_principal(&self.owner), session_id)
            .await
    }

    pub async fn delete(
        &self,
        session_id: AgentSessionId,
        requested_at: i64,
        deleted_at: i64,
    ) -> Result<DeleteResult, AgentPlatformError> {
        let result = self
            .platform
            .delete_session(
                DeleteAgentSessionCommand {
                    operation_id: OperationId::from(format!(
                        "delete:{}",
                        session_id.as_ref()
                    )),
                    agent_session_id: session_id.clone(),
                    owner_ref: coding_principal(&self.owner),
                    requested_at,
                },
                deleted_at,
            )
            .await?;
        self.contract
            .validate_delete(&result, &session_id, &coding_principal(&self.owner))
            .map_err(|error| AgentPlatformError::Contract(error.to_string()))?;
        Ok(result)
    }
}

impl TriadHarness {
    pub fn coding_codex(
        &self,
    ) -> Result<AgentPlatformCodingHarness, CodingCodexError> {
        CodingCodexHarness::new(Arc::clone(self.platform()), self.owner().clone())
    }
}

fn capability_ids(capabilities: &[CapabilityRef]) -> BTreeSet<CapabilityId> {
    capabilities
        .iter()
        .map(|capability| capability.id.clone())
        .collect()
}

fn coding_principal(owner: &UserId) -> PrincipalRef {
    PrincipalRef {
        principal_kind: "user".to_owned(),
        principal_id: owner.as_ref().to_owned(),
    }
}

fn validate_resource_defaults(
    snapshot: &ResolvedSnapshotContent,
    contract: &CodingCodexContract,
) -> Result<(), CodingCodexError> {
    let seed_manifest = official_preset_seed_manifest_payload();
    let seed = &seed_manifest.templates[&OfficialPresetKey::CodingCodex];
    let bindings = snapshot
        .typed_resource_bindings
        .iter()
        .map(|binding| (binding.resource_kind.as_ref(), binding))
        .collect::<BTreeMap<_, _>>();
    for required in seed
        .typed_resource_defaults
        .iter()
        .filter(|resource| resource.required)
    {
        let Some(binding) = bindings.get(required.resource_kind.as_ref()) else {
            return Err(CodingCodexError::Snapshot(format!(
                "required Coding resource kind {} is unbound",
                required.resource_kind.as_ref()
            )));
        };
        if !required.operations.is_subset(&binding.operations) {
            return Err(CodingCodexError::Snapshot(format!(
                "resource {} lacks frozen Coding operations",
                binding.binding_id.as_ref()
            )));
        }
    }
    if contract.initial_capabilities.is_empty() {
        return Err(CodingCodexError::FrozenContract(
            "coding.codex initial partition must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn validate_api_workspace_binding(
    bindings: &[nomifun_api_types::TypedResourceBindingDto],
) -> Result<(), CodingCodexError> {
    let binding = bindings
        .iter()
        .find(|binding| binding.resource_kind == "workspace")
        .ok_or_else(|| {
            CodingCodexError::Revision(
                "required workspace resource is not bound".to_owned(),
            )
        })?;
    let required = BTreeSet::from([
        "execute".to_owned(),
        "read".to_owned(),
        "write".to_owned(),
    ]);
    if !required.is_subset(&binding.operations) {
        return Err(CodingCodexError::Revision(
            "workspace binding lacks execute/read/write operations".to_owned(),
        ));
    }
    Ok(())
}

fn validate_strict_event_sequence(
    events: &[SessionEventRecord],
) -> Result<(), CodingCodexError> {
    if events
        .windows(2)
        .any(|pair| pair[1].seq != pair[0].seq.saturating_add(1))
    {
        return Err(CodingCodexError::Evidence(
            "SessionEvent sequence has a gap or duplicate".to_owned(),
        ));
    }
    Ok(())
}

fn surfaces_for_kind(
    events: &[SessionEventRecord],
    kind: &str,
) -> BTreeSet<CodingSurface> {
    events
        .iter()
        .filter(|event| event.kind.0 == kind)
        .filter_map(event_surface)
        .collect()
}

fn event_surface(event: &SessionEventRecord) -> Option<CodingSurface> {
    let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
        return None;
    };
    let value = payload.0.get("coding_surface")?.as_str()?;
    CodingSurface::ALL
        .into_iter()
        .find(|surface| surface.as_str() == value)
}

fn expected_active_generation(
    events: &[SessionEventRecord],
) -> Result<u64, CodingCodexError> {
    events
        .iter()
        .filter(|event| event.kind.0 == "capability/active-set-committed")
        .filter_map(|event| {
            let SessionEventPayloadRef::InlineJson(payload) = &event.payload else {
                return None;
            };
            payload.0.get("generation").and_then(serde_json::Value::as_u64)
        })
        .max()
        .ok_or_else(|| {
            CodingCodexError::Evidence(
                "Session has no capability/active-set-committed generation".to_owned(),
            )
        })
}
