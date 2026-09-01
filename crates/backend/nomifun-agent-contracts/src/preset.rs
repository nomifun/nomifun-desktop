use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::digest::digest_payload;
use crate::package::{
    CapabilityRef, PackageRef, SkillRef, TargetPackageInventoryPayload,
};
use crate::runtime::RuntimeProfileKind;
use crate::{
    ActionId, AgentPresetId, ArtifactEnvelope, CanonicalErrorCode, CanonicalSchemaRef, DigestHex,
    ChatRouteIdentity, ChatRouteRecord, McpServerId, McpToolKey, ModelRouteId, OperationId,
    PrincipalRef, ResolvedSnapshotId, ResourceBindingId, ResourceKind, RuntimeFeatureId,
    StrictJsonValue, TypedResourceBindings, UserId, VersionString,
};

pub const CAPABILITY_NOT_MATERIALIZED: &str = "CAPABILITY_NOT_MATERIALIZED";
pub const CAPABILITY_NOT_IN_PRESET: &str = "CAPABILITY_NOT_IN_PRESET";
pub const CAPABILITY_NOT_ACTIVE: &str = "CAPABILITY_NOT_ACTIVE";
pub const CAPABILITY_UNAVAILABLE_ON_PLATFORM: &str = "CAPABILITY_UNAVAILABLE_ON_PLATFORM";
pub const PRESET_CAPABILITY_DUPLICATE: &str = "PRESET_CAPABILITY_DUPLICATE";
pub const PRESET_CAPABILITY_SET_OVERLAP: &str = "PRESET_CAPABILITY_SET_OVERLAP";
pub const PRESET_REVISION_DIGEST_MISMATCH: &str = "PRESET_REVISION_DIGEST_MISMATCH";
pub const PRESET_REVISION_SAVE_FAILED: &str = "PRESET_REVISION_SAVE_FAILED";
pub const PRESET_RESOURCE_NOT_BOUND: &str = "PRESET_RESOURCE_NOT_BOUND";
pub const RESOURCE_OWNER_MISMATCH: &str = "RESOURCE_OWNER_MISMATCH";
pub const OFFICIAL_PRESET_KEY_SET_MISMATCH: &str = "OFFICIAL_PRESET_KEY_SET_MISMATCH";
pub const CHAT_MINIMAL_NOT_EXACT_EMPTY: &str = "CHAT_MINIMAL_NOT_EXACT_EMPTY";
pub const CODING_CODEX_NATIVE_INCOMPLETE: &str = "CODING_CODEX_NATIVE_INCOMPLETE";
pub const ROLE_COVERAGE_INCOMPLETE: &str = "ROLE_COVERAGE_INCOMPLETE";
pub const MODEL_ROUTE_RECORD_INVALID: &str = "MODEL_ROUTE_RECORD_INVALID";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentPresetSource {
    Official,
    User,
    Package,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPreset {
    pub preset_id: AgentPresetId,
    pub owner_user_id: Option<UserId>,
    pub source: AgentPresetSource,
    pub display_name: String,
    pub description: Option<String>,
    pub current_stable_revision: Option<PresetRevisionRef>,
}

#[derive(
    Clone,
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
#[serde(deny_unknown_fields)]
pub struct PresetRevisionRef {
    pub preset_id: AgentPresetId,
    pub revision: u64,
    pub revision_digest: DigestHex,
}

impl PresetRevisionRef {
    pub fn revision_id(&self) -> String {
        format!("{}@{}", self.preset_id.as_ref(), self.revision)
    }
}

#[derive(
    Clone,
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
#[serde(deny_unknown_fields)]
pub struct ResolvedSnapshotRef {
    pub snapshot_id: ResolvedSnapshotId,
    pub snapshot_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentBindingValue {
    pub preset_revision_ref: PresetRevisionRef,
    pub resolved_snapshot_ref: ResolvedSnapshotRef,
    pub typed_resource_bindings: TypedResourceBindings,
    pub binding_version: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySelection {
    pub capability: CapabilityRef,
    pub required: bool,
    pub exposure: CapabilityExposure,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub action_allowlist: BTreeSet<ActionId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_binding_refs: Vec<ResourceBindingId>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub destination_constraints: BTreeSet<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_budget_override: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_budget_override: Option<u32>,
    pub config: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityExposure {
    Advertised,
    Discoverable,
    Hidden,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceDefaultBindingPolicy {
    RequireExplicitSelection,
    SelectOnlyOwnedResource,
    LeaveUnbound,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypedResourceDefault {
    pub slot_key: String,
    pub resource_kind: ResourceKind,
    pub required: bool,
    pub operations: BTreeSet<String>,
    pub binding_policy: ResourceDefaultBindingPolicy,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevisionPayload {
    pub schema_version: VersionString,
    pub surfaces: BTreeSet<String>,
    pub model_route_refs: BTreeMap<String, ModelRouteId>,
    /// Complete route facts used by the Fresh-v4 persistence writer. Legacy
    /// opaque IDs are not sufficient to construct a provider request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat_route_records: BTreeMap<String, ChatRouteRecord>,
    pub initial_capabilities: Vec<CapabilitySelection>,
    pub on_demand_capabilities: Vec<CapabilitySelection>,
    pub skill_bindings: Vec<SkillRef>,
    pub resource_bindings: TypedResourceBindings,
    pub persona: String,
    pub instructions: String,
    pub context_policy: StrictJsonValue,
    pub execution_constraints: StrictJsonValue,
    pub runtime_budget: StrictJsonValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevision {
    pub reference: PresetRevisionRef,
    pub payload: AgentPresetRevisionPayload,
    pub created_by: UserId,
    pub created_at_ms: i64,
    pub reason: Option<String>,
}

impl AgentPresetRevision {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        validate_chat_route_records_for_revision(
            &self.payload,
            Some(&self.reference.revision_id()),
        )?;
        validate_capability_selections(
            &self.payload.initial_capabilities,
            &self.payload.on_demand_capabilities,
        )?;
        let digest = digest_payload(&self.payload).map_err(|error| PresetContractViolation {
            code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
            message: error.to_string(),
        })?;
        if digest != self.reference.revision_digest {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: "revision_digest must cover only the immutable revision payload".into(),
            });
        }
        Ok(())
    }

    pub fn chat_route_identity(
        &self,
    ) -> Result<Option<ChatRouteIdentity>, PresetContractViolation> {
        validate_chat_route_records_for_revision(
            &self.payload,
            Some(&self.reference.revision_id()),
        )?;
        let Some(route_id) = self
            .payload
            .model_route_refs
            .get(crate::CHAT_MODEL_TASK_AGENT_CHAT)
        else {
            return Ok(None);
        };
        let record = self
            .payload
            .chat_route_records
            .get(crate::CHAT_MODEL_TASK_AGENT_CHAT)
            .expect("route-record validation keeps route references paired");
        Ok(Some(ChatRouteIdentity::new(
            self.reference.revision_id(),
            crate::CHAT_MODEL_TASK_AGENT_CHAT,
            route_id.clone(),
            record.primary.model_route_revision,
        )))
    }
}

pub fn validate_chat_route_records(
    payload: &AgentPresetRevisionPayload,
) -> Result<(), PresetContractViolation> {
    validate_chat_route_records_for_revision(payload, None)
}

fn validate_chat_route_records_for_revision(
    payload: &AgentPresetRevisionPayload,
    preset_revision_id: Option<&str>,
) -> Result<(), PresetContractViolation> {
    let chat_route = payload
        .model_route_refs
        .get(crate::CHAT_MODEL_TASK_AGENT_CHAT);
    let chat_record = payload
        .chat_route_records
        .get(crate::CHAT_MODEL_TASK_AGENT_CHAT);
    if chat_route.is_some() != chat_record.is_some() {
        return Err(PresetContractViolation {
            code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
            message:
                "agent_chat must have exactly one opaque route reference and one canonical route record"
                    .into(),
        });
    }

    if let (Some(route_id), Some(record)) = (chat_route, chat_record) {
        record
            .validate()
            .map_err(|error| PresetContractViolation {
                code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                message: error.to_string(),
            })?;
        if record.primary.model_route_id != *route_id {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                message: "agent_chat route reference does not match the canonical record".into(),
            });
        }
        if let Some(preset_revision_id) = preset_revision_id {
            let identity = ChatRouteIdentity::new(
                preset_revision_id,
                crate::CHAT_MODEL_TASK_AGENT_CHAT,
                route_id.clone(),
                record.primary.model_route_revision,
            );
            identity
                .validate()
                .map_err(|error| PresetContractViolation {
                    code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                    message: error.to_string(),
                })?;
            record
                .validate_for(&identity)
                .map_err(|error| PresetContractViolation {
                    code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                    message: error.to_string(),
                })?;
        }
    }

    for task in payload.chat_route_records.keys() {
        if task != crate::CHAT_MODEL_TASK_AGENT_CHAT {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                message: format!(
                    "canonical Chat route records do not support model task {task:?}"
                ),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCapability {
    pub capability: CapabilityRef,
    pub source_package: PackageRef,
    pub schema_digest: DigestHex,
    pub dependency_path: Vec<crate::CapabilityId>,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompactOnDemandCapabilityEntry {
    pub capability_id: crate::CapabilityId,
    pub display_name: String,
    pub short_description: String,
    pub search_terms: Vec<String>,
    pub activation_plan_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrecomputedActivationPlan {
    pub root_capability_id: crate::CapabilityId,
    pub capability_bundle: Vec<crate::CapabilityId>,
    pub tool_schema_refs: Vec<CanonicalSchemaRef>,
    pub context_schema_refs: Vec<CanonicalSchemaRef>,
    pub resource_binding_refs: Vec<ResourceBindingId>,
    pub model_route_refs: Vec<ModelRouteId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSkillLock {
    pub skill: SkillRef,
    pub body_digest: DigestHex,
    pub required_capabilities: BTreeSet<crate::CapabilityId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedMcpToolLock {
    pub server_id: McpServerId,
    pub canonical_tool_key: McpToolKey,
    pub capability_id: crate::CapabilityId,
    pub schema_digest: DigestHex,
    pub materialization_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSnapshotContent {
    pub schema_version: VersionString,
    pub resolver_version: VersionString,
    pub preset_revision_ref: PresetRevisionRef,
    pub required_runtime_protocol_version: VersionString,
    pub required_runtime_profile: RuntimeProfileKind,
    pub runtime_feature_inventory_digest: DigestHex,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
    pub compiled_runtime_profile_digest: DigestHex,
    pub model_route_refs: BTreeMap<String, ModelRouteId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_route_identity: Option<ChatRouteIdentity>,
    pub initial_capabilities: Vec<ResolvedCapability>,
    pub on_demand_capabilities: Vec<ResolvedCapability>,
    pub on_demand_activation_plans:
        BTreeMap<crate::CapabilityId, PrecomputedActivationPlan>,
    pub compact_on_demand_index: Vec<CompactOnDemandCapabilityEntry>,
    pub capability_allowlist: BTreeSet<crate::CapabilityId>,
    pub skill_locks: Vec<ResolvedSkillLock>,
    pub mcp_tool_locks: Vec<ResolvedMcpToolLock>,
    pub typed_resource_bindings: TypedResourceBindings,
    pub canonical_schema_manifest_digest: DigestHex,
    pub target_contribution_manifest_digest: DigestHex,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSnapshotEnvelope {
    pub snapshot_ref: ResolvedSnapshotRef,
    pub content: ResolvedSnapshotContent,
    pub actor: PrincipalRef,
    pub scene: String,
    pub surface: String,
    pub audience: String,
    pub created_at_ms: i64,
    pub resolver_run_id: OperationId,
    pub availability_evidence_revision: String,
}

impl ResolvedSnapshotEnvelope {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        validate_resolved_capability_sets(
            &self.content.initial_capabilities,
            &self.content.on_demand_capabilities,
        )?;
        validate_snapshot_chat_route_identity(&self.content)?;
        let digest = digest_payload(&self.content).map_err(|error| PresetContractViolation {
            code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
            message: error.to_string(),
        })?;
        if digest != self.snapshot_ref.snapshot_digest {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: "snapshot_digest must cover only ResolvedSnapshotContent".into(),
            });
        }
        Ok(())
    }
}

fn validate_snapshot_chat_route_identity(
    content: &ResolvedSnapshotContent,
) -> Result<(), PresetContractViolation> {
    let expected_revision_id = content.preset_revision_ref.revision_id();
    let route_ref = content
        .model_route_refs
        .get(crate::CHAT_MODEL_TASK_AGENT_CHAT);
    match (&content.chat_route_identity, route_ref) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(PresetContractViolation {
            code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
            message: "Snapshot model route references require a chat route identity".into(),
        }),
        (Some(identity), Some(route_id)) => {
            identity.validate().map_err(|error| PresetContractViolation {
                code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                message: error.to_string(),
            })?;
            if identity.preset_revision_id != expected_revision_id
                || identity.model_task != crate::CHAT_MODEL_TASK_AGENT_CHAT
                || &identity.route_id != route_id
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
                    message:
                        "Snapshot chat route identity does not match its Preset Revision or route reference"
                            .into(),
                });
            }
            Ok(())
        }
        (Some(_), None) => Err(PresetContractViolation {
            code: CanonicalErrorCode::from(MODEL_ROUTE_RECORD_INVALID),
            message: "Snapshot chat route identity has no matching route reference".into(),
        }),
    }
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
pub enum OfficialPresetKey {
    #[serde(rename = "chat.minimal")]
    ChatMinimal,
    #[serde(rename = "assistant.general")]
    AssistantGeneral,
    #[serde(rename = "coding.codex")]
    CodingCodex,
    #[serde(rename = "companion.default")]
    CompanionDefault,
    #[serde(rename = "robot.default")]
    RobotDefault,
    #[serde(rename = "customer-service.default")]
    CustomerServiceDefault,
    #[serde(rename = "creative-studio.default")]
    CreativeStudioDefault,
}

impl OfficialPresetKey {
    pub const ALL: [Self; 7] = [
        Self::ChatMinimal,
        Self::AssistantGeneral,
        Self::CodingCodex,
        Self::CompanionDefault,
        Self::RobotDefault,
        Self::CustomerServiceDefault,
        Self::CreativeStudioDefault,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatMinimal => "chat.minimal",
            Self::AssistantGeneral => "assistant.general",
            Self::CodingCodex => "coding.codex",
            Self::CompanionDefault => "companion.default",
            Self::RobotDefault => "robot.default",
            Self::CustomerServiceDefault => "customer-service.default",
            Self::CreativeStudioDefault => "creative-studio.default",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetSeed {
    pub initial_capabilities: Vec<CapabilityRef>,
    pub on_demand_capabilities: Vec<CapabilityRef>,
    pub skill_bindings: Vec<SkillRef>,
    pub typed_resource_defaults: Vec<TypedResourceDefault>,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetRoleCoverage {
    pub required_capability_categories: BTreeSet<String>,
    pub required_capability_ids: BTreeSet<crate::CapabilityId>,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
    pub required_resource_kinds: BTreeSet<ResourceKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetSeedManifestPayload {
    pub manifest_version: VersionString,
    pub target_first_party_contribution_digest: DigestHex,
    pub target_runtime_feature_inventory_digest: DigestHex,
    pub templates: BTreeMap<OfficialPresetKey, OfficialPresetSeed>,
    pub role_coverage: BTreeMap<OfficialPresetKey, OfficialPresetRoleCoverage>,
    pub non_template_capability_packs: BTreeSet<String>,
    pub forbidden_official_keys: BTreeSet<String>,
}

pub type OfficialPresetSeedManifest = ArtifactEnvelope<OfficialPresetSeedManifestPayload>;

pub const OFFICIAL_PRESET_SEED_MANIFEST_PAYLOAD_JSON: &str =
    include_str!("../contracts/presets/official-preset-seed-manifest.payload.json");

pub fn official_preset_seed_manifest_payload() -> OfficialPresetSeedManifestPayload {
    serde_json::from_str(OFFICIAL_PRESET_SEED_MANIFEST_PAYLOAD_JSON)
        .expect("official preset seed fixture must match OfficialPresetSeedManifestPayload")
}

impl OfficialPresetSeedManifestPayload {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        let expected = OfficialPresetKey::ALL.into_iter().collect::<BTreeSet<_>>();
        let actual = self.templates.keys().copied().collect::<BTreeSet<_>>();
        let coverage = self
            .role_coverage
            .keys()
            .copied()
            .collect::<BTreeSet<_>>();
        if actual != expected || coverage != expected {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(OFFICIAL_PRESET_KEY_SET_MISMATCH),
                message: "official template and role-coverage maps must contain exactly seven keys"
                    .into(),
            });
        }

        for (key, seed) in &self.templates {
            validate_exact_capability_refs(
                &seed.initial_capabilities,
                &seed.on_demand_capabilities,
            )?;
            let coverage = &self.role_coverage[key];
            let selected = seed
                .initial_capabilities
                .iter()
                .chain(&seed.on_demand_capabilities)
                .map(|capability| capability.id.clone())
                .collect::<BTreeSet<_>>();
            let default_resource_kinds = seed
                .typed_resource_defaults
                .iter()
                .map(|resource| resource.resource_kind.clone())
                .collect::<BTreeSet<_>>();
            let mut default_slot_keys = BTreeSet::new();
            if seed
                .typed_resource_defaults
                .iter()
                .any(|resource| {
                    !default_slot_keys.insert(resource.slot_key.as_str())
                        || (resource.required
                            && resource.binding_policy
                                == ResourceDefaultBindingPolicy::LeaveUnbound)
                })
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(ROLE_COVERAGE_INCOMPLETE),
                    message: format!(
                        "{} has duplicate or unbound required typed resource defaults",
                        key.as_str()
                    ),
                });
            }
            if !coverage.required_capability_ids.is_subset(&selected)
                || !coverage
                    .required_runtime_features
                    .is_subset(&seed.required_runtime_features)
                || !coverage
                    .required_resource_kinds
                    .is_subset(&default_resource_kinds)
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(ROLE_COVERAGE_INCOMPLETE),
                    message: format!("{} does not cover its declared role", key.as_str()),
                });
            }
        }

        let chat = &self.templates[&OfficialPresetKey::ChatMinimal];
        let chat_coverage = &self.role_coverage[&OfficialPresetKey::ChatMinimal];
        if !chat.initial_capabilities.is_empty()
            || !chat.on_demand_capabilities.is_empty()
            || !chat.skill_bindings.is_empty()
            || !chat.typed_resource_defaults.is_empty()
            || !chat.required_runtime_features.is_empty()
            || !chat_coverage.required_capability_categories.is_empty()
            || !chat_coverage.required_capability_ids.is_empty()
            || !chat_coverage.required_runtime_features.is_empty()
            || !chat_coverage.required_resource_kinds.is_empty()
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(CHAT_MINIMAL_NOT_EXACT_EMPTY),
                message: "chat.minimal must be exact-empty".into(),
            });
        }

        let coding = &self.templates[&OfficialPresetKey::CodingCodex];
        if coding
            .initial_capabilities
            .iter()
            .chain(&coding.on_demand_capabilities)
            .any(|capability| {
                capability.id.as_ref().starts_with("browser.")
                    || capability.id.as_ref().starts_with("computer.")
            })
            || coding.typed_resource_defaults.iter().any(|resource| {
                resource.resource_kind.as_ref().starts_with("browser")
                    || resource.resource_kind.as_ref().starts_with("computer")
            })
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(CODING_CODEX_NATIVE_INCOMPLETE),
                message:
                    "coding.codex must not require Browser/Computer capabilities on headless hosts"
                        .into(),
            });
        }

        let companion = &self.templates[&OfficialPresetKey::CompanionDefault];
        let companion_union = companion
            .initial_capabilities
            .iter()
            .chain(&companion.on_demand_capabilities)
            .map(|capability| capability.id.as_ref())
            .collect::<BTreeSet<_>>();
        for capability in [
            "companion.persona",
            "knowledge.search",
            "knowledge.read",
            "memory.companion.recall",
            "memory.companion.write",
            "channel.receive",
            "channel.reply",
            "channel.send",
        ] {
            if !companion_union.contains(capability) {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(ROLE_COVERAGE_INCOMPLETE),
                    message: format!("companion.default is missing {capability}"),
                });
            }
        }

        if !self.forbidden_official_keys.contains("research")
            || !self.forbidden_official_keys.contains("research.web")
            || !self
                .forbidden_official_keys
                .contains("requirements.analyst")
            || !self
                .forbidden_official_keys
                .contains("autowork.executor")
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(OFFICIAL_PRESET_KEY_SET_MISMATCH),
                message: "Research and legacy workflow keys must remain non-template identities"
                    .into(),
            });
        }
        if !self
            .non_template_capability_packs
            .contains("research.core")
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(OFFICIAL_PRESET_KEY_SET_MISMATCH),
                message: "research.core must remain a Capability Pack, not a template".into(),
            });
        }

        Ok(())
    }

    pub fn validate_against_target_inventory(
        &self,
        inventory: &TargetPackageInventoryPayload,
    ) -> Result<(), PresetContractViolation> {
        let inventory_digest =
            digest_payload(inventory).map_err(|error| PresetContractViolation {
                code: CanonicalErrorCode::from(CAPABILITY_NOT_MATERIALIZED),
                message: error.to_string(),
            })?;
        if inventory_digest != self.target_first_party_contribution_digest {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(CAPABILITY_NOT_MATERIALIZED),
                message: "target first-party contribution digest mismatch".into(),
            });
        }

        let available = inventory
            .packages
            .iter()
            .flat_map(|package| &package.capabilities)
            .map(|capability| capability.capability.clone())
            .collect::<Vec<_>>();
        let available_skills = inventory
            .packages
            .iter()
            .flat_map(|package| &package.skills)
            .cloned()
            .collect::<Vec<_>>();
        for (key, seed) in &self.templates {
            for capability in seed
                .initial_capabilities
                .iter()
                .chain(&seed.on_demand_capabilities)
            {
                if !available.iter().any(|available| available == capability) {
                    return Err(PresetContractViolation {
                        code: CanonicalErrorCode::from(CAPABILITY_NOT_MATERIALIZED),
                        message: format!(
                            "{} references missing capability {}@{}",
                            key.as_str(),
                            capability.id.as_ref(),
                            capability.version.as_ref()
                        ),
                    });
                }
            }
            for skill in &seed.skill_bindings {
                if !available_skills.iter().any(|available| available == skill) {
                    return Err(PresetContractViolation {
                        code: CanonicalErrorCode::from(CAPABILITY_NOT_MATERIALIZED),
                        message: format!(
                            "{} references missing skill {}@{}",
                            key.as_str(),
                            skill.id.as_ref(),
                            skill.version.as_ref()
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    pub fn validate_against_runtime_feature_inventory(
        &self,
        inventory_digest: &DigestHex,
        available_features: &BTreeSet<RuntimeFeatureId>,
    ) -> Result<(), PresetContractViolation> {
        if inventory_digest != &self.target_runtime_feature_inventory_digest {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(CODING_CODEX_NATIVE_INCOMPLETE),
                message: "runtime feature inventory digest mismatch".into(),
            });
        }
        for (key, seed) in &self.templates {
            if !seed
                .required_runtime_features
                .is_subset(available_features)
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(CODING_CODEX_NATIVE_INCOMPLETE),
                    message: format!(
                        "{} requires unavailable runtime features",
                        key.as_str()
                    ),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum CanonicalHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalApiOperation {
    pub operation_id: String,
    pub method: CanonicalHttpMethod,
    pub path: String,
    pub request_schema: Option<CanonicalSchemaRef>,
    pub response_schema: Option<CanonicalSchemaRef>,
    pub canonical_errors: BTreeSet<CanonicalErrorCode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CanonicalApiInventoryPayload {
    pub inventory_version: VersionString,
    pub operations: Vec<CanonicalApiOperation>,
    pub canonical_error_codes: BTreeSet<CanonicalErrorCode>,
    pub forbidden_paths: BTreeSet<String>,
}

pub type CanonicalApiInventory = ArtifactEnvelope<CanonicalApiInventoryPayload>;

pub const CANONICAL_API_INVENTORY_PAYLOAD_JSON: &str =
    include_str!("../contracts/presets/canonical-api-inventory.payload.json");

pub fn canonical_api_inventory_payload() -> CanonicalApiInventoryPayload {
    serde_json::from_str(CANONICAL_API_INVENTORY_PAYLOAD_JSON)
        .expect("canonical API inventory fixture must match CanonicalApiInventoryPayload")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorDraftState {
    Clean,
    Dirty,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EditorRevisionAction {
    ReuseCurrentRevision,
    SaveOrdinaryVisibleRevision,
    SaveFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D022EditorTestFixtureCase {
    pub case_id: String,
    pub draft_state: EditorDraftState,
    pub revision_action: EditorRevisionAction,
    pub expected_revision_delta: u32,
    pub expected_agent_session_delta: u32,
    pub expected_external_effect_delta: u32,
    pub selected_revision_is_ordinary_visible_immutable: bool,
    pub uses_exact_agent_binding_value: bool,
    pub session_create_path: Option<String>,
    pub session_is_ordinary_persistent: bool,
    pub delete_path: Option<String>,
    pub uses_real_typed_resources: bool,
    pub uses_full_auto: bool,
    pub expected_error: Option<CanonicalErrorCode>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct D022EditorTestFixturePayload {
    pub schema_version: VersionString,
    pub cases: Vec<D022EditorTestFixtureCase>,
    pub forbidden_backend_surface: BTreeSet<String>,
}

pub const D022_EDITOR_TEST_FIXTURE_JSON: &str =
    include_str!("../contracts/presets/d022-editor-test.fixture.json");

pub fn d022_editor_test_fixture_cases() -> Vec<D022EditorTestFixtureCase> {
    serde_json::from_str::<D022EditorTestFixturePayload>(D022_EDITOR_TEST_FIXTURE_JSON)
        .expect("D-022 fixture must match D022EditorTestFixturePayload")
        .cases
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PresetContractViolation {
    pub code: CanonicalErrorCode,
    pub message: String,
}

fn validate_capability_selections(
    initial: &[CapabilitySelection],
    on_demand: &[CapabilitySelection],
) -> Result<(), PresetContractViolation> {
    validate_capability_ids(
        initial.iter().map(|selection| &selection.capability.id),
        on_demand
            .iter()
            .map(|selection| &selection.capability.id),
    )
}

fn validate_resolved_capability_sets(
    initial: &[ResolvedCapability],
    on_demand: &[ResolvedCapability],
) -> Result<(), PresetContractViolation> {
    validate_capability_ids(
        initial.iter().map(|selection| &selection.capability.id),
        on_demand
            .iter()
            .map(|selection| &selection.capability.id),
    )
}

fn validate_exact_capability_refs(
    initial: &[CapabilityRef],
    on_demand: &[CapabilityRef],
) -> Result<(), PresetContractViolation> {
    validate_capability_ids(
        initial.iter().map(|selection| &selection.id),
        on_demand.iter().map(|selection| &selection.id),
    )
}

fn validate_capability_ids<'a>(
    initial: impl Iterator<Item = &'a crate::CapabilityId>,
    on_demand: impl Iterator<Item = &'a crate::CapabilityId>,
) -> Result<(), PresetContractViolation> {
    let mut initial_ids = BTreeSet::new();
    for capability in initial {
        if !initial_ids.insert(capability) {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_CAPABILITY_DUPLICATE),
                message: format!("duplicate initial capability {}", capability.as_ref()),
            });
        }
    }

    let mut on_demand_ids = BTreeSet::new();
    for capability in on_demand {
        if !on_demand_ids.insert(capability) {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_CAPABILITY_DUPLICATE),
                message: format!("duplicate on-demand capability {}", capability.as_ref()),
            });
        }
    }

    if let Some(overlap) = initial_ids.intersection(&on_demand_ids).next() {
        return Err(PresetContractViolation {
            code: CanonicalErrorCode::from(PRESET_CAPABILITY_SET_OVERLAP),
            message: format!(
                "capability {} cannot be both initial and on-demand",
                overlap.as_ref()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(id: &str) -> CapabilityRef {
        crate::ExactVersionRef {
            id: crate::CapabilityId::from(id),
            version: VersionString::from("1.0.0"),
        }
    }

    #[test]
    fn official_key_type_is_the_exact_seven_key_set() {
        let actual = OfficialPresetKey::ALL
            .into_iter()
            .map(OfficialPresetKey::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            BTreeSet::from([
                "assistant.general",
                "chat.minimal",
                "coding.codex",
                "companion.default",
                "creative-studio.default",
                "customer-service.default",
                "robot.default",
            ])
        );
        assert!(!actual.contains("research"));
    }

    #[test]
    fn capability_sets_are_mutually_exclusive() {
        let error = validate_exact_capability_refs(
            &[capability("fs.read")],
            &[capability("fs.read")],
        )
        .unwrap_err();
        assert_eq!(error.code.as_ref(), PRESET_CAPABILITY_SET_OVERLAP);
    }

    #[test]
    fn payload_shapes_never_embed_their_own_digest() {
        let json = serde_json::to_value(OfficialPresetSeedManifestPayload {
            manifest_version: VersionString::from("1.0.0"),
            target_first_party_contribution_digest: DigestHex::from("external"),
            target_runtime_feature_inventory_digest: DigestHex::from("external"),
            templates: BTreeMap::new(),
            role_coverage: BTreeMap::new(),
            non_template_capability_packs: BTreeSet::new(),
            forbidden_official_keys: BTreeSet::new(),
        })
        .unwrap();
        let object = json.as_object().unwrap();
        assert!(!object.contains_key("manifest_digest"));
        assert!(!object.contains_key("payload_digest"));
    }

    #[test]
    fn official_seed_fixture_is_the_valid_target_contract() {
        official_preset_seed_manifest_payload().validate().unwrap();
    }

    #[test]
    fn d022_has_clean_dirty_and_save_failure_cases() {
        let cases = d022_editor_test_fixture_cases();
        assert_eq!(cases.len(), 3);
        assert!(cases.iter().any(|case| {
            case.draft_state == EditorDraftState::Clean
                && case.expected_revision_delta == 0
                && case.expected_agent_session_delta == 1
                && case.session_create_path.as_deref() == Some("/api/agent-sessions")
                && case.delete_path.as_deref()
                    == Some("/api/agent-sessions/{agent_session_id}")
        }));
        assert!(cases.iter().any(|case| {
            case.draft_state == EditorDraftState::Dirty
                && case.expected_revision_delta == 1
                && case.expected_agent_session_delta == 1
                && case.selected_revision_is_ordinary_visible_immutable
                && case.uses_exact_agent_binding_value
        }));
        assert!(cases.iter().any(|case| {
            case.revision_action == EditorRevisionAction::SaveFailed
                && case.expected_agent_session_delta == 0
                && case.expected_external_effect_delta == 0
                && case.session_create_path.is_none()
                && case.delete_path.is_none()
        }));
    }

    #[test]
    fn canonical_api_inventory_has_no_test_or_legacy_resource() {
        let inventory = canonical_api_inventory_payload();
        assert!(inventory.operations.iter().all(|operation| {
            !operation.path.starts_with("/api/presets")
                && !operation.path.starts_with("/api/conversations")
                && !operation.path.contains("/test")
        }));
        assert!(inventory
            .canonical_error_codes
            .iter()
            .all(|code| code.as_ref() == code.as_ref().to_ascii_uppercase()));
    }
}
