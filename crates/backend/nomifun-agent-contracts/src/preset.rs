use std::collections::{BTreeMap, BTreeSet};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::digest::{CanonicalDigestError, digest_payload};
use crate::package::{
    CapabilityActionDescriptor, CapabilityRef, ExactRoleProviderRef, PackageRef,
    PluginSourceMetadata, RoleProviderSelection, SkillRef,
    TargetPackageInventoryPayload,
};
use crate::runtime::RuntimeProfileKind;
use crate::{
    ActionId, AgentPresetId, ArtifactEnvelope, CanonicalErrorCode, CanonicalSchemaRef,
    ContributionId, ContributionSourceKind, DigestHex, ChatRouteIdentity, ChatRouteRecord,
    McpBindingId, McpServerId, McpToolKey, ModelRouteId, OperationId, PluginMountId,
    PluginProductId,
    PrincipalRef, ResolvedSnapshotId, ResourceKind, RuntimeFeatureId,
    StableSourceIdentity, TypedResourceBindings, UserId, VersionString,
};
use crate::plugin_runtime::PluginReleaseRef;

pub const CAPABILITY_NOT_MATERIALIZED: &str = "CAPABILITY_NOT_MATERIALIZED";
pub const CAPABILITY_NOT_IN_PRESET: &str = "CAPABILITY_NOT_IN_PRESET";
pub const CAPABILITY_NOT_ACTIVE: &str = "CAPABILITY_NOT_ACTIVE";
pub const CAPABILITY_UNAVAILABLE_ON_PLATFORM: &str = "CAPABILITY_UNAVAILABLE_ON_PLATFORM";
pub const AGENT_PRESET_NOT_FOUND: &str = "AGENT_PRESET_NOT_FOUND";
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
pub const PRESET_CONTRIBUTION_LOCK_INVALID: &str = "PRESET_CONTRIBUTION_LOCK_INVALID";

/// A task-only media Agent does not need a language-model route to submit
/// explicit generation requests. Any conversational or other capability keeps
/// the normal Chat-route requirement; this is never inferred from its name.
pub fn is_direct_creation_agent<'a>(capabilities: impl IntoIterator<Item = &'a str>) -> bool {
    let mut has_generation = false;
    for capability in capabilities {
        match capability {
            "creation.media" => has_generation = true,
            "creative.workshop" | "office" => {}
            _ => return false,
        }
    }
    has_generation
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentPresetSource {
    Official,
    User,
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
pub struct ContributionLock {
    pub source_kind: ContributionSourceKind,
    pub source_identity: StableSourceIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mount_id: Option<PluginMountId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_product_id: Option<PluginProductId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_binding_id: Option<McpBindingId>,
    pub contribution_id: ContributionId,
    pub contract_digest: DigestHex,
}

impl ContributionLock {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        validate_non_empty_canonical_value(self.source_identity.as_ref(), "source_identity")?;
        validate_non_empty_canonical_value(self.contribution_id.as_ref(), "contribution_id")?;
        if !is_lowercase_hex_digest(&self.contract_digest) {
            return Err(contribution_lock_violation(
                "contract_digest must be 64 lowercase hexadecimal characters",
            ));
        }

        let valid_source_identity = match self.source_kind {
            ContributionSourceKind::PlatformBuiltin => {
                self.mount_id.is_none()
                    && self.plugin_product_id.is_none()
                    && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::PluginMount => {
                self.mount_id.is_some()
                    && self.plugin_product_id.is_none()
                    && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::PluginProductActiveRelease => {
                self.plugin_product_id.is_some() && self.mcp_binding_id.is_none()
            }
            ContributionSourceKind::McpBinding => {
                self.mcp_binding_id.is_some() && self.plugin_product_id.is_none()
            }
        };
        if !valid_source_identity {
            return Err(contribution_lock_violation(format!(
                "source-specific provenance is invalid for {:?}",
                self.source_kind
            )));
        }
        Ok(())
    }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySelection {
    /// Exact Capability Module revision. The historical field name remains on
    /// the v1 wire until AgentPreset vNext switches the outer document.
    pub capability: CapabilityRef,
    /// Exact granted Action IDs. Empty means no Action authority, never all
    /// actions declared by the Module.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub action_allowlist: BTreeSet<ActionId>,
}

impl CapabilitySelection {
    pub fn module(&self) -> &CapabilityRef {
        &self.capability
    }

    pub fn allowed_actions(&self) -> &BTreeSet<ActionId> {
        &self.action_allowlist
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevisionPayload {
    /// Ordered Context contributors; omitted contributors follow canonical ID order.
    /// This is presentation/execution order, never an authorization grant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_order: Vec<crate::CapabilityId>,
    /// Ordered request middleware; omitted contributors follow canonical ID
    /// order. This controls composition, never selection or authorization.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub middleware_order: Vec<crate::CapabilityId>,
    pub schema_version: VersionString,
    pub model_route_refs: BTreeMap<String, ModelRouteId>,
    /// Complete route facts used by the canonical Agent Store writer. Legacy
    /// opaque IDs are not sufficient to construct a provider request.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub chat_route_records: BTreeMap<String, ChatRouteRecord>,
    pub enabled_capabilities: Vec<CapabilitySelection>,
    pub skill_bindings: Vec<SkillRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub system_role_provider_overrides:
        BTreeMap<crate::ExecutionRoleId, RoleProviderSelection>,
    pub persona: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starter_prompts: Vec<String>,
    /// Session supervision defaults. These are runtime policy, never a model
    /// Capability grant, and are copied only when a new Session is created.
    #[serde(
        default,
        skip_serializing_if = "crate::AgentRuntimePolicy::is_default"
    )]
    pub runtime_policy: crate::AgentRuntimePolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevisionDigestInput {
    pub payload: AgentPresetRevisionPayload,
    pub contribution_locks: Vec<ContributionLock>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentPresetRevision {
    pub reference: PresetRevisionRef,
    pub payload: AgentPresetRevisionPayload,
    pub contribution_locks: Vec<ContributionLock>,
    pub created_by: UserId,
    pub created_at_ms: i64,
    pub reason: Option<String>,
}

impl AgentPresetRevision {
    pub fn revision_digest_input(&self) -> AgentPresetRevisionDigestInput {
        let mut contribution_locks = self.contribution_locks.clone();
        contribution_locks.sort();
        AgentPresetRevisionDigestInput {
            payload: self.payload.clone(),
            contribution_locks,
        }
    }

    pub fn revision_digest(&self) -> Result<DigestHex, CanonicalDigestError> {
        digest_payload(&self.revision_digest_input())
    }

    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        self.payload.runtime_policy.validate()?;
        validate_chat_route_records_for_revision(
            &self.payload,
            Some(&self.reference.revision_id()),
        )?;
        validate_capability_selections(
            &self.payload.enabled_capabilities,

        )?;
        validate_role_provider_overrides(&self.payload.system_role_provider_overrides)?;
        validate_context_order(
            &self.payload.context_order,
            &self.payload.enabled_capabilities.iter().map(|value| value.capability.id.clone()).collect(),
        )?;
        validate_contribution_locks(&self.contribution_locks)?;
        validate_contribution_order(
            &self.payload.middleware_order,
            &self.payload.enabled_capabilities.iter().map(|value| value.capability.id.clone()).collect(),
            "middleware_order",
        )?;
        let digest = self.revision_digest().map_err(|error| PresetContractViolation {
            code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
            message: error.to_string(),
        })?;
        if digest != self.reference.revision_digest {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message:
                    "revision_digest must cover the normalized payload and contribution_locks"
                        .into(),
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

/// Why a capability belongs to this plan. Membership does not imply public
/// Tool/Context consumption; dependencies execute through a scoped caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityConsumption {
    #[default]
    Contribution,
    Dependency,
}

impl CapabilityConsumption {
    pub fn is_contribution(&self) -> bool {
        matches!(self, Self::Contribution)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCapability {
    #[serde(default, skip_serializing_if = "CapabilityConsumption::is_contribution")]
    pub consumption: CapabilityConsumption,
    /// Exact direct edges of the selected execution plan, not a second copy of
    /// the dependency's descriptor or a grant to call arbitrary plan members.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependency_refs: Vec<CapabilityRef>,
    pub capability: CapabilityRef,
    pub source_package: PackageRef,
    pub contribution_id: ContributionId,
    pub contribution_lock: ContributionLock,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_mount_id: Option<PluginMountId>,
    pub resolved_source: PluginSourceMetadata,
    pub target_artifact_digest: DigestHex,
    pub schema_digest: DigestHex,
    pub dependency_path: Vec<crate::CapabilityId>,
    pub required_runtime_features: BTreeSet<RuntimeFeatureId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_product_id: Option<PluginProductId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_release: Option<PluginReleaseRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_release_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_digest: Option<DigestHex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<CapabilityActionDescriptor>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_resource_kinds: BTreeSet<ResourceKind>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub action_allowlist: BTreeSet<ActionId>,
}

impl ResolvedCapability {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        validate_non_empty_canonical_value(
            self.capability.id.as_ref(),
            "capability.id",
        )?;
        validate_non_empty_canonical_value(
            self.capability.version.as_ref(),
            "capability.version",
        )?;
        validate_non_empty_canonical_value(
            self.source_package.id.as_ref(),
            "source_package.id",
        )?;
        validate_non_empty_canonical_value(
            self.source_package.version.as_ref(),
            "source_package.version",
        )?;
        validate_non_empty_canonical_value(
            self.contribution_id.as_ref(),
            "contribution_id",
        )?;
        if let Some(mount_id) = &self.resolved_mount_id {
            validate_non_empty_canonical_value(mount_id.as_ref(), "resolved_mount_id")?;
        }
        validate_non_empty_canonical_value(
            &self.resolved_source.source_identity,
            "resolved_source.source_identity",
        )?;
        self.contribution_lock.validate()?;
        if self.contribution_id != self.contribution_lock.contribution_id {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "contribution_id does not match contribution_lock",
            ));
        }
        if self.schema_digest != self.contribution_lock.contract_digest {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "schema_digest does not match contribution_lock.contract_digest",
            ));
        }
        if let Some(mount_id) = &self.contribution_lock.mount_id
            && self.resolved_mount_id.as_ref() != Some(mount_id)
        {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "contribution_lock.mount_id does not match resolved_mount_id",
            ));
        }
        for (field, digest) in [
            ("schema_digest", &self.schema_digest),
            ("target_artifact_digest", &self.target_artifact_digest),
        ] {
            if !is_lowercase_hex_digest(digest) {
                return Err(snapshot_capability_violation(
                    &self.capability.id,
                    format!("{field} must be 64 lowercase hexadecimal characters"),
                ));
            }
        }
        if let Some(source_digest) = &self.resolved_source.source_digest
            && !is_lowercase_hex_digest(source_digest)
        {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "resolved_source.source_digest must be 64 lowercase hexadecimal characters",
            ));
        }
        if self.dependency_path.is_empty()
            || self.dependency_path.last() != Some(&self.capability.id)
        {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "dependency_path must terminate at the resolved capability",
            ));
        }
        let mut declared_actions = BTreeSet::new();
        for action in &self.actions {
            if action.action_id.as_ref().trim().is_empty()
                || action.input_schema.as_ref().trim().is_empty()
                || action.output_schema.as_ref().trim().is_empty()
                || !declared_actions.insert(action.action_id.clone())
            {
                return Err(snapshot_capability_violation(
                    &self.capability.id,
                    "actions must have unique non-empty identities and schemas",
                ));
            }
        }
        if let Some(action_id) = self
            .action_allowlist
            .iter()
            .find(|action_id| !declared_actions.contains(*action_id))
        {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                format!(
                    "action grant contains undeclared action {}",
                    action_id.as_ref()
                ),
            ));
        }
        if !self.actions.is_empty() && self.action_allowlist.is_empty() {
            return Err(snapshot_capability_violation(
                &self.capability.id,
                "an action-bearing Module must freeze an explicit non-empty action grant",
            ));
        }
        match self.contribution_lock.source_kind {
            ContributionSourceKind::PluginProductActiveRelease => {
                let product_id = self.plugin_product_id.as_ref().ok_or_else(|| {
                    snapshot_capability_violation(
                        &self.capability.id,
                        "plugin_product_id is required for a Plugin Product release",
                    )
                })?;
                validate_non_empty_canonical_value(product_id.as_ref(), "plugin_product_id")?;
                let release = self.active_release.as_ref().ok_or_else(|| {
                    snapshot_capability_violation(
                        &self.capability.id,
                        "active_release is required for a Plugin Product release",
                    )
                })?;
                release
                    .validate()
                    .map_err(|error| snapshot_capability_violation(
                        &self.capability.id,
                        error.to_string(),
                    ))?;
                let active_release_epoch = self.active_release_epoch.ok_or_else(|| {
                    snapshot_capability_violation(
                        &self.capability.id,
                        "active_release_epoch is required for a Plugin Product release",
                    )
                })?;
                if active_release_epoch == 0 {
                    return Err(snapshot_capability_violation(
                        &self.capability.id,
                        "active_release_epoch must be greater than zero",
                    ));
                }
                if !self.resolved_mount_id.is_none()
                    || self.contribution_lock.mount_id.is_some()
                    || self.contribution_lock.mcp_binding_id.is_some()
                    || self.contribution_lock.plugin_product_id.as_ref() != Some(product_id)
                {
                    return Err(snapshot_capability_violation(
                        &self.capability.id,
                        "Plugin Product release provenance is invalid",
                    ));
                }
                let catalog_digest = self.catalog_digest.as_ref().ok_or_else(|| {
                    snapshot_capability_violation(
                        &self.capability.id,
                        "catalog_digest is required for a Plugin Product release",
                    )
                })?;
                if !is_lowercase_hex_digest(catalog_digest) {
                    return Err(snapshot_capability_violation(
                        &self.capability.id,
                        "catalog_digest must be 64 lowercase hexadecimal characters",
                    ));
                }
                if self.actions.is_empty() {
                    return Err(snapshot_capability_violation(
                        &self.capability.id,
                        "Plugin Product capability must freeze at least one action",
                    ));
                }
            }
            _ => {
                if self.plugin_product_id.is_some()
                    || self.active_release.is_some()
                    || self.active_release_epoch.is_some()
                    || self.catalog_digest.is_some()
                {
                    return Err(snapshot_capability_violation(
                        &self.capability.id,
                        "Plugin Product release fields require Plugin Product release provenance",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSkillLock {
    pub skill: SkillRef,
    pub body_digest: DigestHex,
    pub required_capabilities: BTreeSet<crate::CapabilityId>,
    pub contribution_lock: ContributionLock,
    pub resolved_mount_id: PluginMountId,
    pub resolved_source: PluginSourceMetadata,
    pub target_artifact_digest: DigestHex,
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

impl ResolvedMcpToolLock {
    pub fn validate(&self) -> Result<(), PresetContractViolation> {
        validate_non_empty_canonical_value(self.server_id.as_ref(), "server_id")?;
        validate_non_empty_canonical_value(
            self.canonical_tool_key.as_ref(),
            "canonical_tool_key",
        )?;
        validate_non_empty_canonical_value(self.capability_id.as_ref(), "capability_id")?;
        if !is_lowercase_hex_digest(&self.schema_digest) {
            return Err(mcp_lock_violation(
                "schema_digest must be 64 lowercase hexadecimal characters",
            ));
        }
        if self.materialization_revision == 0 {
            return Err(mcp_lock_violation(
                "materialization_revision must be greater than or equal to 1",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedRoleProviderLock {
    pub provider: ExactRoleProviderRef,
    pub source: PluginSourceMetadata,
    pub supported_members: BTreeSet<crate::CapabilityId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSnapshotContent {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_order: Vec<crate::CapabilityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub middleware_order: Vec<crate::CapabilityId>,
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
    pub enabled_capabilities: Vec<ResolvedCapability>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub required_resource_kinds: BTreeSet<ResourceKind>,
    pub capability_allowlist: BTreeSet<crate::CapabilityId>,
    pub skill_locks: Vec<ResolvedSkillLock>,
    pub mcp_tool_locks: Vec<ResolvedMcpToolLock>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_role_providers:
        BTreeMap<crate::ExecutionRoleId, ResolvedRoleProviderLock>,
    pub canonical_schema_manifest_digest: DigestHex,
    pub target_contribution_manifest_digest: DigestHex,
}

impl ResolvedSnapshotContent {
    pub fn contributions(&self) -> impl Iterator<Item = &ResolvedCapability> {
        self.enabled_capabilities.iter().filter(|value| value.consumption.is_contribution())
    }
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
            &self.content.enabled_capabilities,

        )?;
        validate_snapshot_chat_route_identity(&self.content)?;
        validate_dependency_graph(&self.content)?;
        let order_candidates = self.content.contributions()
            .map(|value| value.capability.id.clone())
            .filter(|id| self.content.capability_allowlist.contains(id))
            .collect();
        validate_context_order(&self.content.context_order, &order_candidates)?;
        validate_contribution_order(&self.content.middleware_order, &order_candidates, "middleware_order")?;
        let mut skill_ids = BTreeSet::new();
        for lock in &self.content.skill_locks {
            lock.contribution_lock.validate()?;
            if !skill_ids.insert(&lock.skill.id)
                || lock.skill.id.as_ref().trim().is_empty()
                || lock.skill.version.as_ref().trim().is_empty()
                || lock.resolved_mount_id.as_ref().trim().is_empty()
                || lock.resolved_source.source_identity.trim().is_empty()
                || !is_lowercase_hex_digest(&lock.body_digest)
                || !is_lowercase_hex_digest(&lock.target_artifact_digest)
                || (lock.contribution_lock.source_kind == ContributionSourceKind::PluginMount
                    && lock.contribution_lock.mount_id.as_ref() != Some(&lock.resolved_mount_id))
                || !lock.required_capabilities.is_subset(&self.content.capability_allowlist)
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                    message: format!("Skill {} has invalid identity, provenance or dependencies", lock.skill.id.as_ref()),
                });
            }
        }
        validate_resolved_role_provider_locks(
            &self.content.resolved_role_providers,
        )?;
        validate_resolved_mcp_tool_locks(
            &self.content.mcp_tool_locks,
            &self.content.enabled_capabilities,

        )?;
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

fn validate_context_order(
    order: &[crate::CapabilityId],
    selected: &BTreeSet<crate::CapabilityId>,
) -> Result<(), PresetContractViolation> {
    validate_contribution_order(order, selected, "context_order")
}

fn validate_contribution_order(
    order: &[crate::CapabilityId],
    selected: &BTreeSet<crate::CapabilityId>,
    field: &str,
) -> Result<(), PresetContractViolation> {
    let mut seen = BTreeSet::new();
    for id in order {
        if !selected.contains(id) || !seen.insert(id) {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_CONTRIBUTION_LOCK_INVALID),
                message: format!("{field} contains duplicate or unselected capability {}", id.as_ref()),
            });
        }
    }
    Ok(())
}

/// Validate the frozen graph itself, independent of its digest and the live
/// Registry. Edges describe exact membership, never authority to call a peer.
fn validate_dependency_graph(content: &ResolvedSnapshotContent) -> Result<(), PresetContractViolation> {
    let nodes = content.enabled_capabilities.iter()
        .map(|value| (value.capability.id.clone(), value)).collect::<BTreeMap<_, _>>();
    let invalid = |message: &str| PresetContractViolation {
        code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
        message: message.into(),
    };
    if nodes.keys().cloned().collect::<BTreeSet<_>>() != content.capability_allowlist {
        return Err(invalid("capability_allowlist must match the frozen dependency graph"));
    }
    let mut incoming = nodes.keys().map(|id| (id.clone(), 0usize)).collect::<BTreeMap<_, _>>();
    for node in nodes.values() {
        let mut seen = BTreeSet::new();
        for dependency in &node.dependency_refs {
            if !seen.insert(&dependency.id)
                || !nodes.get(&dependency.id).is_some_and(|target| target.capability == *dependency)
            {
                return Err(invalid("dependency edges must be unique exact references to frozen capabilities"));
            }
            *incoming.get_mut(&dependency.id).expect("validated target") += 1;
        }
    }
    let mut ready = incoming.iter().filter(|(_, count)| **count == 0)
        .map(|(id, _)| id.clone()).collect::<Vec<_>>();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        for edge in &nodes[&id].dependency_refs {
            let count = incoming.get_mut(&edge.id).expect("validated target");
            *count -= 1;
            if *count == 0 { ready.push(edge.id.clone()); }
        }
    }
    if visited != nodes.len() {
        return Err(invalid("frozen capability dependency graph contains a cycle"));
    }
    let mut reachable = BTreeSet::new();
    let mut work = content.contributions().map(|value| value.capability.id.clone()).collect::<Vec<_>>();
    while let Some(id) = work.pop() {
        if reachable.insert(id.clone()) {
            work.extend(nodes[&id].dependency_refs.iter().map(|edge| edge.id.clone()));
        }
    }
    if reachable.len() != nodes.len() {
        return Err(invalid("dependency-only capabilities must be reachable from a contribution"));
    }
    Ok(())
}

fn validate_resolved_role_provider_locks(
    locks: &BTreeMap<crate::ExecutionRoleId, ResolvedRoleProviderLock>,
) -> Result<(), PresetContractViolation> {
    for (role_id, lock) in locks {
        if role_id != &lock.provider.role.key.role_id
            || lock.provider.mount_id.as_ref().trim().is_empty()
            || lock.provider.package.id.as_ref().trim().is_empty()
            || lock.supported_members.is_empty()
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "resolved role provider lock {} has inconsistent identity",
                    role_id.as_ref()
                ),
            });
        }
    }
    Ok(())
}

fn validate_resolved_mcp_tool_locks(
    locks: &[ResolvedMcpToolLock],
    initial: &[ResolvedCapability],
) -> Result<(), PresetContractViolation> {
    let mut identity_keys = BTreeSet::new();
    let mut capability_ids = BTreeSet::new();

    for lock in locks {
        lock.validate()?;

        let identity = (lock.server_id.clone(), lock.canonical_tool_key.clone());
        if !identity_keys.insert(identity) {
            return Err(mcp_lock_violation(format!(
                "duplicate MCP tool lock for server {} and tool {}",
                lock.server_id.as_ref(),
                lock.canonical_tool_key.as_ref()
            )));
        }
        if !capability_ids.insert(lock.capability_id.clone()) {
            return Err(mcp_lock_violation(format!(
                "duplicate MCP tool lock capability {}",
                lock.capability_id.as_ref()
            )));
        }

        let resolved = initial
            .iter()
            .filter(|capability| capability.consumption.is_contribution())
            .find(|capability| capability.capability.id == lock.capability_id)
            .ok_or_else(|| {
                mcp_lock_violation(format!(
                    "MCP tool lock capability {} is not resolved in initial or on-demand capabilities",
                    lock.capability_id.as_ref()
                ))
            })?;
        // `ResolvedCapability.schema_digest` is the digest of the complete
        // Capability manifest. `ResolvedMcpToolLock.schema_digest` is the
        // materialized MCP action-input schema digest; they intentionally
        // describe different layers and must not be compared here. The
        // materialization/source and owner validate the latter against the
        // stored tool schema immediately before network execution.
        let _ = resolved;
    }
    Ok(())
}

fn validate_non_empty_canonical_value(
    value: &str,
    field: &str,
) -> Result<(), PresetContractViolation> {
    if value.trim().is_empty() || value != value.trim() {
        return Err(mcp_lock_violation(format!(
            "{field} must be a non-empty canonical value"
        )));
    }
    Ok(())
}

fn is_lowercase_hex_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn mcp_lock_violation(message: impl Into<String>) -> PresetContractViolation {
    PresetContractViolation {
        code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
        message: message.into(),
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
    #[serde(rename = "customer-service.default")]
    CustomerServiceDefault,
    #[serde(rename = "creative-studio.default")]
    CreativeStudioDefault,
}

impl OfficialPresetKey {
    pub const ALL: [Self; 6] = [
        Self::ChatMinimal,
        Self::AssistantGeneral,
        Self::CodingCodex,
        Self::CompanionDefault,
        Self::CustomerServiceDefault,
        Self::CreativeStudioDefault,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChatMinimal => "chat.minimal",
            Self::AssistantGeneral => "assistant.general",
            Self::CodingCodex => "coding.codex",
            Self::CompanionDefault => "companion.default",
            Self::CustomerServiceDefault => "customer-service.default",
            Self::CreativeStudioDefault => "creative-studio.default",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OfficialPresetSeed {
    pub enabled_capabilities: Vec<CapabilitySelection>,
    pub skill_bindings: Vec<SkillRef>,
    pub required_resource_kinds: BTreeSet<ResourceKind>,
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
    include_str!("../contracts/presets/official-agent-seed-manifest.payload.json");

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
                message: "official template and role-coverage maps must contain exactly six keys"
                    .into(),
            });
        }

        for (key, seed) in &self.templates {
            validate_capability_selections(&seed.enabled_capabilities)?;
            let coverage = &self.role_coverage[key];
            let selected = seed
                .enabled_capabilities
                .iter()
                .map(|selection| selection.capability.id.clone())
                .collect::<BTreeSet<_>>();
            if !coverage.required_capability_ids.is_subset(&selected)
                || !coverage
                    .required_runtime_features
                    .is_subset(&seed.required_runtime_features)
                || coverage.required_resource_kinds != seed.required_resource_kinds
            {
                return Err(PresetContractViolation {
                    code: CanonicalErrorCode::from(ROLE_COVERAGE_INCOMPLETE),
                    message: format!("{} does not cover its declared role", key.as_str()),
                });
            }
        }

        let chat = &self.templates[&OfficialPresetKey::ChatMinimal];
        let chat_coverage = &self.role_coverage[&OfficialPresetKey::ChatMinimal];
        if !chat.enabled_capabilities.is_empty()

            || !chat.skill_bindings.is_empty()
            || !chat.required_resource_kinds.is_empty()
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
            .enabled_capabilities
            .iter()
            .any(|capability| {
                capability.capability.id.as_ref() == "browser"
                    || capability.capability.id.as_ref() == "computer"
            })
            || coding.required_resource_kinds.iter().any(|resource_kind| {
                resource_kind.as_ref().starts_with("browser")
                    || resource_kind.as_ref().starts_with("computer")
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
            .enabled_capabilities
            .iter()
            .map(|selection| selection.capability.id.as_ref())
            .collect::<BTreeSet<_>>();
        for capability in [
            "companion",
            "companion.memory",
            "channel.messaging",
            "robot",
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
            for selection in &seed.enabled_capabilities {
                if !available
                    .iter()
                    .any(|available| available == &selection.capability)
                {
                    return Err(PresetContractViolation {
                        code: CanonicalErrorCode::from(CAPABILITY_NOT_MATERIALIZED),
                        message: format!(
                            "{} references missing capability {}@{}",
                            key.as_str(),
                            selection.capability.id.as_ref(),
                            selection.capability.version.as_ref()
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
#[serde(deny_unknown_fields)]
pub struct PresetContractViolation {
    pub code: CanonicalErrorCode,
    pub message: String,
}

fn validate_capability_selections(
    initial: &[CapabilitySelection],
) -> Result<(), PresetContractViolation> {
    validate_capability_ids(
        initial.iter().map(|selection| &selection.capability.id),
    )?;
    for grant in initial {
        if let Some(action_id) = grant
            .action_allowlist
            .iter()
            .find(|action_id| action_id.as_ref().trim().is_empty())
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "capability module {} contains an empty Action grant ({})",
                    grant.capability.id.as_ref(),
                    action_id.as_ref()
                ),
            });
        }
    }
    Ok(())
}

fn validate_contribution_locks(
    locks: &[ContributionLock],
) -> Result<(), PresetContractViolation> {
    let mut contribution_ids = BTreeSet::new();
    for lock in locks {
        lock.validate()?;
        if !contribution_ids.insert(lock.contribution_id.clone()) {
            return Err(contribution_lock_violation(format!(
                "duplicate contribution lock {}",
                lock.contribution_id.as_ref()
            )));
        }
    }
    Ok(())
}

fn contribution_lock_violation(message: impl Into<String>) -> PresetContractViolation {
    PresetContractViolation {
        code: CanonicalErrorCode::from(PRESET_CONTRIBUTION_LOCK_INVALID),
        message: message.into(),
    }
}

fn validate_role_provider_overrides(
    overrides: &BTreeMap<crate::ExecutionRoleId, RoleProviderSelection>,
) -> Result<(), PresetContractViolation> {
    for (role_id, selection) in overrides {
        if role_id.as_ref().trim().is_empty() || !role_id.as_ref().contains('.') {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "role provider override key {} must be a namespaced execution role",
                    role_id.as_ref()
                ),
            });
        }
        if &selection.role.key.role_id != role_id
            || selection.provider_mount_id.as_ref().trim().is_empty()
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "role provider override {} has inconsistent role or mount identity",
                    role_id.as_ref()
                ),
            });
        }
        if !looks_like_semver(selection.role.key.contract_version.as_ref()) {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "role provider override {} has an invalid contract version",
                    role_id.as_ref()
                ),
            });
        }
        if selection.role.contract_digest.as_ref().len() != 64
            || !selection
                .role
                .contract_digest
                .as_ref()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
                message: format!(
                    "role provider override {} has an invalid contract digest",
                    role_id.as_ref()
                ),
            });
        }
    }
    Ok(())
}

fn looks_like_semver(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(major) = parts.next() else {
        return false;
    };
    let Some(minor) = parts.next() else {
        return false;
    };
    let Some(patch) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && [major, minor, patch]
            .into_iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn validate_resolved_capability_sets(
    initial: &[ResolvedCapability],
) -> Result<(), PresetContractViolation> {
    for capability in initial.iter() {
        capability.validate()?;
    }
    validate_capability_ids(
        initial.iter().map(|selection| &selection.capability.id),
    )
}

fn snapshot_capability_violation(
    capability_id: &crate::CapabilityId,
    reason: impl AsRef<str>,
) -> PresetContractViolation {
    PresetContractViolation {
        code: CanonicalErrorCode::from(PRESET_REVISION_DIGEST_MISMATCH),
        message: format!(
            "resolved capability {} has invalid exact provenance: {}",
            capability_id.as_ref(),
            reason.as_ref()
        ),
    }
}

fn validate_capability_ids<'a>(
    enabled: impl Iterator<Item = &'a crate::CapabilityId>,
) -> Result<(), PresetContractViolation> {
    let mut ids = BTreeSet::new();
    for capability in enabled {
        if !ids.insert(capability) {
            return Err(PresetContractViolation {
                code: CanonicalErrorCode::from(PRESET_CAPABILITY_DUPLICATE),
                message: format!("duplicate enabled capability {}", capability.as_ref()),
            });
        }
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

    fn resolved_capability(id: &str, schema_digest: &str) -> ResolvedCapability {
        let capability_id = crate::CapabilityId::from(id);
        let contribution_id =
            crate::ContributionId::from(format!("capability:{id}"));
        let source_package = PackageRef {
            id: crate::PackageId::from("fixture.package"),
            version: VersionString::from("1.0.0"),
        };
        ResolvedCapability {
            consumption: Default::default(),
            dependency_refs: Vec::new(),
            capability: CapabilityRef {
                id: capability_id.clone(),
                version: VersionString::from("1.0.0"),
            },
            source_package,
            contribution_id: contribution_id.clone(),
            contribution_lock: ContributionLock {
                source_kind: ContributionSourceKind::PlatformBuiltin,
                source_identity: StableSourceIdentity::from("fixture.package"),
                mount_id: None,
                plugin_product_id: None,
                mcp_binding_id: None,
                contribution_id,
                contract_digest: DigestHex::from(schema_digest),
            },
            resolved_mount_id: None,
            resolved_source: PluginSourceMetadata {
                source_kind: crate::PluginSourceKind::Bundled,
                source_identity: "fixture.package".to_owned(),
                source_digest: Some(DigestHex::from("b".repeat(64))),
            },
            target_artifact_digest: DigestHex::from("b".repeat(64)),
            schema_digest: DigestHex::from(schema_digest),
            dependency_path: vec![capability_id],
            required_runtime_features: BTreeSet::new(),
            plugin_product_id: None,
            active_release: None,
            active_release_epoch: None,
            catalog_digest: None,
            display_name: None,
            description: None,
            actions: Vec::new(),
            required_resource_kinds: BTreeSet::new(),
            action_allowlist: BTreeSet::new(),
        }
    }

    fn mcp_lock(
        server_id: &str,
        canonical_tool_key: &str,
        capability_id: &str,
        schema_digest: &str,
    ) -> ResolvedMcpToolLock {
        ResolvedMcpToolLock {
            server_id: McpServerId::from(server_id),
            canonical_tool_key: McpToolKey::from(canonical_tool_key),
            capability_id: crate::CapabilityId::from(capability_id),
            schema_digest: DigestHex::from(schema_digest),
            materialization_revision: 1,
        }
    }

    fn snapshot_content(
        enabled_capabilities: Vec<ResolvedCapability>,
        mcp_tool_locks: Vec<ResolvedMcpToolLock>,
    ) -> ResolvedSnapshotContent {
        let capability_allowlist = enabled_capabilities.iter()
            .map(|value| value.capability.id.clone()).collect();
        ResolvedSnapshotContent {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from("1.0.0"),
            resolver_version: VersionString::from("1.0.0"),
            preset_revision_ref: PresetRevisionRef {
                preset_id: AgentPresetId::from("fixture.preset"),
                revision: 1,
                revision_digest: DigestHex::from("fixture-revision"),
            },
            required_runtime_protocol_version: VersionString::from("1.0.0"),
            required_runtime_profile: RuntimeProfileKind::CodingNative,
            runtime_feature_inventory_digest: DigestHex::from("fixture-runtime"),
            required_runtime_features: BTreeSet::new(),
            compiled_runtime_profile_digest: DigestHex::from("fixture-profile"),
            model_route_refs: BTreeMap::new(),
            chat_route_identity: None,
            enabled_capabilities,
            required_resource_kinds: BTreeSet::new(),
            capability_allowlist,
            skill_locks: Vec::new(),
            mcp_tool_locks,
            resolved_role_providers: BTreeMap::new(),
            canonical_schema_manifest_digest: DigestHex::from("fixture-schema"),
            target_contribution_manifest_digest: DigestHex::from("fixture-target"),
        }
    }

    fn envelope(content: ResolvedSnapshotContent) -> ResolvedSnapshotEnvelope {
        let snapshot_digest = digest_payload(&content).expect("fixture content digest");
        ResolvedSnapshotEnvelope {
            snapshot_ref: ResolvedSnapshotRef {
                snapshot_id: ResolvedSnapshotId::from("fixture.snapshot"),
                snapshot_digest,
            },
            content,
            actor: PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "fixture-user".to_owned(),
            },
            scene: "fixture".to_owned(),
            surface: "test".to_owned(),
            audience: "fixture".to_owned(),
            created_at_ms: 1,
            resolver_run_id: OperationId::from("fixture-operation"),
            availability_evidence_revision: "1".to_owned(),
        }
    }

    #[test]
    fn frozen_dependency_graph_validates_structure_even_with_a_fresh_digest() {
        let mut root = resolved_capability("root", &"a".repeat(64));
        let mut child = resolved_capability("child", &"b".repeat(64));
        child.consumption = CapabilityConsumption::Dependency;
        root.dependency_refs = vec![child.capability.clone()];
        let content = snapshot_content(vec![root, child], Vec::new());
        assert!(envelope(content.clone()).validate().is_ok());
        assert_eq!(content.contributions().count(), 1);
        for case in 0..6 {
            let mut invalid = content.clone();
            match case {
                0 => invalid.enabled_capabilities[0].dependency_refs[0].version = "wrong".into(),
                1 => invalid.enabled_capabilities[0].dependency_refs.push(content.enabled_capabilities[1].capability.clone()),
                2 => invalid.enabled_capabilities[1].dependency_refs.push(content.enabled_capabilities[0].capability.clone()),
                3 => invalid.enabled_capabilities[0].dependency_refs.clear(),
                4 => invalid.context_order.push("child".into()),
                _ => { invalid.capability_allowlist.remove(&crate::CapabilityId::from("child")); },
            }
            assert!(envelope(invalid).validate().is_err(), "case {case}");
        }
        let mut direct_too = content;
        direct_too.enabled_capabilities[1].consumption = CapabilityConsumption::Contribution;
        direct_too.context_order.push("child".into());
        assert_eq!(direct_too.contributions().count(), 2);
        assert!(envelope(direct_too).validate().is_ok());
    }

    #[test]
    fn resolved_module_action_grant_is_exact_and_never_uses_empty_as_all() {
        let mut module = resolved_capability("workspace.files", &"a".repeat(64));
        module.actions = ["read", "patch"]
            .into_iter()
            .map(|action| crate::CapabilityActionDescriptor {
                action_id: crate::ActionId::from(format!("workspace.files/{action}")),
                input_schema: crate::CanonicalSchemaRef::from(format!("schema://workspace.files/{action}/input")),
                output_schema: crate::CanonicalSchemaRef::from(format!("schema://workspace.files/{action}/output")),
                effect_class: crate::EffectClass::ReadSensitive,
                presentation: crate::ToolPresentationKind::FunctionTool,
            })
            .collect();
        module.action_allowlist = BTreeSet::from([crate::ActionId::from("workspace.files/read")]);
        assert!(module.validate().is_ok());

        let mut empty = module.clone();
        empty.action_allowlist.clear();
        assert!(empty.validate().unwrap_err().message.contains("explicit non-empty action grant"));

        module.action_allowlist = BTreeSet::from([crate::ActionId::from("workspace.files/delete")]);
        assert!(module.validate().unwrap_err().message.contains("undeclared action"));
    }

    #[test]
    fn legacy_contribution_serialization_does_not_invent_dependency_edges() {
        let capability = resolved_capability("root", &"a".repeat(64));
        let value = serde_json::to_value(&capability).unwrap();
        assert!(value.get("consumption").is_none());
        assert!(value.get("dependency_refs").is_none());
        let decoded: ResolvedCapability = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded, capability);
        assert_eq!(digest_payload(&decoded).unwrap(), digest_payload(&value).unwrap());
    }

    #[test]
    fn context_order_is_optional_unique_selected_and_digest_covered() {
        let selected = BTreeSet::from([crate::CapabilityId::from("a"), "z".into()]);
        assert!(validate_context_order(&["z".into(), "a".into()], &selected).is_ok());
        assert!(validate_context_order(&["z".into(), "z".into()], &selected).is_err());
        assert!(validate_context_order(&["missing".into()], &selected).is_err());
        let mut content = snapshot_content(Vec::new(), Vec::new());
        let legacy = serde_json::to_value(&content).unwrap();
        assert!(legacy.get("context_order").is_none());
        assert_eq!(serde_json::from_value::<ResolvedSnapshotContent>(legacy.clone()).unwrap(), content);
        assert_eq!(digest_payload(&legacy).unwrap(), digest_payload(&content).unwrap());
        content.context_order = vec!["z".into(), "a".into()];
        let first = digest_payload(&content).unwrap();
        content.context_order.reverse();
        assert_ne!(first, digest_payload(&content).unwrap());
        assert!(envelope(content).validate().unwrap_err().message.contains("context_order"));
    }

    #[test]
    fn middleware_order_is_optional_unique_selected_and_digest_covered() {
        let mut content = snapshot_content(vec![resolved_capability("a", &"a".repeat(64)), resolved_capability("z", &"b".repeat(64))], Vec::new());
        let legacy = serde_json::to_value(&content).unwrap();
        assert!(legacy.get("middleware_order").is_none());
        assert_eq!(serde_json::from_value::<ResolvedSnapshotContent>(legacy.clone()).unwrap(), content);
        assert_eq!(digest_payload(&legacy).unwrap(), digest_payload(&content).unwrap());
        content.middleware_order = vec!["z".into(), "a".into()];
        assert!(envelope(content.clone()).validate().is_ok());
        let first = digest_payload(&content).unwrap();
        content.middleware_order.reverse();
        assert_ne!(first, digest_payload(&content).unwrap());
        for order in [vec!["z", "z"], vec!["missing"]] {
            let mut invalid = content.clone();
            invalid.middleware_order = order.into_iter().map(Into::into).collect();
            assert!(envelope(invalid).validate().unwrap_err().message.contains("middleware_order"));
        }
        content.enabled_capabilities[0].consumption = CapabilityConsumption::Dependency;
        content.enabled_capabilities[1].dependency_refs = vec![content.enabled_capabilities[0].capability.clone()];
        assert!(envelope(content).validate().unwrap_err().message.contains("middleware_order"));
    }

    #[test]
    fn resource_neutral_preset_contract_rejects_legacy_resource_fields() {
        let mut payload = serde_json::to_value(AgentPresetRevisionPayload {
            context_order: Vec::new(),
            middleware_order: Vec::new(),
            schema_version: VersionString::from("1.0.0"),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            enabled_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            system_role_provider_overrides: BTreeMap::new(),
            persona: String::new(),
            instructions: String::new(),
            starter_prompts: Vec::new(),
            runtime_policy: Default::default(),
        })
        .unwrap();
        assert!(payload.get("context_order").is_none());
        assert!(payload.get("runtime_policy").is_none());
        let legacy: AgentPresetRevisionPayload = serde_json::from_value(payload.clone()).unwrap();
        assert!(legacy.context_order.is_empty());
        assert!(payload.get("middleware_order").is_none());
        assert!(legacy.middleware_order.is_empty());
        assert_eq!(digest_payload(&payload).unwrap(), digest_payload(&legacy).unwrap());
        payload["resource_bindings"] = serde_json::json!([]);
        assert!(
            serde_json::from_value::<AgentPresetRevisionPayload>(payload).is_err(),
            "Revision payloads must not freeze concrete resource bindings"
        );

        let mut selection = serde_json::to_value(CapabilitySelection {
            capability: capability("knowledge"),
            action_allowlist: BTreeSet::from([ActionId::from("knowledge/search")]),
        })
        .unwrap();
        selection["resource_binding_refs"] = serde_json::json!(["knowledge"]);
        assert!(
            serde_json::from_value::<CapabilitySelection>(selection).is_err(),
            "Capability selections must declare capability intent without resource identities"
        );

        let mut snapshot =
            serde_json::to_value(snapshot_content(Vec::new(), Vec::new()))
                .unwrap();
        snapshot["typed_resource_bindings"] = serde_json::json!([]);
        assert!(
            serde_json::from_value::<ResolvedSnapshotContent>(snapshot).is_err(),
            "Snapshots must not freeze target-scoped resource bindings"
        );

        let mut nested_snapshot =
            serde_json::to_value(snapshot_content(Vec::new(), Vec::new()))
                .unwrap();
        nested_snapshot["on_demand_activation_plans"] = serde_json::json!({
            "knowledge": {
                "root_capability_id": "knowledge",
                "capability_bundle": [],
                "tool_schema_refs": [],
                "context_schema_refs": [],
                "resource_binding_refs": [],
                "model_route_refs": []
            }
        });
        assert!(
            serde_json::from_value::<ResolvedSnapshotContent>(nested_snapshot).is_err(),
            "Nested Snapshot records must fail closed on retired resource fields"
        );
    }

    #[test]
    fn agent_runtime_selection_is_not_part_of_the_preset_contract() {
        let legacy = serde_json::json!({
            "schema_version":"1.0.0", "model_route_refs":{},
            "enabled_capabilities":[], "skill_bindings":[],
            "persona":"", "instructions":""
        });
        let payload: AgentPresetRevisionPayload = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(&payload).unwrap(), legacy);
        let mut selected = legacy;
        selected["runtime_build"] = serde_json::json!({
            "selector": {"selection":"exact","family_id":"customer.workflow","build_id":"v1","build_digest":"a".repeat(64)},
            "profile":"custom"
        });
        assert!(serde_json::from_value::<AgentPresetRevisionPayload>(selected).is_err());
    }

    #[test]
    fn resolved_mcp_tool_lock_requires_canonical_identity_digest_and_revision() {
        let digest = "a".repeat(64);
        let valid = mcp_lock("server-1", "vendor.echo", "capability.echo", &digest);
        assert!(valid.validate().is_ok());

        let mut empty_server = valid.clone();
        empty_server.server_id = McpServerId::from(" ");
        assert!(empty_server.validate().is_err());

        let mut noncanonical_tool = valid.clone();
        noncanonical_tool.canonical_tool_key = McpToolKey::from(" vendor.echo");
        assert!(noncanonical_tool.validate().is_err());

        let mut empty_capability = valid.clone();
        empty_capability.capability_id = crate::CapabilityId::from("");
        assert!(empty_capability.validate().is_err());

        let mut uppercase_digest = valid.clone();
        uppercase_digest.schema_digest = DigestHex::from("A".repeat(64));
        assert!(uppercase_digest.validate().is_err());

        let mut short_digest = valid.clone();
        short_digest.schema_digest = DigestHex::from("a".repeat(63));
        assert!(short_digest.validate().is_err());

        let mut zero_revision = valid;
        zero_revision.materialization_revision = 0;
        assert!(zero_revision.validate().is_err());
    }

    #[test]
    fn resolved_snapshot_rejects_duplicate_mcp_lock_identity_and_capability() {
        let digest = "a".repeat(64);
        let initial = vec![
            resolved_capability("capability.echo", &digest),
            resolved_capability("capability.other", &digest),
        ];
        let first = mcp_lock("server-1", "vendor.echo", "capability.echo", &digest);

        let duplicate_identity = envelope(snapshot_content(
            initial.clone(),
            vec![first.clone(), first.clone()],
        ));
        assert!(duplicate_identity.validate().is_err());

        let duplicate_capability = envelope(snapshot_content(
            initial,
            vec![
                first,
                mcp_lock(
                    "server-2",
                    "vendor.other",
                    "capability.echo",
                    &digest,
                ),
            ],
        ));
        assert!(duplicate_capability.validate().is_err());
    }

    #[test]
    fn resolved_snapshot_mcp_lock_matches_enabled_capability_schema() {
        let digest = "a".repeat(64);
        let initial = vec![resolved_capability("capability.initial", &digest), resolved_capability("capability.extra", &digest)];

        let valid = envelope(snapshot_content(
            initial.clone(),
            vec![
                mcp_lock(
                    "server-1",
                    "vendor.initial",
                    "capability.initial",
                    &digest,
                ),
                mcp_lock("server-1", "vendor.extra", "capability.extra", &digest),
            ],
        ));
        assert!(valid.validate().is_ok());

        let unknown = envelope(snapshot_content(
            initial.clone(),
            vec![mcp_lock(
                "server-1",
                "vendor.unknown",
                "capability.unknown",
                &digest,
            )],
        ));
        assert!(unknown.validate().is_err());

        let independent_tool_schema_digest = envelope(snapshot_content(
            initial,
            vec![mcp_lock(
                "server-1",
                "vendor.initial",
                "capability.initial",
                &"b".repeat(64),
            )],
        ));
        assert!(
            independent_tool_schema_digest.validate().is_ok(),
            "MCP action schema digest is independent from the full capability manifest digest"
        );
    }

    #[test]
    fn official_key_type_is_the_exact_six_key_set() {
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
            ])
        );
        assert!(!actual.contains("research"));
    }

    #[test]
    fn direct_creation_does_not_require_chat_and_general_assistants_still_do() {
        assert!(is_direct_creation_agent(["creation.media", "creative.workshop"]));
        assert!(is_direct_creation_agent(["creation.media", "office"]));
        assert!(!is_direct_creation_agent(["creation.media", "web.research"]));
        assert!(!is_direct_creation_agent(["creative.workshop"]));
        assert!(!is_direct_creation_agent(std::iter::empty()));
    }

    #[test]
    fn enabled_capabilities_reject_duplicate_ids() {
        let error = validate_capability_selections(&[
            CapabilitySelection {
                capability: capability("workspace.files"),
                action_allowlist: BTreeSet::from([ActionId::from("workspace.files/read")]),
            },
            CapabilitySelection {
                capability: capability("workspace.files"),
                action_allowlist: BTreeSet::from([ActionId::from("workspace.files/read")]),
            },
        ])
        .unwrap_err();
        assert_eq!(error.code.as_ref(), PRESET_CAPABILITY_DUPLICATE);
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
    fn canonical_api_inventory_has_no_test_or_legacy_resource() {
        let inventory = canonical_api_inventory_payload();
        assert!(inventory.operations.iter().all(|operation| {
            !operation.path.starts_with(&["/api/", "presets"].concat())
                && !operation.path.starts_with("/api/conversations")
                && !operation.path.contains("/test")
        }));
        assert!(inventory
            .canonical_error_codes
            .iter()
            .all(|code| code.as_ref() == code.as_ref().to_ascii_uppercase()));
    }
}
