//! Production Fresh-v4 Agent platform composition.
//!
//! The canonical Agent platform owns a validated Fresh-v4 pool and its
//! registration inventory. It never accepts a legacy application service graph
//! or a v3 database pool.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use nomifun_agent_contracts::{
    CodingRuntimeFeatureInventoryPayload, DigestHex, FreshV4ReadyMarker,
    FreshV4SchemaMetadata, RuntimeProfileKind, RuntimeTarget, VersionString,
    canonical_json_bytes, digest_bytes, digest_payload,
    fresh_v4_schema_manifest_payload, official_preset_seed_manifest_payload,
    FRESH_V4_BASELINE_SQL, FRESH_V4_DATA_GENERATION, FRESH_V4_MIGRATION_HEAD,
    FRESH_V4_PROJECTION_SCHEMA_VERSION,
};
use nomifun_agent_control_plane::CompilerReleaseInputs;
use nomifun_agent_domain_wave1::{
    Wave1CapabilityOperation, Wave1FetchRequest, Wave1HostPort, Wave1HostPortError,
    Wave1HostRequest, Wave1KnowledgeReadRequest, Wave1MemoryMutationRequest,
    Wave1SearchRequest,
};
use nomifun_agent_kernel::{
    CompilerEnvironment, MaterializationPolicy, MAX_PLUGIN_STATE_BYTES,
    MAX_PLUGIN_STATE_KEY_BYTES,
};
use nomifun_agent_domain_wave2::{Wave2HostPort, Wave2RoleHostPorts};
use nomifun_agent_domain_wave3::Wave3HostPort;
use nomifun_agent_domain_wave4::Wave4HostPort;
use nomifun_agent_domain_wave5::Wave5HostPort;
use nomifun_agent_platform::{
    AgentPlatform, AgentPlatformConfig, ChatExecutionAuthority,
    BrokerBackedRuntimePort, ProductionChatCausalityGate,
    RuntimeStartTurnBrokerBridge, SupervisedCodexRuntimePort,
};
use nomifun_agent_session::AgentSessionStore;
use nomifun_codex_runtime::CodexRuntimeSupervisor;
use nomifun_v4_root::application_build_digest;
#[cfg(test)]
use nomifun_v4_root::{
    FRESH_V4_DATABASE_FILE, FRESH_V4_READY_MARKER_FILE,
    canonical_schema_manifest_digest,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;

#[cfg(any(feature = "browser-use", feature = "computer-use"))]
use super::agent_role_host::RoleHostPortAdapter;
use super::agent_wave2_host::Wave2ApplicationHost;
use super::agent_wave2_mcp::{
    McpOwnerAdapter, SqliteMcpRuntimeBindingSource,
};
use super::agent_wave4_host::Wave4ApplicationHost;
use super::chat_broker_host::{
    ChatBrokerHostComposition, ConnectionCredentialLeaseRegistry,
    SqliteChatOperationClaimStore,
};
use crate::bootstrap::APPLICATION_BUILD_IDENTITY;

const CONTRACT_VERSION: &str = "1.0.0";
const C7_AVAILABILITY_REVISION: &str = "c7-windows-continuous-2026-08-30";
const BASELINE_MIGRATION_NAME: &str = "0001_fresh_v4";
const MAX_READY_MARKER_BYTES: u64 = 64 * 1024;
const MOUNT_INITIALIZATION_TIMEOUT: Duration = Duration::from_secs(30);
const MODEL_MEDIA_PACKAGE_ID: &str = "nomifun.model-media";
const RUNTIME_FEATURE_INVENTORY_JSON: &str = include_str!(
    "../../../nomifun-agent-contracts/contracts/runtime/coding-runtime-feature-inventory.payload.json"
);

/// The first concrete Wave 1 owner mounted by the Fresh-v4 host.
///
/// URL fetching already has a standalone, SSRF-checked domain owner. This
/// adapter exposes that real operation, binding-backed Knowledge reads, and a
/// bounded first-party memory mutation owner backed by the Kernel PluginState
/// API. Research search, Knowledge mutations, and Skill actions remain
/// fail-closed until their v4 owners are available.
#[derive(Clone, Default)]
struct Wave1ApplicationHost {
    fetcher: nomifun_knowledge::source_url::HttpFetcher,
    knowledge_reader: nomifun_knowledge::BoundKnowledgeReadService,
}

const KNOWLEDGE_ROOT_PARAMETER: &str = "knowledge_root";
const KNOWLEDGE_NAME_PARAMETER: &str = "knowledge_name";
const DEFAULT_KNOWLEDGE_SEARCH_LIMIT: usize = 20;
const MEMORY_STATE_KEY: &str = "memory.entries";
const MEMORY_STATE_FORMAT_VERSION: &str = "1.0.0";
const MAX_MEMORY_ENTRIES: usize = 128;
const MAX_MEMORY_ENTRY_BYTES: usize = 16 * 1024;
const MAX_MEMORY_CAS_ATTEMPTS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wave1MemoryOperation {
    ProjectWrite,
    ProjectDistill,
    CompanionWrite,
    CompanionMerge,
    CompanionEvolve,
}

impl Wave1MemoryOperation {
    fn label(self) -> &'static str {
        match self {
            Self::ProjectWrite => "project.write",
            Self::ProjectDistill => "project.distill",
            Self::CompanionWrite => "companion.write",
            Self::CompanionMerge => "companion.merge",
            Self::CompanionEvolve => "companion.evolve",
        }
    }

    fn capability_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            Self::ProjectDistill => nomifun_agent_domain_wave1::MEMORY_PROJECT_DISTILL,
            Self::CompanionWrite => nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
            Self::CompanionMerge => nomifun_agent_domain_wave1::MEMORY_COMPANION_MERGE,
            Self::CompanionEvolve => nomifun_agent_domain_wave1::MEMORY_COMPANION_EVOLVE,
        }
    }

    fn package_id(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID
            }
            Self::CompanionWrite | Self::CompanionMerge | Self::CompanionEvolve => {
                nomifun_agent_domain_wave1::COMPANION_MEMORY_PACKAGE_ID
            }
        }
    }

    fn mount_id(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID
            }
            Self::CompanionWrite | Self::CompanionMerge | Self::CompanionEvolve => {
                nomifun_agent_domain_wave1::COMPANION_MEMORY_MOUNT_ID
            }
        }
    }

    fn resource_kind(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_RESOURCE_KIND
            }
            Self::CompanionWrite | Self::CompanionMerge | Self::CompanionEvolve => {
                nomifun_agent_domain_wave1::COMPANION_MEMORY_RESOURCE_KIND
            }
        }
    }

    fn is_project(self) -> bool {
        matches!(self, Self::ProjectWrite | Self::ProjectDistill)
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "project.write" => Some(Self::ProjectWrite),
            "project.distill" => Some(Self::ProjectDistill),
            "companion.write" => Some(Self::CompanionWrite),
            "companion.merge" => Some(Self::CompanionMerge),
            "companion.evolve" => Some(Self::CompanionEvolve),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl Wave1HostPort for Wave1ApplicationHost {
    async fn invoke(
        &self,
        request: Wave1HostRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let nomifun_agent_domain_wave1::Wave1HostRequest { context, operation } = request;
        match operation {
            Wave1CapabilityOperation::ResearchFetch(Wave1FetchRequest { url }) => {
                let page = self
                    .fetcher
                    .fetch_page(&url)
                    .await
                    .map_err(wave1_application_error)?;
                Ok(nomifun_agent_contracts::StrictJsonValue(serde_json::json!({
                    "url": page.final_url,
                    "title": page.title,
                    "markdown": page.markdown,
                    "truncated": page.truncated
                })))
            }
            Wave1CapabilityOperation::KnowledgeSearch(request) => {
                self.search_knowledge(context, request).await
            }
            Wave1CapabilityOperation::KnowledgeRead(request) => {
                self.read_knowledge(context, request).await
            }
            Wave1CapabilityOperation::ProjectMemoryWrite(request) => {
                self.persist_memory(
                    context,
                    Wave1MemoryOperation::ProjectWrite,
                    request,
                )
                .await
            }
            Wave1CapabilityOperation::ProjectMemoryDistill(request) => {
                self.persist_memory(
                    context,
                    Wave1MemoryOperation::ProjectDistill,
                    request,
                )
                .await
            }
            Wave1CapabilityOperation::CompanionMemoryWrite(request) => {
                self.persist_memory(
                    context,
                    Wave1MemoryOperation::CompanionWrite,
                    request,
                )
                .await
            }
            Wave1CapabilityOperation::CompanionMemoryMerge(request) => {
                self.persist_memory(
                    context,
                    Wave1MemoryOperation::CompanionMerge,
                    request,
                )
                .await
            }
            Wave1CapabilityOperation::CompanionMemoryEvolve(request) => {
                self.persist_memory(
                    context,
                    Wave1MemoryOperation::CompanionEvolve,
                    request,
                )
                .await
            }
            operation => Err(Wave1HostPortError::unavailable(format!(
                "no Fresh-v4 Wave 1 owner is wired for {}",
                operation.capability_id().as_ref()
            ))),
        }
    }
}

impl Wave1ApplicationHost {
    async fn search_knowledge(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1SearchRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let knowledge_base = resolve_bound_knowledge_base(
            &context,
            nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
            "search",
        )?;
        let hits = self
            .knowledge_reader
            .search(
                &knowledge_base,
                &request.query,
                request.limit.unwrap_or(DEFAULT_KNOWLEDGE_SEARCH_LIMIT),
            )
            .await
            .map_err(wave1_bound_knowledge_error)?;
        Ok(nomifun_agent_contracts::StrictJsonValue(
            serde_json::json!({
                "resource_id": knowledge_base.knowledge_base_id(),
                "total": hits.len(),
                "hits": hits,
            }),
        ))
    }

    async fn read_knowledge(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1KnowledgeReadRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let knowledge_base = resolve_bound_knowledge_base(
            &context,
            nomifun_agent_domain_wave1::KNOWLEDGE_READ,
            "read",
        )?;
        let handle_resource_id = nomifun_knowledge::decode_doc_handle(
            &request.handle,
        )
        .map(|(knowledge_base_id, _)| knowledge_base_id)
        .ok_or_else(|| {
            Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                "invalid knowledge document handle",
            )
        })?;
        if &handle_resource_id != knowledge_base.knowledge_base_id() {
            return Err(Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "knowledge document handle points to a different bound resource",
            ));
        }
        let document = self
            .knowledge_reader
            .read(&knowledge_base, &request.handle)
            .await
            .map_err(wave1_bound_knowledge_error)?;
        Ok(nomifun_agent_contracts::StrictJsonValue(
            serde_json::json!(document),
        ))
    }

    /// Persist the bounded memory mutation in the package's namespace-scoped
    /// PluginState store. This is a real v4-owned state transition, not a
    /// synthetic action receipt; the operation is retried only on a bounded
    /// CAS conflict and never routed to a legacy memory service.
    async fn persist_memory(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        operation: Wave1MemoryOperation,
        request: Wave1MemoryMutationRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        use nomifun_agent_contracts::{
            PluginStateCompareAndSwapOutcome, StateKey, StrictJsonValue, VersionString,
        };

        let state = context.state.clone();
        let expected_action = nomifun_agent_domain_wave1::action_id(operation.capability_id())
            .expect("every memory mutation has a canonical action");
        if context.capability_id.as_ref() != operation.capability_id()
            || context.action_id != expected_action
        {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!(
                    "{} operation identity does not match the host context",
                    operation.label()
                ),
            ));
        }

        let descriptor = state.descriptor();
        if descriptor.package_id.as_ref() != operation.package_id()
            || descriptor.mount_id.as_ref() != operation.mount_id()
        {
            return Err(Wave1HostPortError::unavailable(format!(
                "{} state handle is mounted as {}/{}",
                operation.label(),
                descriptor.package_id.as_ref(),
                descriptor.mount_id.as_ref()
            )));
        }

        // Project/Companion memory is shared by the exact bound resource, not
        // by a transient Session. A missing or ambiguous target is a wiring
        // error and must fail closed rather than silently falling back to the
        // session scope.
        let matching_bindings = context
            .resource_bindings
            .iter()
            .filter(|binding| binding.resource_kind.as_ref() == operation.resource_kind())
            .collect::<Vec<_>>();
        let binding = match matching_bindings.as_slice() {
            [binding] => *binding,
            [] => {
                return Err(Wave1HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "{} has no bound {} resource",
                        operation.label(),
                        operation.resource_kind()
                    ),
                ));
            }
            _ => {
                return Err(Wave1HostPortError::new(
                    "PRESET_RESOURCE_NOT_BOUND",
                    format!(
                        "{} requires exactly one bound {} resource",
                        operation.label(),
                        operation.resource_kind()
                    ),
                ));
            }
        };
        if binding.owner_id != context.principal.principal_id {
            return Err(Wave1HostPortError::new(
                "RESOURCE_OWNER_MISMATCH",
                format!(
                    "{} resource {} is owned by a different principal",
                    operation.label(),
                    binding.resource_id.as_ref()
                ),
            ));
        }
        if !binding.operations.contains("write") {
            return Err(Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "{} resource binding {} does not grant write",
                    operation.label(),
                    binding.binding_id.as_ref()
                ),
            ));
        }
        if binding.resource_id.as_ref().trim().is_empty() {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!("{} resource ID must not be blank", operation.label()),
            ));
        }

        let scope = nomifun_agent_contracts::ScopeKey::from(format!(
            "resource:{}",
            binding.resource_id.as_ref()
        ));
        if scope.as_ref().len() > MAX_PLUGIN_STATE_KEY_BYTES {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!(
                    "{} resource scope exceeds {MAX_PLUGIN_STATE_KEY_BYTES} bytes",
                    operation.label()
                ),
            ));
        }
        let state_key = StateKey::from(MEMORY_STATE_KEY);
        let format = VersionString::from(MEMORY_STATE_FORMAT_VERSION);

        validate_memory_request(&request, operation.label())?;
        if context.idempotency_key.as_ref().trim().is_empty() {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                "memory mutation requires a non-empty idempotency key",
            ));
        }
        let request_value = memory_request_value(request);
        let request_digest = memory_request_digest(operation, binding, &request_value)?;
        let mut entry = serde_json::json!({
            "operation": operation.label(),
            "request": request_value,
            "request_digest": request_digest,
            "idempotency_key": context.idempotency_key.as_ref(),
            "operation_id": context.operation_id.as_ref(),
            "correlation_id": context.correlation_id.as_ref(),
        });
        for _attempt in 0..MAX_MEMORY_CAS_ATTEMPTS {
            let current = state
                .get(&scope, &state_key)
                .await
                .map_err(|error| {
                    Wave1HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("memory state could not be read: {error}"),
                    )
                })?;
            let revision = current.as_ref().map(|entry| entry.revision).unwrap_or(0);
            let mut entries = decode_memory_state(
                current.as_ref(),
                &format,
                operation,
                binding,
            )?;
            if let Some(previous) = entries.iter().find(|previous| {
                previous
                    .get("idempotency_key")
                    .and_then(serde_json::Value::as_str)
                    == Some(context.idempotency_key.as_ref())
            }) {
                let Some(previous_digest) = previous
                    .get("request_digest")
                    .and_then(serde_json::Value::as_str)
                else {
                    return Err(Wave1HostPortError::unavailable(
                        "memory state idempotency record has no request digest",
                    ));
                };
                if previous_digest != request_digest.as_ref() {
                    return Err(Wave1HostPortError::new(
                        "IDEMPOTENCY_CONFLICT",
                        format!(
                            "{} idempotency key was already used for different input",
                            operation.label()
                        ),
                    ));
                }
                let Some(result) = previous.get("result").cloned() else {
                    return Err(Wave1HostPortError::unavailable(
                        "memory state idempotency record has no replay result",
                    ));
                };
                return Ok(StrictJsonValue(result));
            }
            if entries.len() >= MAX_MEMORY_ENTRIES {
                return Err(Wave1HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "memory state reached its {MAX_MEMORY_ENTRIES} entry limit"
                    ),
                ));
            }
            let next_revision = revision.checked_add(1).ok_or_else(|| {
                Wave1HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    "memory state revision counter is exhausted",
                )
            })?;
            let entry_count = entries.len() + 1;
            let result = serde_json::json!({
                "persisted": true,
                "operation": operation.label(),
                "revision": next_revision,
                "entry_count": entry_count
            });
            let Some(entry_object) = entry.as_object_mut() else {
                return Err(Wave1HostPortError::new(
                    "INVALID_PAYLOAD",
                    "memory entry unexpectedly lost its object shape",
                ));
            };
            entry_object.insert("result".to_owned(), result.clone());
            let entry_bytes = canonical_json_bytes(&entry).map_err(|error| {
                Wave1HostPortError::new(
                    "INVALID_PAYLOAD",
                    format!("memory entry could not be encoded: {error}"),
                )
            })?;
            if entry_bytes.len() > MAX_MEMORY_ENTRY_BYTES {
                return Err(Wave1HostPortError::new(
                    "INVALID_PAYLOAD",
                    format!(
                        "memory entry exceeds {MAX_MEMORY_ENTRY_BYTES} bytes"
                    ),
                ));
            }
            entries.push(entry.clone());
            let next = StrictJsonValue(serde_json::json!({
                "entries": entries,
                "last_operation": operation.label(),
            }));
            let state_bytes = canonical_json_bytes(&next.0).map_err(|error| {
                Wave1HostPortError::new(
                    "INVALID_PAYLOAD",
                    format!("memory state could not be encoded: {error}"),
                )
            })?;
            if state_bytes.len() > MAX_PLUGIN_STATE_BYTES {
                return Err(Wave1HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!(
                        "memory state exceeds the {MAX_PLUGIN_STATE_BYTES}-byte PluginState limit"
                    ),
                ));
            }
            let response = state
                .compare_and_swap(
                    &scope,
                    &state_key,
                    revision,
                    &format,
                    Some(next),
                )
                .await
                .map_err(|error| {
                    Wave1HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!("memory state could not be committed: {error}"),
                    )
                })?;
            match response {
                PluginStateCompareAndSwapOutcome::Applied { revision }
                    if revision == next_revision =>
                {
                    return Ok(StrictJsonValue(result));
                }
                PluginStateCompareAndSwapOutcome::Applied { revision } => {
                    return Err(Wave1HostPortError::new(
                        "CAPABILITY_UNAVAILABLE",
                        format!(
                            "memory state committed unexpected revision {revision}, expected {next_revision}"
                        ),
                    ));
                }
                PluginStateCompareAndSwapOutcome::Conflict { .. } => continue,
            }
        }
        Err(Wave1HostPortError::new(
            "CAPABILITY_UNAVAILABLE",
            "memory state changed concurrently; bounded CAS retry exhausted",
        ))
    }
}

fn resolve_bound_knowledge_base(
    context: &nomifun_agent_domain_wave1::Wave1HostContext,
    capability_id: &str,
    operation: &str,
) -> Result<nomifun_knowledge::BoundKnowledgeBase, Wave1HostPortError> {
    resolve_bound_knowledge_base_parts(
        &context.principal.principal_id,
        &context.capability_id,
        &context.action_id,
        &context.resource_bindings,
        capability_id,
        operation,
    )
}

fn resolve_bound_knowledge_base_parts(
    principal_id: &str,
    actual_capability_id: &nomifun_agent_contracts::CapabilityId,
    actual_action_id: &nomifun_agent_contracts::ActionId,
    resource_bindings: &[nomifun_agent_contracts::TypedResourceBinding],
    capability_id: &str,
    operation: &str,
) -> Result<nomifun_knowledge::BoundKnowledgeBase, Wave1HostPortError> {
    let expected_action = nomifun_agent_domain_wave1::action_id(capability_id)
        .expect("every Knowledge owner capability has a canonical action");
    if actual_capability_id.as_ref() != capability_id
        || actual_action_id != &expected_action
    {
        return Err(Wave1HostPortError::new(
            "INVALID_PAYLOAD",
            format!(
                "{capability_id} operation identity does not match the host context"
            ),
        ));
    }

    let matching_bindings = resource_bindings
        .iter()
        .filter(|binding| {
            binding.resource_kind.as_ref()
                == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
        })
        .collect::<Vec<_>>();
    let binding = match matching_bindings.as_slice() {
        [binding] => *binding,
        [] => {
            return Err(Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "{capability_id} has no bound {} resource",
                    nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
                ),
            ));
        }
        _ => {
            return Err(Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "{capability_id} requires exactly one bound {} resource",
                    nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
                ),
            ));
        }
    };
    if binding.owner_id != principal_id {
        return Err(Wave1HostPortError::new(
            "RESOURCE_OWNER_MISMATCH",
            format!(
                "knowledge resource {} is owned by a different principal",
                binding.resource_id.as_ref()
            ),
        ));
    }
    if !binding.operations.contains(operation) {
        return Err(Wave1HostPortError::new(
            "PRESET_RESOURCE_NOT_BOUND",
            format!(
                "knowledge resource binding {} does not grant {operation}",
                binding.binding_id.as_ref()
            ),
        ));
    }

    let knowledge_base_id = nomifun_common::KnowledgeBaseId::parse(
        binding.resource_id.as_ref().to_owned(),
    )
    .map_err(|error| {
        Wave1HostPortError::new(
            "INVALID_PAYLOAD",
            format!(
                "knowledge resource ID must be a canonical UUIDv7: {error}"
            ),
        )
    })?;
    let root = binding
        .typed_parameters
        .get(KNOWLEDGE_ROOT_PARAMETER)
        .map(String::as_str)
        .filter(|root| !root.trim().is_empty())
        .ok_or_else(|| {
            Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                format!(
                    "knowledge resource binding {} has no {KNOWLEDGE_ROOT_PARAMETER}",
                    binding.binding_id.as_ref()
                ),
            )
        })?;
    let name = match binding.typed_parameters.get(KNOWLEDGE_NAME_PARAMETER) {
        Some(name) if name.trim().is_empty() => {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!(
                    "knowledge resource binding {} has a blank {KNOWLEDGE_NAME_PARAMETER}",
                    binding.binding_id.as_ref()
                ),
            ));
        }
        Some(name) => name.trim().to_owned(),
        None => knowledge_base_id.as_str().to_owned(),
    };

    nomifun_knowledge::BoundKnowledgeBase::new(
        knowledge_base_id,
        name,
        PathBuf::from(root),
    )
    .map_err(wave1_application_error)
}

fn validate_memory_request(
    request: &Wave1MemoryMutationRequest,
    operation: &str,
) -> Result<(), Wave1HostPortError> {
    let has_content = request
        .content
        .as_deref()
        .is_some_and(|content| !content.trim().is_empty());
    let has_items = request
        .items
        .as_ref()
        .is_some_and(|items| !items.is_empty());
    if !has_content && !has_items {
        return Err(Wave1HostPortError::new(
            "INVALID_PAYLOAD",
            format!("{operation} requires non-empty content or items"),
        ));
    }
    if let Some(content) = request.content.as_deref() {
        if content.trim().is_empty() {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!("{operation} content must not be blank"),
            ));
        }
        if content.chars().count() > 65_536 {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!("{operation} content exceeds 65536 characters"),
            ));
        }
    }
    if let Some(title) = request.title.as_deref() {
        if title.trim().is_empty() || title.chars().count() > 512 {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!("{operation} title is blank or exceeds 512 characters"),
            ));
        }
    }
    if let Some(items) = request.items.as_ref() {
        if items.is_empty() || items.len() > MAX_MEMORY_ENTRIES {
            return Err(Wave1HostPortError::new(
                "INVALID_PAYLOAD",
                format!(
                    "{operation} items must contain between 1 and {MAX_MEMORY_ENTRIES} entries"
                ),
            ));
        }
    }
    Ok(())
}

fn decode_memory_state(
    current: Option<&nomifun_agent_contracts::PluginStateEntry>,
    expected_format: &nomifun_agent_contracts::VersionString,
    operation: Wave1MemoryOperation,
    binding: &nomifun_agent_contracts::TypedResourceBinding,
) -> Result<Vec<serde_json::Value>, Wave1HostPortError> {
    let Some(current) = current else {
        return Ok(Vec::new());
    };
    if current.revision == 0 {
        return Err(Wave1HostPortError::unavailable(
            "memory state has an invalid zero revision",
        ));
    }
    if current.state_format_version != *expected_format {
        return Err(Wave1HostPortError::unavailable(format!(
            "{} state format {} is unsupported; expected {}",
            operation.label(),
            current.state_format_version.as_ref(),
            expected_format.as_ref()
        )));
    }
    let object = current.value.0.as_object().ok_or_else(|| {
        Wave1HostPortError::unavailable("memory state has an invalid stored shape")
    })?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "entries" | "last_operation"))
    {
        return Err(Wave1HostPortError::unavailable(
            "memory state contains unknown top-level fields",
        ));
    }
    let entries = object
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            Wave1HostPortError::unavailable(
                "memory state has an invalid stored entries array",
            )
        })?;
    if entries.len() > MAX_MEMORY_ENTRIES {
        return Err(Wave1HostPortError::unavailable(format!(
            "memory state contains more than {MAX_MEMORY_ENTRIES} entries"
        )));
    }
    let entry_count = u64::try_from(entries.len()).map_err(|_| {
        Wave1HostPortError::unavailable("memory state entry count cannot be represented")
    })?;
    let first_entry_revision = current
        .revision
        .checked_sub(entry_count.saturating_sub(1))
        .ok_or_else(|| {
            Wave1HostPortError::unavailable(
                "memory state revision is older than its entry history",
            )
        })?;
    let mut idempotency_keys = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let entry_object = entry.as_object().ok_or_else(|| {
            Wave1HostPortError::unavailable(format!(
                "memory state entry {index} is not an object"
            ))
        })?;
        if entry_object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "operation"
                    | "request"
                    | "request_digest"
                    | "idempotency_key"
                    | "operation_id"
                    | "correlation_id"
                    | "result"
            )
        }) {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} contains unknown fields"
            )));
        }
        let entry_operation = entry_object
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} has no operation"
                ))
            })?;
        let Some(entry_domain) = Wave1MemoryOperation::from_label(entry_operation) else {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} has an unknown operation"
            )));
        };
        if entry_domain.is_project() != operation.is_project() {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} crosses project/companion state domains"
            )));
        }
        let key = entry_object
            .get("idempotency_key")
            .and_then(serde_json::Value::as_str)
            .filter(|key| !key.trim().is_empty())
            .ok_or_else(|| {
                Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} has an invalid idempotency key"
                ))
            })?;
        if !idempotency_keys.insert(key.to_owned()) {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state contains duplicate idempotency key at entry {index}"
            )));
        }
        let request = entry_object.get("request").ok_or_else(|| {
            Wave1HostPortError::unavailable(format!(
                "memory state entry {index} has no request"
            ))
        })?;
        validate_stored_memory_request(request, operation.label(), index)?;
        let request_digest = entry_object
            .get("request_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|digest| {
                digest.len() == 64
                    && digest.bytes().all(|byte| {
                        byte.is_ascii_digit()
                            || (b'a'..=b'f').contains(&byte)
                    })
            })
            .ok_or_else(|| {
                Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} has an invalid request digest"
                ))
            })?;
        let expected_digest =
            memory_request_digest(entry_domain, binding, request).map_err(|error| {
                Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} request digest could not be recomputed: {error}"
                ))
            })?;
        if request_digest != expected_digest.as_ref() {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} request digest does not match its request"
            )));
        }
        for field in ["operation_id", "correlation_id"] {
            if entry_object
                .get(field)
                .and_then(serde_json::Value::as_str)
                .is_none_or(|value| value.trim().is_empty())
            {
                return Err(Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} has an invalid {field}"
                )));
            }
        }
        let result = entry_object
            .get("result")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                Wave1HostPortError::unavailable(format!(
                    "memory state entry {index} has no replay result"
                ))
            })?;
        if result.keys().any(|key| {
            !matches!(key.as_str(), "persisted" | "operation" | "revision" | "entry_count")
        })
            || result.get("persisted") != Some(&serde_json::Value::Bool(true))
            || result
                .get("operation")
                .and_then(serde_json::Value::as_str)
                != Some(entry_operation)
            || result
                .get("revision")
                .and_then(serde_json::Value::as_u64)
                != Some(
                    first_entry_revision
                        .checked_add(index as u64)
                        .ok_or_else(|| {
                            Wave1HostPortError::unavailable(
                                "memory state entry revision cannot be represented",
                            )
                        })?,
                )
            || result
                .get("entry_count")
                .and_then(serde_json::Value::as_u64)
                != Some(index as u64 + 1)
        {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} has an invalid replay result"
            )));
        }
        let entry_bytes = canonical_json_bytes(entry).map_err(|error| {
            Wave1HostPortError::unavailable(format!(
                "memory state entry {index} could not be encoded: {error}"
            ))
        })?;
        if entry_bytes.len() > MAX_MEMORY_ENTRY_BYTES {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} exceeds {MAX_MEMORY_ENTRY_BYTES} bytes"
            )));
        }
    }
    if let Some(last_operation) = object.get("last_operation") {
        let last_operation = last_operation
            .as_str()
            .ok_or_else(|| {
                Wave1HostPortError::unavailable(
                    "memory state last_operation is not a string",
                )
            })?;
        if entries
            .last()
            .and_then(|entry| entry.get("operation"))
            .and_then(serde_json::Value::as_str)
            != Some(last_operation)
        {
            return Err(Wave1HostPortError::unavailable(
                "memory state last_operation does not match its last entry",
            ));
        }
    } else if !entries.is_empty() {
        return Err(Wave1HostPortError::unavailable(
            "memory state has entries but no last_operation",
        ));
    }
    let state_bytes = canonical_json_bytes(&current.value.0).map_err(|error| {
        Wave1HostPortError::unavailable(format!(
            "memory state could not be encoded: {error}"
        ))
    })?;
    if state_bytes.len() > MAX_PLUGIN_STATE_BYTES {
        return Err(Wave1HostPortError::unavailable(format!(
            "memory state exceeds the {MAX_PLUGIN_STATE_BYTES}-byte PluginState limit"
        )));
    }
    Ok(entries.clone())
}

fn validate_stored_memory_request(
    value: &serde_json::Value,
    operation: &str,
    index: usize,
) -> Result<(), Wave1HostPortError> {
    let object = value.as_object().ok_or_else(|| {
        Wave1HostPortError::unavailable(format!(
            "memory state entry {index} request is not an object"
        ))
    })?;
    for key in object.keys() {
        if !matches!(key.as_str(), "content" | "title" | "items") {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} request contains unknown field {key:?}"
            )));
        }
    }
    let content = match object.get("content") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(content)) => Some(content.clone()),
        Some(_) => {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} request content is not a string"
            )));
        }
    };
    let title = match object.get("title") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(title)) => Some(title.clone()),
        Some(_) => {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} request title is not a string"
            )));
        }
    };
    let items = match object.get("items") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Array(items)) => Some(items.clone()),
        Some(_) => {
            return Err(Wave1HostPortError::unavailable(format!(
                "memory state entry {index} request items is not an array"
            )));
        }
    };
    validate_memory_request(
        &Wave1MemoryMutationRequest {
            content,
            title,
            items,
        },
        operation,
    )
    .map_err(|error| {
        Wave1HostPortError::unavailable(format!(
            "memory state entry {index} request is invalid: {error}"
        ))
    })
}

fn memory_request_value(request: Wave1MemoryMutationRequest) -> serde_json::Value {
    serde_json::json!({
        "content": request.content,
        "title": request.title,
        "items": request.items,
    })
}

fn memory_request_digest(
    operation: Wave1MemoryOperation,
    binding: &nomifun_agent_contracts::TypedResourceBinding,
    request: &serde_json::Value,
) -> Result<nomifun_agent_contracts::DigestHex, Wave1HostPortError> {
    let fingerprint = serde_json::json!({
        "operation": operation.label(),
        "capability_id": operation.capability_id(),
        "action_id": nomifun_agent_domain_wave1::action_id(operation.capability_id())
            .expect("every memory mutation has a canonical action"),
        "resource_kind": binding.resource_kind.as_ref(),
        "resource_id": binding.resource_id.as_ref(),
        "request": request,
    });
    digest_payload(&fingerprint).map_err(|error| {
        Wave1HostPortError::new(
            "INVALID_PAYLOAD",
            format!("memory request could not be canonicalized: {error}"),
        )
    })
}

fn wave1_application_error(error: nomifun_common::AppError) -> Wave1HostPortError {
    use nomifun_agent_contracts::CanonicalErrorCode;

    let code = match &error {
        nomifun_common::AppError::BadRequest(_) => "INVALID_PAYLOAD",
        nomifun_common::AppError::Timeout(_) => "CAPABILITY_UNAVAILABLE",
        nomifun_common::AppError::NotFound(_) => "RESOURCE_NOT_FOUND",
        nomifun_common::AppError::Forbidden(_) => "PRESET_RESOURCE_NOT_BOUND",
        _ => "CAPABILITY_UNAVAILABLE",
    };
    Wave1HostPortError::new(
        CanonicalErrorCode::from(code),
        error.to_string(),
    )
}

fn wave1_bound_knowledge_error(
    error: nomifun_common::AppError,
) -> Wave1HostPortError {
    use nomifun_agent_contracts::CanonicalErrorCode;

    let (code, message) = match error {
        nomifun_common::AppError::BadRequest(_) => (
            "INVALID_PAYLOAD",
            "bound knowledge request is invalid",
        ),
        nomifun_common::AppError::NotFound(_) => (
            "RESOURCE_NOT_FOUND",
            "bound knowledge document was not found",
        ),
        nomifun_common::AppError::Forbidden(_) => (
            "PRESET_RESOURCE_NOT_BOUND",
            "bound knowledge resource is outside the authorized scope",
        ),
        nomifun_common::AppError::Timeout(_) => (
            "CAPABILITY_UNAVAILABLE",
            "bound knowledge operation timed out",
        ),
        _ => (
            "CAPABILITY_UNAVAILABLE",
            "bound knowledge resource is unavailable",
        ),
    };
    Wave1HostPortError::new(CanonicalErrorCode::from(code), message)
}

/// All domain action hosts mounted into one Fresh-v4 AgentPlatform
/// generation.
///
/// The platform owns the registration generation, while each domain owns its
/// action vocabulary and typed host boundary. Keeping the five ports together
/// makes the composition seam explicit and gives the central owner one place
/// to replace a fail-closed/unconfigured domain with a real owner-backed
/// adapter. No domain port is allowed to reach back into `AppServices` or a
/// second Session authority.
#[derive(Clone)]
pub(crate) struct AgentDomainHostPorts {
    pub wave1: Arc<dyn Wave1HostPort>,
    pub wave2_roles: Wave2RoleHostPorts,
    pub wave3: Arc<dyn Wave3HostPort>,
    pub wave4: Arc<dyn Wave4HostPort>,
    pub wave5: Arc<dyn Wave5HostPort>,
}

impl AgentDomainHostPorts {
    fn for_workspace_root(workspace_root: PathBuf, pool: SqlitePool) -> Self {
        let mcp_source = Arc::new(SqliteMcpRuntimeBindingSource::new(pool));
        // Fresh-v4 intentionally does not carry the legacy `oauth_tokens`
        // table. Keep this release's supported MCP owner anonymous and let
        // credentialed/OAuth bindings fail with their typed unavailable
        // result instead of reintroducing a legacy schema dependency.
        let mcp_credentials: Arc<dyn nomifun_mcp::McpCredentialAuthority> =
            Arc::new(nomifun_mcp::AnonymousMcpCredentialAuthority);
        let mcp_owner = match nomifun_net::http_client_no_redirect() {
            Ok(client) => Some(Arc::new(McpOwnerAdapter::new(Arc::new(
                nomifun_mcp::McpOwner::new(
                    mcp_credentials,
                    client,
                ),
            )))),
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "canonical MCP owner HTTP client could not be constructed; MCP remains unavailable"
                );
                None
            }
        };
        let wave2: Arc<dyn Wave2HostPort> = match mcp_owner {
            Some(owner) => Arc::new(Wave2ApplicationHost::for_workspace_root_with_mcp(
                workspace_root.clone(),
                owner,
                mcp_source,
            )),
            None => Arc::new(Wave2ApplicationHost::for_workspace_root(
                workspace_root.clone(),
            )),
        };
        let mut wave2_roles = Wave2RoleHostPorts::with_actions(Arc::clone(&wave2));
        wave2_roles.browser_actions = nomifun_agent_domain_wave2::unconfigured_host_port();
        #[cfg(feature = "computer-use")]
        {
            let computer_invoker: Arc<super::agent_role_host::ComputerRoleInvoker> = Arc::new(
                super::agent_role_host::ComputerRoleInvoker::new(Arc::new(
                    nomi_computer::tool::ComputerTool::new(
                        &nomi_config::config::ComputerConfig::default(),
                    ),
                )),
            );
            wave2_roles.computer_actions = Arc::new(RoleHostPortAdapter::new(
                Arc::clone(&computer_invoker) as Arc<dyn super::agent_role_host::RoleHostInvoker>,
            ));
            wave2_roles.computer_contexts = computer_invoker;
        }
        Self {
            wave1: Arc::new(Wave1ApplicationHost::default()),
            wave2_roles,
            wave3: nomifun_agent_domain_wave3::unconfigured_host_port(),
            wave4: Arc::new(Wave4ApplicationHost),
            wave5: nomifun_agent_domain_wave5::unconfigured_host_port(),
        }
    }

    #[cfg(feature = "browser-use")]
    fn with_browser_hub(
        mut self,
        hub: Arc<nomifun_browser_platform::BrowserSessionHub>,
    ) -> Self {
        let browser_runtime =
            Arc::new(super::agent_role_host::BrowserRoleRuntime::new(hub));
        self.wave2_roles.browser_actions = Arc::new(RoleHostPortAdapter::new(
            Arc::clone(&browser_runtime) as Arc<dyn super::agent_role_host::RoleHostInvoker>,
        ));
        self.wave2_roles.browser_contexts = Arc::clone(&browser_runtime)
            as Arc<dyn nomifun_agent_domain_wave2::Wave2ContextHostPort>;
        self.wave2_roles.browser_operation_tools = Arc::clone(&browser_runtime)
            as Arc<dyn nomifun_agent_domain_wave2::Wave2OperationToolHostPort>;
        self.wave2_roles.browser_resources =
            browser_runtime as Arc<dyn nomifun_agent_domain_wave2::Wave2ResourceHostPort>;
        self
    }

    #[cfg(test)]
    fn with_wave1_and_wave2(
        wave1: Arc<dyn Wave1HostPort>,
        wave2: Arc<dyn Wave2HostPort>,
    ) -> Self {
        Self {
            wave1,
            wave2_roles: Wave2RoleHostPorts::with_actions(Arc::clone(&wave2)),
            wave3: nomifun_agent_domain_wave3::unconfigured_host_port(),
            wave4: Arc::new(Wave4ApplicationHost),
            wave5: nomifun_agent_domain_wave5::unconfigured_host_port(),
        }
    }
}

/// Build the canonical Agent platform from an already-open Fresh-v4 pool and
/// an optional provider pool.  A Fresh-v4 host passes its own pool here so
/// provider/model/connection facts remain in the same canonical database; the
/// explicit `None` path is retained for test fixtures that exercise the
/// fail-closed unconfigured broker shape.
#[cfg(not(feature = "browser-use"))]
pub(crate) async fn build_from_open_pool(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    workspace_root: PathBuf,
) -> anyhow::Result<Arc<AgentPlatform>> {
    let host_ports = AgentDomainHostPorts::for_workspace_root(
        workspace_root,
        pool.clone(),
    );
    build_from_open_pool_with_host_ports(
        pool,
        ready_path,
        marker,
        expected_schema_digest,
        provider_pool,
        encryption_key,
        host_ports,
    )
    .await
}

#[cfg(feature = "browser-use")]
pub(crate) async fn build_from_open_pool_with_browser(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    workspace_root: PathBuf,
    browser_hub: Option<Arc<nomifun_browser_platform::BrowserSessionHub>>,
) -> anyhow::Result<Arc<AgentPlatform>> {
    let host_ports =
        AgentDomainHostPorts::for_workspace_root(workspace_root, pool.clone());
    let host_ports = match browser_hub {
        Some(hub) => host_ports.with_browser_hub(hub),
        None => host_ports,
    };
    build_from_open_pool_with_host_ports(
        pool,
        ready_path,
        marker,
        expected_schema_digest,
        provider_pool,
        encryption_key,
        host_ports,
    )
    .await
}

#[cfg(feature = "browser-use")]
pub(crate) async fn build_browser_session_hub(
    data_dir: &Path,
    workspace_root: &Path,
    encryption_key: [u8; 32],
) -> anyhow::Result<Option<Arc<nomifun_browser_platform::BrowserSessionHub>>> {
    let browser_data = data_dir.join("browser-data");
    let platform_profiles = browser_data.join("platform-profiles");
    let recovery = tokio::task::spawn_blocking(move || {
        use nomi_browser_engine::profile::{
            ProfileRecoveryMode, ProfileRecoveryReport, recover_owned_profiles,
        };

        let mut report = ProfileRecoveryReport::default();
        for profiles_root in [
            browser_data.join("profiles"),
            platform_profiles.join("anonymous"),
            platform_profiles.join("replica"),
            platform_profiles.join("isolated"),
        ] {
            report.merge(recover_owned_profiles(
                &profiles_root,
                ProfileRecoveryMode::DeleteEphemeralProfile,
            ));
        }
        for stable_root in [
            browser_data.join("profile"),
            platform_profiles.join("primary"),
        ] {
            report.merge(recover_owned_profiles(
                &stable_root,
                ProfileRecoveryMode::PreserveStableProfile,
            ));
        }
        report
    })
    .await;
    let recovery = match recovery {
        Ok(report) => report,
        Err(error) => {
            tracing::error!(
                cancelled = error.is_cancelled(),
                panic = error.is_panic(),
                "Fresh-v4 Browser profile recovery worker failed; Browser remains unavailable"
            );
            return Ok(None);
        }
    };
    if recovery.failures != 0 || recovery.profiles_preserved != 0 {
        tracing::error!(
            summary = %recovery.safety_summary(),
            "Fresh-v4 Browser profile recovery was not proven safe; Browser remains unavailable"
        );
        return Ok(None);
    }

    let storage_state = nomi_browser_engine::load_storage_state(
        &nomi_browser_engine::shared_storage_state_path(data_dir),
        &encryption_key,
    )
    .map(nomi_browser_engine::StorageState::into_cookie_only)
    .and_then(|state| state.to_json().ok());
    let startup_identity_snapshot = storage_state.clone();
    let engine_config = nomi_browser_engine::EngineConfig {
        data_dir: data_dir.join("browser-data"),
        bundled_dir: crate::browser_resource::bundled_chrome_dir(),
        headful: false,
        chrome_source: nomi_browser_engine::ChromeSource::System,
        workspace_dir: Some(workspace_root.to_path_buf()),
        evaluate_full_power: false,
        evaluate_persistent_login: true,
        storage_state,
        ..Default::default()
    };
    let factory = nomi_browser::ManagedEngineHostFactory::new(engine_config)
        .with_identity_vault(
            nomi_browser_engine::shared_storage_state_path(data_dir),
            encryption_key,
        )
        .with_lane_policy(Arc::new(move |tool| {
            tool.persistent_login_key(encryption_key)
        }));
    let hub = Arc::new(nomifun_browser_platform::BrowserSessionHub::new(
        Arc::new(factory),
        nomifun_browser_platform::HubConfig::default(),
    ));
    if let Some(payload) = startup_identity_snapshot {
        hub.publish_identity_snapshot(
            nomifun_browser_platform::IdentitySnapshotPayload::from_json(payload),
            nomifun_browser_platform::SnapshotCoverage::cookies_only(),
        )
        .map_err(|error| anyhow::anyhow!("seed Browser identity snapshot: {error}"))?;
    }
    Ok(Some(hub))
}

/// Build the canonical Agent platform with explicit domain action ports.
///
/// This is the central composition seam for the migration waves. Production
/// callers may replace only the domains that have a real v4 owner; omitted
/// domains must be represented by their fail-closed port rather than by a
/// synthetic action implementation.
pub(crate) async fn build_from_open_pool_with_host_ports(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    host_ports: AgentDomainHostPorts,
) -> anyhow::Result<Arc<AgentPlatform>> {
    initialize_platform_with_cleanup_and_host_ports(
        pool,
        ready_path,
        marker,
        expected_schema_digest,
        provider_pool,
        encryption_key,
        host_ports,
    )
    .await
}

pub(crate) async fn open_validated_pool(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    Ok(SqlitePoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?)
}

#[cfg(test)]
async fn initialize_platform_with_cleanup(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    wave1_host: Arc<dyn Wave1HostPort>,
    wave2_host: Arc<dyn Wave2HostPort>,
) -> anyhow::Result<Arc<AgentPlatform>> {
    initialize_platform_with_cleanup_and_host_ports(
        pool,
        ready_path,
        marker,
        expected_schema_digest,
        provider_pool,
        encryption_key,
        AgentDomainHostPorts::with_wave1_and_wave2(wave1_host, wave2_host),
    )
    .await
}

async fn initialize_platform_with_cleanup_and_host_ports(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    host_ports: AgentDomainHostPorts,
) -> anyhow::Result<Arc<AgentPlatform>> {
    // AgentPlatform::from_pool takes ownership of the pool while it builds its
    // persistent adapters and publishes the initial generation. Keep one
    // cleanup handle outside that future so every ordinary initialization
    // failure closes the pool after all internal clones have been dropped.
    let cleanup_pool = pool.clone();
    let result = match tokio::time::timeout(
        MOUNT_INITIALIZATION_TIMEOUT,
        initialize_platform(
            pool,
            ready_path,
            marker,
            expected_schema_digest,
            provider_pool,
            encryption_key,
            host_ports,
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => Err(anyhow::anyhow!(
            "Fresh-v4 Agent platform initialization timed out after {} seconds: {error}",
            MOUNT_INITIALIZATION_TIMEOUT.as_secs()
        )),
    };
    match result {
        Ok(platform) => Ok(platform),
        Err(error) => {
            cleanup_pool.close().await;
            Err(error)
        }
    }
}

async fn initialize_platform(
    pool: SqlitePool,
    ready_path: PathBuf,
    marker: FreshV4ReadyMarker,
    expected_schema_digest: DigestHex,
    provider_pool: Option<SqlitePool>,
    encryption_key: [u8; 32],
    host_ports: AgentDomainHostPorts,
) -> anyhow::Result<Arc<AgentPlatform>> {
    validate_ready_marker(&marker, &expected_schema_digest)?;
    validate_schema_metadata(&pool, &marker, &expected_schema_digest).await?;
    // Bind the opened pool to the same immutable marker that was inspected
    // before the connection was established. The application lock normally
    // prevents this race; retaining the check makes a replacement fail closed.
    if read_ready_marker(&ready_path)? != marker {
        anyhow::bail!("Fresh-v4 ready marker changed while the mount was opening");
    }

    let feature_inventory: CodingRuntimeFeatureInventoryPayload =
        serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON)?;
    feature_inventory
        .validate()
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let feature_digest = digest_payload(&feature_inventory)?;
    let seed = official_preset_seed_manifest_payload();
    if feature_digest != seed.target_runtime_feature_inventory_digest {
        anyhow::bail!("runtime feature inventory digest differs from the frozen seed");
    }

    let mut policy = MaterializationPolicy::stable(CONTRACT_VERSION);
    policy.available_runtime_features = feature_inventory.runtime_features.clone();
    let kernel_environment = CompilerEnvironment {
        resolver_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
        required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
        runtime_feature_inventory_digest: feature_digest.clone(),
        available_runtime_features: feature_inventory.runtime_features,
        installation_role_bindings: BTreeMap::new(),
        canonical_schema_manifest_digest: expected_schema_digest.clone(),
        target_contribution_manifest_digest: seed.target_first_party_contribution_digest.clone(),
        host_target: current_runtime_target(),
        host_surface: current_host_surface(),
        availability_evidence_revision: C7_AVAILABILITY_REVISION.to_owned(),
    };
    let release = CompilerReleaseInputs {
        resolver_version: VersionString::from(CONTRACT_VERSION),
        runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
        runtime_feature_inventory_digest: feature_digest,
        canonical_schema_manifest_digest: expected_schema_digest,
        target_contribution_manifest_digest: seed.target_first_party_contribution_digest,
        availability_evidence_revision: C7_AVAILABILITY_REVISION.to_owned(),
    };

    let registrations = bundled_registrations_with_host_ports(host_ports)?;

    // Runtime process supervision is real and shared by all v4 Sessions. Its
    // constructor is inert: no Tokio task or child process exists until a
    // Session is launched. Compose the broker from exact v4 route records,
    // provider/connection repositories, a durable Session facts gate, and a
    // single-attempt provider transport.
    let sessions = Arc::new(AgentSessionStore::from_pool(pool.clone()).await?);
    let operation_claims = Arc::new(SqliteChatOperationClaimStore::new(sessions.clone()));
    let causality_gate = Arc::new(ProductionChatCausalityGate::new(
        sessions.clone(),
        operation_claims,
        ChatExecutionAuthority::Primary,
    ));
    let broker = match provider_pool {
        Some(provider_pool) => {
            let composition = ChatBrokerHostComposition::new(
                pool.clone(),
                provider_pool,
                encryption_key,
                ConnectionCredentialLeaseRegistry::new(),
            );
            let model_invoke = composition.build_model_invoke(
                nomifun_net::http_client_no_redirect()
                    .map_err(|error| anyhow::anyhow!("build provider HTTP client: {error}"))?,
            );
            composition.build_broker(
                causality_gate,
                model_invoke,
                nomifun_chat_model_broker::BrokerRetryPolicy::default(),
            )?
        }
        None => super::chat_broker_host::build_unconfigured_broker(
            causality_gate,
            encryption_key,
            nomifun_chat_model_broker::BrokerRetryPolicy::default(),
        )?,
    };
    let supervisor = Arc::new(CodexRuntimeSupervisor::new());
    let runtime_delegate = Arc::new(SupervisedCodexRuntimePort::new(supervisor));
    let runtime_bridge = Arc::new(RuntimeStartTurnBrokerBridge::new(
        Arc::clone(&sessions),
        broker.clone(),
    ));
    let runtime = Arc::new(BrokerBackedRuntimePort::new(
        runtime_delegate,
        runtime_bridge,
    ));
    let mut config = AgentPlatformConfig::with_runtime(
        pool,
        policy,
        release,
        kernel_environment,
        runtime,
        broker,
    );
    config.initial_plugins = registrations;
    // `initial_plugins` is the sole publication input for this host
    // generation. AgentPlatform publishes it once transactionally while it
    // constructs the platform; do not publish the same inventory a second
    // time from router assembly.
    Ok(AgentPlatform::from_pool(config).await?)
}

#[cfg(test)]
fn bundled_registrations(
    wave1_host: Arc<dyn Wave1HostPort>,
    wave2_host: Arc<dyn Wave2HostPort>,
) -> anyhow::Result<Vec<nomifun_agent_kernel::PluginRegistration>> {
    bundled_registrations_with_host_ports(AgentDomainHostPorts::with_wave1_and_wave2(
        wave1_host,
        wave2_host,
    ))
}

fn bundled_registrations_with_host_ports(
    host_ports: AgentDomainHostPorts,
) -> anyhow::Result<Vec<nomifun_agent_kernel::PluginRegistration>> {
    let target_specs = nomifun_agent_domain_support::c7_package_specs();
    let model_media_specs = target_specs
        .iter()
        .copied()
        .filter(|spec| spec.id == MODEL_MEDIA_PACKAGE_ID)
        .collect::<Vec<_>>();
    if model_media_specs.len() != 1 {
        anyhow::bail!(
            "C7 target inventory must contain exactly one {MODEL_MEDIA_PACKAGE_ID} package spec"
        );
    }
    let model_media_spec = model_media_specs[0];
    let mut registrations =
        Vec::with_capacity(target_specs.len());
    append_wave_registrations(
        &mut registrations,
        "Wave 1",
        &nomifun_agent_domain_wave1::PACKAGE_IDS,
        nomifun_agent_domain_wave1::registrations_with_host_port(host_ports.wave1),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 2",
        &nomifun_agent_domain_wave2::PACKAGE_IDS,
        nomifun_agent_domain_wave2::registrations_with_role_host_ports(host_ports.wave2_roles),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 3",
        &nomifun_agent_domain_wave3::PACKAGE_IDS,
        nomifun_agent_domain_wave3::registrations_with_host_port(host_ports.wave3),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 4",
        &nomifun_agent_domain_wave4::PACKAGE_IDS,
        nomifun_agent_domain_wave4::registrations_with_host_port(host_ports.wave4),
    )?;
    append_wave_registrations(
        &mut registrations,
        "Wave 5",
        &nomifun_agent_domain_wave5::PACKAGE_IDS,
        nomifun_agent_domain_wave5::registrations_with_host_port(host_ports.wave5),
    )?;
    registrations.push(
        nomifun_agent_domain_support::registration(model_media_spec)
            .map_err(|error| anyhow::anyhow!(error))?,
    );
    validate_bundled_registrations(&registrations, &target_specs)?;
    Ok(registrations)
}

fn append_wave_registrations(
    destination: &mut Vec<nomifun_agent_kernel::PluginRegistration>,
    wave_name: &str,
    expected_package_ids: &[&str],
    registrations: Result<
        Vec<nomifun_agent_kernel::PluginRegistration>,
        String,
    >,
) -> anyhow::Result<()> {
    let registrations =
        registrations.map_err(|error| anyhow::anyhow!("{wave_name} registration failed: {error}"))?;
    validate_registration_package_set(
        wave_name,
        &registrations,
        expected_package_ids,
    )?;
    destination.extend(registrations);
    Ok(())
}

fn validate_registration_package_set(
    owner: &str,
    registrations: &[nomifun_agent_kernel::PluginRegistration],
    expected_package_ids: &[&str],
) -> anyhow::Result<()> {
    let actual = registrations
        .iter()
        .map(|registration| {
            registration
                .metadata
                .manifest
                .payload
                .package_id
                .as_ref()
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    let expected = expected_package_ids
        .iter()
        .map(|package_id| (*package_id).to_owned())
        .collect::<BTreeSet<_>>();
    if actual != expected || registrations.len() != expected.len() {
        anyhow::bail!(
            "{owner} registration package set mismatch: expected {expected:?}, found {actual:?}"
        );
    }
    Ok(())
}

fn validate_bundled_registrations(
    registrations: &[nomifun_agent_kernel::PluginRegistration],
    target_specs: &[nomifun_agent_domain_support::PackageSpec],
) -> anyhow::Result<()> {
    nomifun_agent_domain_support::validate_inventory(registrations)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let mut expected_packages = BTreeSet::new();
    let mut expected_capabilities = BTreeMap::<String, BTreeSet<String>>::new();
    for spec in target_specs {
        if !expected_packages.insert(spec.id.to_owned()) {
            anyhow::bail!(
                "C7 target inventory contains duplicate package {}",
                spec.id
            );
        }
        let capabilities = expected_capabilities
            .entry(spec.id.to_owned())
            .or_default();
        for capability in spec.capabilities {
            if !capabilities.insert(capability.id.to_owned()) {
                anyhow::bail!(
                    "C7 target inventory contains duplicate capability {} in {}",
                    capability.id,
                    spec.id
                );
            }
        }
    }

    let mut actual_packages = BTreeSet::new();
    let mut actual_capabilities = BTreeMap::<String, BTreeSet<String>>::new();
    for registration in registrations {
        let manifest = &registration.metadata.manifest.payload;
        let package_id = manifest.package_id.as_ref().to_owned();
        if !actual_packages.insert(package_id.clone()) {
            anyhow::bail!(
                "bundled registration inventory publishes package {} more than once",
                package_id
            );
        }
        if manifest.package_version.as_ref() != CONTRACT_VERSION {
            anyhow::bail!(
                "bundled package {} has unexpected version {}",
                package_id,
                manifest.package_version.as_ref()
            );
        }
        let capabilities = actual_capabilities
            .entry(package_id)
            .or_default();
        for capability in &manifest.contributions.capabilities {
            if !capabilities.insert(capability.id.as_ref().to_owned()) {
                anyhow::bail!(
                    "bundled registration inventory publishes capability {} more than once",
                    capability.id.as_ref()
                );
            }
        }
    }

    if actual_packages != expected_packages
        || actual_capabilities != expected_capabilities
    {
        anyhow::bail!(
            "bundled C7 registration inventory differs from the frozen target inventory: \
             expected packages={expected_packages:?}, capabilities={expected_capabilities:?}; \
             found packages={actual_packages:?}, capabilities={actual_capabilities:?}"
        );
    }
    Ok(())
}

fn read_ready_marker(path: &Path) -> anyhow::Result<FreshV4ReadyMarker> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_or_reparse(&metadata) || !metadata.is_file() {
        anyhow::bail!(
            "Fresh-v4 ready marker must be a real regular file: {}",
            path.display()
        );
    }
    if metadata.len() > MAX_READY_MARKER_BYTES {
        anyhow::bail!(
            "Fresh-v4 ready marker exceeds the {} byte limit: {}",
            MAX_READY_MARKER_BYTES,
            path.display()
        );
    }

    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_READY_MARKER_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_READY_MARKER_BYTES {
        anyhow::bail!(
            "Fresh-v4 ready marker exceeds the {} byte limit: {}",
            MAX_READY_MARKER_BYTES,
            path.display()
        );
    }
    let marker: FreshV4ReadyMarker = serde_json::from_slice(&bytes)?;
    marker
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 ready marker: {error}"))?;
    if bytes != canonical_json_bytes(&marker)? {
        anyhow::bail!(
            "Fresh-v4 ready marker is not canonical JSON: {}",
            path.display()
        );
    }
    Ok(marker)
}

fn validate_ready_marker(
    marker: &FreshV4ReadyMarker,
    expected_schema_digest: &DigestHex,
) -> anyhow::Result<()> {
    marker
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 ready marker: {error}"))?;
    if marker.canonical_schema_manifest_digest != *expected_schema_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker schema digest differs from the canonical contract"
        );
    }
    let expected_build_digest =
        application_build_digest(APPLICATION_BUILD_IDENTITY)?;
    if marker.application_build_digest != expected_build_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker application build digest does not match this build"
        );
    }
    let expected_seed_digest =
        digest_payload(&official_preset_seed_manifest_payload())?;
    if marker.seed_manifest_digest != expected_seed_digest {
        anyhow::bail!(
            "Fresh-v4 ready marker seed manifest digest does not match the frozen seed"
        );
    }
    Ok(())
}

type SchemaObject = (String, String, String, Option<String>);

async fn validate_schema_metadata(
    pool: &SqlitePool,
    marker: &FreshV4ReadyMarker,
    schema_digest: &DigestHex,
) -> anyhow::Result<()> {
    let rows: Vec<(
        String,
        i64,
        String,
        i64,
        String,
        String,
        i64,
    )> = sqlx::query_as(
        "SELECT singleton_key, data_generation, root_instance_id, migration_head, \
                seed_manifest_digest, canonical_schema_manifest_digest, \
                projection_schema_version \
         FROM schema_metadata ORDER BY singleton_key",
    )
    .fetch_all(pool)
    .await?;
    if rows.len() != 1 {
        anyhow::bail!(
            "Fresh-v4 schema_metadata must contain exactly one canonical row, found {}",
            rows.len()
        );
    }
    let (
        singleton_key,
        data_generation,
        root_instance_id,
        migration_head,
        seed_manifest_digest,
        canonical_schema_manifest_digest,
        projection_schema_version,
    ) = rows.into_iter().next().expect("row count checked");
    let metadata = FreshV4SchemaMetadata {
        singleton_key,
        data_generation: u32_from_sqlite("data_generation", data_generation)?,
        root_instance_id,
        migration_head: u32_from_sqlite("migration_head", migration_head)?,
        seed_manifest_digest: DigestHex::from(seed_manifest_digest),
        canonical_schema_manifest_digest: DigestHex::from(
            canonical_schema_manifest_digest,
        ),
        projection_schema_version: u32_from_sqlite(
            "projection_schema_version",
            projection_schema_version,
        )?,
    };
    metadata
        .validate()
        .map_err(|error| anyhow::anyhow!("invalid Fresh-v4 schema metadata: {error}"))?;
    let expected_seed_digest =
        digest_payload(&official_preset_seed_manifest_payload())?;
    if metadata.data_generation != FRESH_V4_DATA_GENERATION
        || metadata.migration_head != FRESH_V4_MIGRATION_HEAD
        || metadata.projection_schema_version
            != FRESH_V4_PROJECTION_SCHEMA_VERSION
        || metadata.seed_manifest_digest != expected_seed_digest
        || metadata.canonical_schema_manifest_digest != *schema_digest
        || marker.canonical_schema_manifest_digest != *schema_digest
        || !marker.matches_schema_metadata(&metadata)
    {
        anyhow::bail!("Fresh-v4 schema_metadata does not match the ready marker");
    }

    let expected_tables = fresh_v4_schema_manifest_payload()
        .tables
        .into_iter()
        .map(|table| table.table_name)
        .collect::<BTreeSet<_>>();
    let actual_tables = sqlx::query_scalar::<_, String>(
        "SELECT name FROM sqlite_schema \
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .collect::<BTreeSet<_>>();
    if actual_tables != expected_tables {
        anyhow::bail!(
            "Fresh-v4 table exact-set mismatch: expected {expected_tables:?}, found {actual_tables:?}"
        );
    }

    let actual_objects = schema_objects(pool).await?;
    let expected_objects = baseline_schema_objects().await?;
    if actual_objects != expected_objects {
        anyhow::bail!(
            "Fresh-v4 table/index/trigger definitions do not match the embedded baseline"
        );
    }

    let foreign_keys: i64 =
        sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(pool)
            .await?;
    if foreign_keys != 1 {
        anyhow::bail!("Fresh-v4 pool did not enable SQLite foreign-key enforcement");
    }
    let user_version: i64 =
        sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(pool)
            .await?;
    if user_version != 0 {
        anyhow::bail!(
            "Fresh-v4 PRAGMA user_version must remain 0, found {user_version}"
        );
    }

    let expected_migrations = vec![(
        i64::from(FRESH_V4_MIGRATION_HEAD),
        BASELINE_MIGRATION_NAME.to_owned(),
        digest_bytes(FRESH_V4_BASELINE_SQL.as_bytes()).as_ref().to_owned(),
        0_i64,
    )];
    let migrations: Vec<(i64, String, String, i64)> = sqlx::query_as(
        "SELECT version, name, checksum, applied_at \
         FROM schema_migrations ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    if migrations != expected_migrations {
        anyhow::bail!(
            "Fresh-v4 migration lineage mismatch: {migrations:?}"
        );
    }

    let quick_check: Vec<String> =
        sqlx::query_scalar("PRAGMA quick_check")
            .fetch_all(pool)
            .await?;
    if quick_check.as_slice() != ["ok"] {
        anyhow::bail!(
            "Fresh-v4 SQLite quick_check failed: {}",
            quick_check.join("; ")
        );
    }
    let foreign_key_failures: Vec<(String, i64, String, i64)> =
        sqlx::query_as("PRAGMA foreign_key_check")
            .fetch_all(pool)
            .await?;
    if !foreign_key_failures.is_empty() {
        anyhow::bail!(
            "Fresh-v4 SQLite foreign_key_check found {} violations",
            foreign_key_failures.len()
        );
    }
    Ok(())
}

fn schema_objects(
    pool: &SqlitePool,
) -> impl std::future::Future<Output = anyhow::Result<Vec<SchemaObject>>> + '_ {
    async move {
        Ok(sqlx::query_as(
            "SELECT type, name, tbl_name, sql FROM sqlite_schema \
             WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        )
        .fetch_all(pool)
        .await?)
    }
}

async fn baseline_schema_objects() -> anyhow::Result<Vec<SchemaObject>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(
            SqliteConnectOptions::new()
                .in_memory(true)
                .foreign_keys(true),
        )
        .await?;
    let result: anyhow::Result<Vec<SchemaObject>> = async {
        sqlx::raw_sql(FRESH_V4_BASELINE_SQL)
            .execute(&pool)
            .await?;
        schema_objects(&pool).await
    }
    .await;
    pool.close().await;
    result
}

fn u32_from_sqlite(field: &str, value: i64) -> anyhow::Result<u32> {
    u32::try_from(value).map_err(|_| {
        anyhow::anyhow!(
            "Fresh-v4 schema_metadata.{field} is outside the u32 range: {value}"
        )
    })
}

#[cfg(windows)]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn current_host_surface() -> String {
    if cfg!(feature = "computer-use") {
        "desktop".to_owned()
    } else {
        "headless".to_owned()
    }
}

fn current_runtime_target() -> RuntimeTarget {
    let target = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "x86_64-pc-windows-msvc"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "x86_64-unknown-linux-gnu"
    } else {
        "unsupported-local-target"
    };
    RuntimeTarget::from(target)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use async_trait::async_trait;
    use futures_util::StreamExt;
    use nomifun_agent_contracts::{
        ActionId, AgentPresetId, AgentPresetRevision, AgentPresetRevisionPayload,
        AgentSessionId, CapabilityExposure, CapabilityId, CapabilityRef, CapabilitySelection,
        CorrelationId,
        ChatRouteCandidate, ChatRouteFeature, ChatRouteIdentity, ChatRouteProtocol,
        ChatRouteRecord, ChatRouteRecordSchema, ChatRouteTask, IdempotencyKey,
        OperationId, PluginStateEntry, PresetRevisionRef, ResourceBindingId, ResourceId,
        ResourceKind, ScopeKey, StateKey, StrictJsonValue, TypedResourceBinding, UserId,
        VersionString,
    };
    use nomifun_agent_kernel::{
        ActiveCapabilitySetSnapshot, AgentPresetCompiler, CapabilityInvocationRequest,
        CompileRequest, CompiledSnapshot, InMemoryPluginStatePersistence, KernelRegistry,
        MaterializationPolicy, PluginStatePersistence, PluginStateSnapshot,
        SessionCapabilityState, StateIdentity,
    };
    use nomifun_chat_model_broker::{
        BrokerRetryPolicy, ChatCausality, ChatCausalityGate, ChatModelError,
        ChatContentPart, ChatMessage, ChatModelErrorCode, ChatProtocol,
        ChatResponseFormat, ChatRole, ChatToolChoice, PromptCachePolicy, ProviderIdRef,
        ProductionProviderRepository as ProductionProviderRepositoryPort,
    };
    use nomifun_common::{KnowledgeBaseId, encrypt_string};
    use nomifun_db::{
        CreateProviderParams, IProviderRepository, NewProviderModel,
        NewProviderModelCapability, SqliteProviderRepository,
    };
    use super::super::chat_broker_host::{
        ChatBrokerHostComposition, ConnectionCredentialLeaseRegistry,
    };
    fn valid_ready_marker() -> FreshV4ReadyMarker {
        FreshV4ReadyMarker {
            data_generation: FRESH_V4_DATA_GENERATION,
            root_instance_id: "test-root".to_owned(),
            migration_head: FRESH_V4_MIGRATION_HEAD,
            seed_manifest_digest: digest_payload(
                &official_preset_seed_manifest_payload(),
            )
            .unwrap(),
            canonical_schema_manifest_digest:
                canonical_schema_manifest_digest().unwrap(),
            projection_schema_version: FRESH_V4_PROJECTION_SCHEMA_VERSION,
            application_build_digest:
                application_build_digest(APPLICATION_BUILD_IDENTITY).unwrap(),
        }
    }

    fn principal() -> nomifun_agent_contracts::PrincipalRef {
        nomifun_agent_contracts::PrincipalRef {
            principal_kind: "user".to_owned(),
            principal_id: "wave1-memory-owner".to_owned(),
        }
    }

    struct KnowledgeKernelFixture {
        registry: Arc<KernelRegistry>,
    }

    impl KnowledgeKernelFixture {
        fn new() -> Self {
            let registry = Arc::new(
                KernelRegistry::new(
                    MaterializationPolicy::stable(CONTRACT_VERSION),
                    Arc::new(InMemoryPluginStatePersistence::new()),
                )
                .expect("kernel registry"),
            );
            registry
                .replace_all(
                    nomifun_agent_domain_wave1::registrations_with_host_port(
                        Arc::new(Wave1ApplicationHost::default()),
                    )
                    .expect("Wave 1 registrations"),
                )
                .expect("publish Wave 1 registrations");
            Self { registry }
        }

        fn compile_snapshot(
            &self,
            binding: TypedResourceBinding,
        ) -> (
            Arc<CompiledSnapshot>,
            ActiveCapabilitySetSnapshot,
            TypedResourceBinding,
        ) {
            compile_wave1_snapshot_for_registry(
                &self.registry,
                &[
                    nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
                    nomifun_agent_domain_wave1::KNOWLEDGE_READ,
                ],
                binding,
                "knowledge",
            )
        }

        async fn invoke(
            &self,
            snapshot: &CompiledSnapshot,
            active: &ActiveCapabilitySetSnapshot,
            binding: &TypedResourceBinding,
            capability_id: &str,
            input: serde_json::Value,
            request_id: &str,
        ) -> Result<
            StrictJsonValue,
            nomifun_agent_kernel::KernelError,
        > {
            self.registry
                .invoke(
                    snapshot,
                    active,
                    knowledge_invocation(
                        snapshot,
                        active,
                        binding,
                        capability_id,
                        input,
                        request_id,
                    ),
                )
                .await
        }
    }

    fn knowledge_binding(
        knowledge_base_id: &KnowledgeBaseId,
        root: &Path,
    ) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from("knowledge-primary"),
            resource_kind: ResourceKind::from(
                nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND,
            ),
            resource_id: ResourceId::from(knowledge_base_id.as_str()),
            owner_id: principal().principal_id,
            operations: BTreeSet::from([
                "read".to_owned(),
                "search".to_owned(),
            ]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::from([
                (
                    KNOWLEDGE_ROOT_PARAMETER.to_owned(),
                    root.to_string_lossy().into_owned(),
                ),
                (
                    KNOWLEDGE_NAME_PARAMETER.to_owned(),
                    "Release runbooks".to_owned(),
                ),
            ]),
        }
    }

    fn knowledge_invocation(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        binding: &TypedResourceBinding,
        capability_id: &str,
        input: serde_json::Value,
        request_id: &str,
    ) -> CapabilityInvocationRequest {
        let owner = principal();
        CapabilityInvocationRequest {
            principal: owner.clone(),
            session_owner: owner,
            agent_session_id: AgentSessionId::from(
                "wave1-knowledge-session",
            ),
            operation_id: OperationId::from(format!(
                "wave1-knowledge-operation-{request_id}"
            )),
            idempotency_key: IdempotencyKey::from(format!(
                "wave1-knowledge-key-{request_id}"
            )),
            correlation_id: CorrelationId::from(format!(
                "wave1-knowledge-correlation-{request_id}"
            )),
            resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
            active_set_generation: active.generation,
            capability_id: CapabilityId::from(capability_id),
            action_id: nomifun_agent_domain_wave1::action_id(capability_id)
                .expect("Knowledge capability action"),
            resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
            state_scope_key: ScopeKey::from(
                "session:wave1-knowledge-session",
            ),
            input: StrictJsonValue(input),
        }
    }

    fn compile_wave1_snapshot_for_registry(
        registry: &KernelRegistry,
        capability_ids: &[&str],
        binding: TypedResourceBinding,
        fixture_name: &str,
    ) -> (
        Arc<CompiledSnapshot>,
        ActiveCapabilitySetSnapshot,
        TypedResourceBinding,
    ) {
        let owner = principal();
        let initial_capabilities = capability_ids
            .iter()
            .copied()
            .map(|capability_id| CapabilitySelection {
                capability: CapabilityRef {
                    id: capability_id.into(),
                    version: VersionString::from(CONTRACT_VERSION),
                },
                required: true,
                exposure: CapabilityExposure::Advertised,
                action_allowlist: BTreeSet::from([
                    nomifun_agent_domain_wave1::action_id(capability_id)
                        .expect("Wave 1 capability has an action"),
                ]),
                resource_binding_refs: vec![binding.binding_id.clone()],
                destination_constraints: BTreeSet::new(),
                context_budget_override: None,
                tool_budget_override: None,
                config: StrictJsonValue(serde_json::json!({})),
            })
            .collect();
        let payload = AgentPresetRevisionPayload {
            schema_version: VersionString::from(CONTRACT_VERSION),
            surfaces: BTreeSet::from(["desktop".to_owned()]),
            model_route_refs: BTreeMap::new(),
            chat_route_records: BTreeMap::new(),
            initial_capabilities,
            on_demand_capabilities: Vec::new(),
            skill_bindings: Vec::new(),
            resource_bindings: vec![binding.clone()],
            system_role_provider_overrides: BTreeMap::new(),
            persona: format!("Wave 1 {fixture_name} test"),
            instructions: format!("Exercise the Wave 1 {fixture_name} owner."),
            context_policy: StrictJsonValue(serde_json::json!({})),
            execution_constraints: StrictJsonValue(serde_json::json!({})),
            runtime_budget: StrictJsonValue(serde_json::json!({})),
        };
        let revision = AgentPresetRevision {
            reference: PresetRevisionRef {
                preset_id: AgentPresetId::from(format!("wave1-{fixture_name}")),
                revision: 1,
                revision_digest: digest_payload(&payload)
                    .expect("revision digest"),
            },
            payload,
            created_by: UserId::from(owner.principal_id.clone()),
            created_at_ms: 1,
            reason: None,
        };
        let snapshot = AgentPresetCompiler::compile(
            &registry.snapshot().expect("registry snapshot"),
            &CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(
                    CONTRACT_VERSION,
                ),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: DigestHex::from("runtime"),
                available_runtime_features: BTreeSet::new(),
                installation_role_bindings: BTreeMap::new(),
                canonical_schema_manifest_digest: DigestHex::from("schema"),
                target_contribution_manifest_digest: DigestHex::from("target"),
                host_target: RuntimeTarget::from("test-target"),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision:
                    format!("wave1-{fixture_name}-test"),
            },
            CompileRequest {
                revision,
                principal: owner,
                scene: format!("wave1-{fixture_name}-test"),
                surface: "desktop".to_owned(),
                audience: "test".to_owned(),
                created_at_ms: 2,
                resolver_run_id: OperationId::from(format!(
                    "wave1-{fixture_name}-resolve"
                )),
            },
        )
        .expect("compile Wave 1 capabilities");
        let active = SessionCapabilityState::new(&snapshot)
            .snapshot()
            .expect("initial active set");
        (Arc::new(snapshot), active, binding)
    }

    struct MemoryKernelFixture {
        registry: Arc<KernelRegistry>,
        persistence: Arc<InMemoryPluginStatePersistence>,
    }

    impl MemoryKernelFixture {
        fn new() -> Self {
            let persistence = Arc::new(InMemoryPluginStatePersistence::new());
            Self::with_persistence(persistence)
        }

        fn with_persistence(
            persistence: Arc<InMemoryPluginStatePersistence>,
        ) -> Self {
            let registry = Arc::new(
                KernelRegistry::new(
                    MaterializationPolicy::stable(CONTRACT_VERSION),
                    Arc::clone(&persistence) as Arc<dyn PluginStatePersistence>,
                )
                .expect("kernel registry"),
            );
            registry
                .replace_all(
                    nomifun_agent_domain_wave1::registrations_with_host_port(
                        Arc::new(Wave1ApplicationHost::default()),
                    )
                    .expect("Wave 1 registrations"),
                )
                .expect("publish Wave 1 registrations");
            Self {
                registry,
                persistence,
            }
        }

        fn compile_memory_snapshot(
            &self,
            capability_id: &str,
            resource_id: &str,
        ) -> (
            Arc<CompiledSnapshot>,
            ActiveCapabilitySetSnapshot,
            TypedResourceBinding,
        ) {
            compile_memory_snapshot_for_registry(
                &self.registry,
                capability_id,
                resource_id,
            )
        }
    }

    fn compile_memory_snapshot_for_registry(
        registry: &KernelRegistry,
        capability_id: &str,
        resource_id: &str,
    ) -> (
        Arc<CompiledSnapshot>,
        ActiveCapabilitySetSnapshot,
        TypedResourceBinding,
    ) {
        let binding = TypedResourceBinding {
            binding_id: ResourceBindingId::from(format!(
                "binding-{}",
                resource_id
            )),
            resource_kind: ResourceKind::from(
                if capability_id.starts_with("memory.project.") {
                    nomifun_agent_domain_wave1::PROJECT_MEMORY_RESOURCE_KIND
                } else {
                    nomifun_agent_domain_wave1::COMPANION_MEMORY_RESOURCE_KIND
                },
            ),
            resource_id: ResourceId::from(resource_id),
            owner_id: principal().principal_id,
            operations: BTreeSet::from(["read".to_owned(), "write".to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        };
        let fixture_name =
            format!("memory-{}", capability_id.replace('.', "-"));
        compile_wave1_snapshot_for_registry(
            registry,
            &[capability_id],
            binding,
            &fixture_name,
        )
    }

    fn memory_invocation(
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        binding: &TypedResourceBinding,
        capability_id: &str,
        idempotency_key: &str,
        content: &str,
        operation_id: &str,
    ) -> CapabilityInvocationRequest {
        let owner = principal();
        CapabilityInvocationRequest {
            principal: owner.clone(),
            session_owner: owner,
            agent_session_id: AgentSessionId::from("wave1-memory-session"),
            operation_id: OperationId::from(operation_id),
            idempotency_key: IdempotencyKey::from(idempotency_key),
            correlation_id: CorrelationId::from(format!("correlation-{idempotency_key}")),
            resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
            active_set_generation: active.generation,
            capability_id: CapabilityId::from(capability_id),
            action_id: nomifun_agent_domain_wave1::action_id(capability_id)
                .expect("memory capability action"),
            resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
            state_scope_key: ScopeKey::from("session:wave1-memory-session"),
            input: StrictJsonValue(serde_json::json!({
                "content": content
            })),
        }
    }

    fn malformed_memory_persistence() -> Arc<InMemoryPluginStatePersistence> {
        memory_persistence_with_state(
            serde_json::json!({
                "entries": "not-an-array"
            }),
            MEMORY_STATE_FORMAT_VERSION,
            "project-corrupt",
        )
    }

    fn memory_persistence_with_state(
        value: serde_json::Value,
        state_format_version: &str,
        resource_scope: &str,
    ) -> Arc<InMemoryPluginStatePersistence> {
        let identity = StateIdentity {
            package_id: nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
            mount_id: nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
            scope_key: ScopeKey::from(format!("resource:{resource_scope}")),
            state_key: StateKey::from(MEMORY_STATE_KEY),
        };
        let entry = PluginStateEntry {
            namespace: identity.namespace(),
            revision: 1,
            state_format_version: VersionString::from(state_format_version),
            writer_package_version: VersionString::from(CONTRACT_VERSION),
            value: StrictJsonValue(value),
        };
        let snapshot = PluginStateSnapshot::from_parts(
            BTreeMap::from([(identity.clone(), entry)]),
            BTreeMap::from([(identity, 1)]),
        )
        .expect("malformed fixture namespace");
        Arc::new(InMemoryPluginStatePersistence::reopen(snapshot))
    }

    #[test]
    fn host_target_is_a_concrete_platform_label() {
        assert!(!current_runtime_target().as_ref().is_empty());
        assert!(!current_host_surface().is_empty());
    }

    #[test]
    fn feature_inventory_is_frozen_and_non_empty() {
        let inventory: CodingRuntimeFeatureInventoryPayload =
            serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON).unwrap();
        inventory.validate().unwrap();
        assert!(!inventory.runtime_features.is_empty());
        assert_eq!(inventory.supported_profiles.len(), 2);
    }

    #[test]
    fn bundled_registration_inventory_is_complete_and_unique() {
        let registrations =
            bundled_registrations(
                nomifun_agent_domain_wave1::unconfigured_host_port(),
                Arc::new(Wave2ApplicationHost::new()),
            )
                .unwrap();
        let target_specs = nomifun_agent_domain_support::c7_package_specs();
        validate_bundled_registrations(&registrations, &target_specs)
            .unwrap();
        let expected_packages = target_specs
            .iter()
            .map(|spec| spec.id)
            .collect::<BTreeSet<_>>();
        let actual_packages = registrations
            .iter()
            .map(|registration| {
                registration
                    .metadata
                    .manifest
                    .payload
                    .package_id
                    .as_ref()
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(actual_packages, expected_packages);
        assert_eq!(
            registrations
                .iter()
                .filter(|registration| {
                    registration
                        .metadata
                        .manifest
                        .payload
                        .package_id
                        .as_ref()
                        == MODEL_MEDIA_PACKAGE_ID
                })
                .count(),
            1
        );
        assert_eq!(registrations.len(), target_specs.len());
        assert_eq!(
            registrations
                .iter()
                .map(|registration| {
                    registration
                        .metadata
                        .manifest
                        .payload
                        .contributions
                        .capabilities
                        .len()
                })
                .sum::<usize>(),
            target_specs
                .iter()
                .map(|spec| spec.capabilities.len())
                .sum::<usize>()
        );
    }

    #[cfg(feature = "browser-use")]
    #[tokio::test]
    #[ignore = "requires a local system Chrome; run explicitly with --ignored"]
    async fn browser_role_owner_runs_the_canonical_observe_navigate_act_render_chain() {
        tokio::time::timeout(Duration::from_secs(90), async {
            let directory = tempfile::tempdir().expect("browser role test root");
            let data_dir = directory.path().join("data");
            let bootstrap = nomifun_v4_root::FreshV4Coordinator::default()
                .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
                .await
                .expect("fresh v4 browser role root");
            let pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
                .await
                .expect("browser role v4 pool");
            let encryption_key = [0x42; 32];
            let browser_hub = build_browser_session_hub(
                &data_dir,
                &data_dir,
                encryption_key,
            )
            .await
            .expect("browser hub construction")
            .expect("system Chrome must be available for this live role test");
            let host_ports = AgentDomainHostPorts::for_workspace_root(
                data_dir.clone(),
                pool.clone(),
            )
            .with_browser_hub(browser_hub.clone());
            let platform = initialize_platform_with_cleanup_and_host_ports(
                pool,
                data_dir.join(FRESH_V4_READY_MARKER_FILE),
                bootstrap.ready_marker,
                canonical_schema_manifest_digest().expect("browser role schema digest"),
                None,
                encryption_key,
                host_ports,
            )
            .await
            .expect("canonical browser role platform");

            let owner = nomifun_agent_contracts::PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "browser-role-owner".to_owned(),
            };
            let binding = TypedResourceBinding {
                binding_id: ResourceBindingId::from("browser-role-binding"),
                resource_kind: ResourceKind::from("browser"),
                resource_id: ResourceId::from("browser-role-target"),
                owner_id: owner.principal_id.clone(),
                operations: BTreeSet::from([
                    "observe".to_owned(),
                    "navigate".to_owned(),
                    "interact".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            };
            let materialized = platform
                .materialized_registry()
                .expect("browser role materialized registry");
            let role_id = nomifun_agent_contracts::ExecutionRoleId::from(
                nomifun_agent_domain_wave2::BROWSER_EXECUTION_ROLE_ID,
            );
            let provider_mount =
                nomifun_agent_contracts::PluginMountId::from(
                    nomifun_agent_domain_wave2::BROWSER_MOUNT_ID,
                );
            let provider = materialized
                .role_provider(&role_id, &provider_mount)
                .expect("bundled Browser provider");
            let installation_binding =
                nomifun_agent_contracts::InstallationRoleBinding {
                    selection: nomifun_agent_contracts::RoleProviderSelection {
                        role: provider.provider.role.clone(),
                        provider_mount_id: provider_mount,
                    },
                    binding_version: 1,
                    updated_at_ms: 1,
                };
            let inventory: CodingRuntimeFeatureInventoryPayload =
                serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON)
                    .expect("runtime feature inventory");
            let environment = CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: digest_payload(&inventory)
                    .expect("runtime inventory digest"),
                available_runtime_features: inventory.runtime_features.clone(),
                installation_role_bindings: BTreeMap::from([(role_id.clone(), installation_binding)]),
                canonical_schema_manifest_digest: canonical_schema_manifest_digest()
                    .expect("browser role schema digest"),
                target_contribution_manifest_digest: official_preset_seed_manifest_payload()
                    .target_first_party_contribution_digest,
                host_target: current_runtime_target(),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "browser-role-live-2026-09-03".to_owned(),
            };
            let capability = |id: &str, required: bool, exposure: CapabilityExposure| {
                let action_allowlist = matches!(
                    id,
                    "browser.navigate" | "browser.act" | "browser.render_content"
                )
                .then(|| BTreeSet::from([ActionId::from(format!("{id}.invoke"))]))
                .unwrap_or_default();
                CapabilitySelection {
                    capability: CapabilityRef {
                        id: CapabilityId::from(id),
                        version: VersionString::from(CONTRACT_VERSION),
                    },
                    required,
                    exposure,
                    action_allowlist,
                    resource_binding_refs: vec![binding.binding_id.clone()],
                    destination_constraints: BTreeSet::new(),
                    context_budget_override: None,
                    tool_budget_override: None,
                    config: StrictJsonValue(serde_json::json!({})),
                }
            };
            let payload = AgentPresetRevisionPayload {
                schema_version: VersionString::from(CONTRACT_VERSION),
                surfaces: BTreeSet::from(["desktop".to_owned()]),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
                initial_capabilities: vec![
                    capability("browser.identity", false, CapabilityExposure::Hidden),
                    capability("browser.observe", true, CapabilityExposure::Hidden),
                    capability("browser.navigate", true, CapabilityExposure::Advertised),
                    capability("browser.act", true, CapabilityExposure::Advertised),
                    capability("browser.render_content", false, CapabilityExposure::Hidden),
                ],
                on_demand_capabilities: Vec::new(),
                skill_bindings: Vec::new(),
                resource_bindings: vec![binding.clone()],
                system_role_provider_overrides: BTreeMap::new(),
                persona: "Browser role live test".to_owned(),
                instructions: "Exercise the canonical Browser role owner.".to_owned(),
                context_policy: StrictJsonValue(serde_json::json!({})),
                execution_constraints: StrictJsonValue(serde_json::json!({})),
                runtime_budget: StrictJsonValue(serde_json::json!({})),
            };
            let revision = AgentPresetRevision {
                reference: PresetRevisionRef {
                    preset_id: AgentPresetId::from("browser-role-live-test"),
                    revision: 1,
                    revision_digest: digest_payload(&payload).expect("browser role revision digest"),
                },
                payload,
                created_by: UserId::from(owner.principal_id.clone()),
                created_at_ms: 1,
                reason: None,
            };
            let snapshot = AgentPresetCompiler::compile(
                &materialized,
                &environment,
                CompileRequest {
                    revision,
                    principal: owner.clone(),
                    scene: "browser-role-live-test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("browser-role-live-resolve"),
                },
            )
            .expect("compile Browser role snapshot");
            let snapshot = Arc::new(snapshot);
            let active = SessionCapabilityState::new(&snapshot)
                .snapshot()
                .expect("Browser role active set");
            let session_id = AgentSessionId::from(uuid::Uuid::now_v7().to_string());
            let scope_key = ScopeKey::from(format!("session:{}", session_id.as_ref()));
            let role_request = |capability_id: &str, suffix: &str| {
                nomifun_agent_kernel::RoleMemberInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner.clone(),
                    operation_id: OperationId::from(format!(
                        "browser-role-{capability_id}-{suffix}"
                    )),
                    correlation_id: CorrelationId::from(format!(
                        "browser-role-correlation-{capability_id}-{suffix}"
                    )),
                    capability_id: CapabilityId::from(capability_id),
                    resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
                    state_scope_key: scope_key.clone(),
                    admission: nomifun_agent_kernel::RoleMemberAdmission::Agent {
                        agent_session_id: session_id.clone(),
                        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                        active_set_generation: active.generation,
                    },
                }
            };

            platform
                .kernel_registry()
                .acquire_role_resource(
                    &snapshot,
                    &active,
                    role_request("browser.identity", "acquire"),
                )
                .await
                .expect("Browser identity resource acquisition");
            let observed = platform
                .kernel_registry()
                .contribute_role_context(
                    &snapshot,
                    &active,
                    role_request("browser.observe", "before"),
                )
                .await
                .expect("Browser observe context contribution")
                .value
                .expect("Browser observe must return a context value")
                .0;
            assert!(observed["ref_generation"].as_u64().unwrap_or_default() > 0);

            let action_request = |capability_id: &str, suffix: &str, input: serde_json::Value| {
                CapabilityInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner.clone(),
                    agent_session_id: session_id.clone(),
                    operation_id: OperationId::from(format!(
                        "browser-role-action-{capability_id}-{suffix}"
                    )),
                    idempotency_key: IdempotencyKey::from(format!(
                        "browser-role-key-{capability_id}-{suffix}"
                    )),
                    correlation_id: CorrelationId::from(format!(
                        "browser-role-action-correlation-{capability_id}-{suffix}"
                    )),
                    resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                    active_set_generation: active.generation,
                    capability_id: CapabilityId::from(capability_id),
                    action_id: ActionId::from(format!("{capability_id}.invoke")),
                    resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
                    state_scope_key: scope_key.clone(),
                    input: StrictJsonValue(input),
                }
            };
            let data_url = "data:text/html,<html><body><button id='toggle'>Toggle</button><p id='state'>before</p><script>document.getElementById('toggle').onclick=()=>document.getElementById('state').textContent='after'</script></body></html>";
            platform
                .kernel_registry()
                .invoke(
                    &snapshot,
                    &active,
                    action_request(
                        "browser.navigate",
                        "navigate",
                        serde_json::json!({ "url": data_url, "new_tab": false }),
                    ),
                )
                .await
                .expect("Browser navigate action");
            let after_navigate = platform
                .kernel_registry()
                .contribute_role_context(
                    &snapshot,
                    &active,
                    role_request("browser.observe", "after-navigate"),
                )
                .await
                .expect("Browser observe after navigation")
                .value
                .expect("Browser observe after navigation must return a value")
                .0;
            assert!(after_navigate["output"].is_object() || after_navigate["output"].is_string());
            platform
                .kernel_registry()
                .invoke(
                    &snapshot,
                    &active,
                    action_request(
                        "browser.act",
                        "wait",
                        serde_json::json!({ "action": "wait", "ms": 0 }),
                    ),
                )
                .await
                .expect("Browser act action");
            let rendered = platform
                .kernel_registry()
                .invoke(
                    &snapshot,
                    &active,
                    action_request(
                        "browser.render_content",
                        "render",
                        serde_json::json!({ "url": data_url }),
                    ),
                )
                .await
                .expect("Browser render_content action");
            assert!(rendered.0["html"].as_str().is_some());
            assert_eq!(rendered.0["html_truncated"], false);

            platform
                .kernel_registry()
                .release_role_resources(&scope_key)
                .await
                .expect("Browser role resource release");
            browser_hub.close_all().await.expect("Browser Hub close");
            platform.shutdown().await.expect("Browser role platform shutdown");
            platform.pool().close().await;
        })
        .await
        .expect("Browser Role live chain exceeded its 90 second deadline");
    }

    #[cfg(feature = "computer-use")]
    #[tokio::test]
    #[ignore = "requires a local desktop and UI Automation; run explicitly with --ignored"]
    async fn computer_role_owner_runs_the_canonical_observe_input_chain() {
        tokio::time::timeout(Duration::from_secs(90), async {
            let directory = tempfile::tempdir().expect("computer role test root");
            let data_dir = directory.path().join("data");
            let bootstrap = nomifun_v4_root::FreshV4Coordinator::default()
                .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
                .await
                .expect("fresh v4 computer role root");
            let pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
                .await
                .expect("computer role v4 pool");
            let host_ports = AgentDomainHostPorts::for_workspace_root(
                data_dir.clone(),
                pool.clone(),
            );
            let platform = initialize_platform_with_cleanup_and_host_ports(
                pool,
                data_dir.join(FRESH_V4_READY_MARKER_FILE),
                bootstrap.ready_marker,
                canonical_schema_manifest_digest().expect("computer role schema digest"),
                None,
                [0x43; 32],
                host_ports,
            )
            .await
            .expect("canonical computer role platform");

            let owner = nomifun_agent_contracts::PrincipalRef {
                principal_kind: "user".to_owned(),
                principal_id: "computer-role-owner".to_owned(),
            };
            let binding = TypedResourceBinding {
                binding_id: ResourceBindingId::from("computer-role-binding"),
                resource_kind: ResourceKind::from("computer"),
                resource_id: ResourceId::from("computer-role-target"),
                owner_id: owner.principal_id.clone(),
                operations: BTreeSet::from([
                    "observe".to_owned(),
                    "input".to_owned(),
                    "launch".to_owned(),
                ]),
                connection_config_ref: None,
                typed_parameters: BTreeMap::new(),
            };
            let materialized = platform
                .materialized_registry()
                .expect("computer role materialized registry");
            let role_id = nomifun_agent_contracts::ExecutionRoleId::from(
                nomifun_agent_domain_wave2::COMPUTER_EXECUTION_ROLE_ID,
            );
            let provider_mount =
                nomifun_agent_contracts::PluginMountId::from(
                    nomifun_agent_domain_wave2::COMPUTER_A11Y_MOUNT_ID,
                );
            let provider = materialized
                .role_provider(&role_id, &provider_mount)
                .expect("bundled Computer provider");
            let installation_binding =
                nomifun_agent_contracts::InstallationRoleBinding {
                    selection: nomifun_agent_contracts::RoleProviderSelection {
                        role: provider.provider.role.clone(),
                        provider_mount_id: provider_mount,
                    },
                    binding_version: 1,
                    updated_at_ms: 1,
                };
            let inventory: CodingRuntimeFeatureInventoryPayload =
                serde_json::from_str(RUNTIME_FEATURE_INVENTORY_JSON)
                    .expect("runtime feature inventory");
            let environment = CompilerEnvironment {
                resolver_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_protocol_version: VersionString::from(CONTRACT_VERSION),
                required_runtime_profile: RuntimeProfileKind::ManagedMinimal,
                runtime_feature_inventory_digest: digest_payload(&inventory)
                    .expect("runtime inventory digest"),
                available_runtime_features: inventory.runtime_features.clone(),
                installation_role_bindings: BTreeMap::from([(role_id.clone(), installation_binding)]),
                canonical_schema_manifest_digest: canonical_schema_manifest_digest()
                    .expect("computer role schema digest"),
                target_contribution_manifest_digest: official_preset_seed_manifest_payload()
                    .target_first_party_contribution_digest,
                host_target: current_runtime_target(),
                host_surface: "desktop".to_owned(),
                availability_evidence_revision: "computer-role-live-2026-09-03".to_owned(),
            };
            let capability = |id: &str, required: bool, exposure: CapabilityExposure| {
                let action_allowlist = (id == "computer.input")
                    .then(|| BTreeSet::from([ActionId::from("computer.input.invoke")]))
                    .unwrap_or_default();
                CapabilitySelection {
                    capability: CapabilityRef {
                        id: CapabilityId::from(id),
                        version: VersionString::from(CONTRACT_VERSION),
                    },
                    required,
                    exposure,
                    action_allowlist,
                    resource_binding_refs: vec![binding.binding_id.clone()],
                    destination_constraints: BTreeSet::new(),
                    context_budget_override: None,
                    tool_budget_override: None,
                    config: StrictJsonValue(serde_json::json!({})),
                }
            };
            let payload = AgentPresetRevisionPayload {
                schema_version: VersionString::from(CONTRACT_VERSION),
                surfaces: BTreeSet::from(["desktop".to_owned()]),
                model_route_refs: BTreeMap::new(),
                chat_route_records: BTreeMap::new(),
                initial_capabilities: vec![
                    capability("computer.observe", true, CapabilityExposure::Hidden),
                    capability("computer.input", true, CapabilityExposure::Advertised),
                ],
                on_demand_capabilities: Vec::new(),
                skill_bindings: Vec::new(),
                resource_bindings: vec![binding.clone()],
                system_role_provider_overrides: BTreeMap::new(),
                persona: "Computer role live test".to_owned(),
                instructions: "Exercise the canonical Computer role owner.".to_owned(),
                context_policy: StrictJsonValue(serde_json::json!({})),
                execution_constraints: StrictJsonValue(serde_json::json!({})),
                runtime_budget: StrictJsonValue(serde_json::json!({})),
            };
            let revision = AgentPresetRevision {
                reference: PresetRevisionRef {
                    preset_id: AgentPresetId::from("computer-role-live-test"),
                    revision: 1,
                    revision_digest: digest_payload(&payload)
                        .expect("computer role revision digest"),
                },
                payload,
                created_by: UserId::from(owner.principal_id.clone()),
                created_at_ms: 1,
                reason: None,
            };
            let snapshot = AgentPresetCompiler::compile(
                &materialized,
                &environment,
                CompileRequest {
                    revision,
                    principal: owner.clone(),
                    scene: "computer-role-live-test".to_owned(),
                    surface: "desktop".to_owned(),
                    audience: "test".to_owned(),
                    created_at_ms: 2,
                    resolver_run_id: OperationId::from("computer-role-live-resolve"),
                },
            )
            .expect("compile Computer role snapshot");
            let snapshot = Arc::new(snapshot);
            let active = SessionCapabilityState::new(&snapshot)
                .snapshot()
                .expect("Computer role active set");
            let session_id = AgentSessionId::from(uuid::Uuid::now_v7().to_string());
            let scope_key = ScopeKey::from(format!("session:{}", session_id.as_ref()));
            let role_request = |capability_id: &str, suffix: &str| {
                nomifun_agent_kernel::RoleMemberInvocationRequest {
                    principal: owner.clone(),
                    session_owner: owner.clone(),
                    operation_id: OperationId::from(format!(
                        "computer-role-{capability_id}-{suffix}"
                    )),
                    correlation_id: CorrelationId::from(format!(
                        "computer-role-correlation-{capability_id}-{suffix}"
                    )),
                    capability_id: CapabilityId::from(capability_id),
                    resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
                    state_scope_key: scope_key.clone(),
                    admission: nomifun_agent_kernel::RoleMemberAdmission::Agent {
                        agent_session_id: session_id.clone(),
                        resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                        active_set_generation: active.generation,
                    },
                }
            };

            let observed = platform
                .kernel_registry()
                .contribute_role_context(
                    &snapshot,
                    &active,
                    role_request("computer.observe", "before"),
                )
                .await
                .expect("Computer observe context contribution")
                .value
                .expect("Computer observe must return a context value")
                .0;
            let first_generation = observed["generation"]
                .as_u64()
                .expect("Computer observe generation");
            assert!(first_generation > 0);

            let input = CapabilityInvocationRequest {
                principal: owner.clone(),
                session_owner: owner.clone(),
                agent_session_id: session_id.clone(),
                operation_id: OperationId::from("computer-role-action-computer.input-wait"),
                idempotency_key: IdempotencyKey::from("computer-role-key-input-wait"),
                correlation_id: CorrelationId::from(
                    "computer-role-action-correlation-input-wait",
                ),
                resolved_snapshot_ref: snapshot.snapshot_ref().clone(),
                active_set_generation: active.generation,
                capability_id: CapabilityId::from("computer.input"),
                action_id: ActionId::from("computer.input.invoke"),
                resource_binding_ids: BTreeSet::from([binding.binding_id.clone()]),
                state_scope_key: scope_key.clone(),
                input: StrictJsonValue(serde_json::json!({
                    "action": "wait",
                    "seconds": 0,
                    "expected_generation": first_generation
                })),
            };
            platform
                .kernel_registry()
                .invoke(&snapshot, &active, input)
                .await
                .expect("Computer input action");

            let after_input = platform
                .kernel_registry()
                .contribute_role_context(
                    &snapshot,
                    &active,
                    role_request("computer.observe", "after-input"),
                )
                .await
                .expect("Computer observe after input")
                .value
                .expect("Computer observe after input must return a value")
                .0;
            assert!(
                after_input["generation"]
                    .as_u64()
                    .is_some_and(|generation| generation > first_generation)
            );

            platform
                .kernel_registry()
                .release_role_resources(&scope_key)
                .await
                .expect("Computer role resource release");
            platform
                .shutdown()
                .await
                .expect("Computer role platform shutdown");
            platform.pool().close().await;
        })
        .await
        .expect("Computer Role live chain exceeded its 90 second deadline");
    }

    #[test]
    fn ready_marker_requires_canonical_bytes_and_current_build() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FRESH_V4_READY_MARKER_FILE);
        let marker = valid_ready_marker();
        let mut bytes = canonical_json_bytes(&marker).unwrap();
        bytes.push(b'\n');
        std::fs::write(&path, bytes).unwrap();
        assert!(read_ready_marker(&path).is_err());

        std::fs::write(&path, canonical_json_bytes(&marker).unwrap()).unwrap();
        let read = read_ready_marker(&path).unwrap();
        validate_ready_marker(
            &read,
            &read.canonical_schema_manifest_digest,
        )
        .unwrap();

        let mut wrong_build = read;
        wrong_build.application_build_digest = DigestHex::from("0".repeat(64));
        assert!(validate_ready_marker(
            &wrong_build,
            &wrong_build.canonical_schema_manifest_digest
        )
        .is_err());
    }

    #[tokio::test]
    async fn initialization_failure_closes_the_owned_pool() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        let observer = pool.clone();
        let result = initialize_platform_with_cleanup(
            pool.clone(),
            PathBuf::from("missing-ready-marker"),
            valid_ready_marker(),
            canonical_schema_manifest_digest().unwrap(),
            Some(pool),
            [0; 32],
            nomifun_agent_domain_wave1::unconfigured_host_port(),
            Arc::new(Wave2ApplicationHost::new()),
        )
        .await;
        assert!(result.is_err());
        assert!(observer.is_closed());
    }

    #[tokio::test]
    async fn canonical_fresh_v4_root_passes_mount_validation() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .unwrap();
        let pool = open_validated_pool(
            &data_dir.join(FRESH_V4_DATABASE_FILE),
        )
        .await
        .unwrap();
        validate_schema_metadata(
            &pool,
            &outcome.ready_marker,
            &outcome
                .ready_marker
                .canonical_schema_manifest_digest,
        )
        .await
        .unwrap();
        pool.close().await;
    }

    #[tokio::test]
    async fn canonical_mount_publishes_one_complete_registration_generation() {
        let directory = tempfile::tempdir().unwrap();
        let data_dir = directory.path().join("data");
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .unwrap();
        let pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
            .await
            .unwrap();
        let platform = initialize_platform_with_cleanup(
            pool.clone(),
            data_dir.join(FRESH_V4_READY_MARKER_FILE),
            outcome.ready_marker,
            canonical_schema_manifest_digest().unwrap(),
            Some(pool),
            [0; 32],
            nomifun_agent_domain_wave1::unconfigured_host_port(),
            Arc::new(Wave2ApplicationHost::new()),
        )
        .await
        .unwrap();

        let registry = platform.materialized_registry().unwrap();
        assert_eq!(registry.generation, 1);
        let target_specs = nomifun_agent_domain_support::c7_package_specs();
        let expected_product_packages = target_specs
            .iter()
            .map(|spec| spec.id.to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            registry.packages.len(),
            expected_product_packages.len() + 1
        );
        assert!(registry.packages.contains_key(
            &nomifun_agent_contracts::PackageId::from(
                nomifun_agent_contracts::AGENT_CORE_PACKAGE_ID,
            ),
        ));
        assert_eq!(
            registry
                .packages
                .keys()
                .filter(|package_id| {
                    package_id.as_ref()
                        != nomifun_agent_contracts::AGENT_CORE_PACKAGE_ID
                })
                .map(|package_id| package_id.as_ref().to_owned())
                .collect::<BTreeSet<_>>(),
            expected_product_packages
        );
        let expected_capabilities = target_specs
            .iter()
            .flat_map(|spec| spec.capabilities.iter())
            .map(|capability| CapabilityId::from(capability.id))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            registry.capabilities.keys().cloned().collect::<BTreeSet<_>>(),
            expected_capabilities
        );
        let package_rows: Vec<String> = sqlx::query_scalar(
            "SELECT package_id FROM plugin_packages \
             WHERE package_version = ? ORDER BY package_id",
        )
        .bind(CONTRACT_VERSION)
        .fetch_all(platform.pool())
        .await
        .unwrap();
        assert_eq!(package_rows.len(), expected_product_packages.len() + 1);
        assert!(package_rows
            .iter()
            .any(|package_id| package_id == nomifun_agent_contracts::AGENT_CORE_PACKAGE_ID));
        assert_eq!(
            package_rows.iter().collect::<BTreeSet<_>>().len(),
            package_rows.len()
        );
        platform.pool().close().await;
    }

    struct AllowChatGate;

    #[async_trait]
    impl ChatCausalityGate for AllowChatGate {
        async fn authorize(&self, _causality: &ChatCausality) -> Result<(), ChatModelError> {
            Ok(())
        }
    }

    async fn production_chat_fixture(
        status: u16,
    ) -> (
        nomifun_chat_model_broker::ChatModelStream,
        wiremock::MockServer,
    ) {
        let server = wiremock::MockServer::start().await;
        let body = if status == 200 {
            "event: response.created\ndata: {\"id\":\"host-response\"}\n\n\
             event: text.delta\ndata: {\"text\":\"host success\"}\n\n\
             event: usage\ndata: {\"input_tokens\":1,\"output_tokens\":2}\n\n\
             event: response.completed\ndata: {\"finish_reason\":\"stop\"}\n\n"
        } else {
            "{\"error\":{\"message\":\"provider unavailable\"}}"
        };
        let mut response = wiremock::ResponseTemplate::new(status);
        response = if status == 200 {
            response
                .set_body_raw(body, "text/event-stream")
                .insert_header("cache-control", "no-cache")
        } else {
            response.set_body_raw(body, "application/json")
        };
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat"))
            .respond_with(response)
            .mount(&server)
            .await;

        let v4_dir = tempfile::tempdir().expect("v4 temp root");
        nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(v4_dir.path(), APPLICATION_BUILD_IDENTITY, &[])
            .await
            .expect("v4 root");
        let v4_pool = super::open_validated_pool(
            &v4_dir.path().join(FRESH_V4_DATABASE_FILE),
        )
        .await
        .expect("v4 pool");
        let encrypted =
            encrypt_string(r#"{"api_keys":["host-test-key"]}"#, &[0x41; 32])
                .expect("encrypted credentials");
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            endpoint: Some("/chat"),
            provider_params: r#"{"temperature":0.25}"#,
            output_limit: Some(64),
            ..Default::default()
        }];
        let (provider, _) = SqliteProviderRepository::new(v4_pool.clone())
            .create(
                CreateProviderParams {
                    provider_id: None,
                    platform: "openai",
                    name: "host test provider",
                    base_url: &server.uri(),
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: Some(0),
                },
                &NewProviderModel {
                    model: "host-test-model",
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .expect("provider graph");
        let provider_id = ProviderIdRef::from(provider.provider_id.clone());
        let provider_repository =
            super::super::chat_broker_host::ProductionProviderRepository::new(
                v4_pool.clone(),
            );
        let provider_record = provider_repository
            .find_provider(&provider_id)
            .await
            .expect("provider digest")
            .expect("provider row");

        sqlx::query(
            "INSERT INTO agent_presets \
             (preset_id, owner_ref_json, source_json, display_json, \
              current_stable_revision, created_at) \
             VALUES (?, '{}', '{}', '{}', 1, 0)",
        )
        .bind("host-preset")
        .execute(&v4_pool)
        .await
        .expect("preset row");
        sqlx::query(
            "INSERT INTO agent_preset_revisions \
             (revision_id, preset_id, revision_no, schema_version, \
              editor_document_json, revision_digest, created_by, created_at, reason) \
             VALUES (?, ?, 1, '1.0.0', '{}', ?, 'host-test-owner', 0, '')",
        )
        .bind("host-preset@1")
        .bind("host-preset")
        .bind("a".repeat(64))
        .execute(&v4_pool)
        .await
        .expect("revision row");
        let route_record = ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: "host-route".into(),
                model_route_revision: 1,
                provider_id: provider.provider_id,
                model: "host-test-model".to_owned(),
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: "default".into(),
                config_revision_digest: provider_record.config_revision_digest,
                credential_ref: "host-credential".to_owned(),
                features: BTreeSet::from([
                    ChatRouteFeature::TextInput,
                    ChatRouteFeature::ImageInput,
                    ChatRouteFeature::AudioInput,
                    ChatRouteFeature::TextOutput,
                    ChatRouteFeature::ToolCalls,
                    ChatRouteFeature::Reasoning,
                    ChatRouteFeature::StructuredOutput,
                ]),
            },
            failovers: Vec::new(),
        };
        sqlx::query(
            "INSERT INTO agent_preset_model_routes \
             (revision_id, model_task, route_json) VALUES (?, ?, ?)",
        )
        .bind("host-preset@1")
        .bind("agent_chat")
        .bind(route_record.to_canonical_json().expect("route JSON"))
        .execute(&v4_pool)
        .await
        .expect("route row");

        let composition = ChatBrokerHostComposition::new(
            v4_pool.clone(),
            v4_pool.clone(),
            [0x41; 32],
            ConnectionCredentialLeaseRegistry::new(),
        );
        let broker = composition
            .build_broker(
                Arc::new(AllowChatGate),
                composition.build_model_invoke(
                    reqwest::Client::builder()
                        .no_proxy()
                        .build()
                        .expect("HTTP client"),
                ),
                BrokerRetryPolicy {
                    max_total_attempts: 1,
                    max_attempts_per_route: 1,
                },
            )
            .expect("production broker");
        let fixture = nomifun_chat_model_broker::recorded_conformance_fixtures()
            .into_iter()
            .find(|fixture| fixture.protocol == ChatProtocol::OpenaiChat)
            .expect("OpenAI Chat fixture");
        let mut request = fixture.request;
        let identity = ChatRouteIdentity::new(
            "host-preset@1",
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            "host-route".into(),
            1,
        );
        request.route = identity.clone();
        request.causality.route_identity = identity;
        let stream = broker
            .open_chat_stream(request)
            .await
            .expect("broker stream");
        (stream, server)
    }

    #[tokio::test]
    async fn production_host_chat_broker_streams_a_real_provider_response() {
        let (stream, server) = production_chat_fixture(200).await;
        let events = stream.collect::<Vec<_>>().await;
        assert!(events.iter().all(Result::is_ok));
        assert!(events.iter().any(|event| {
            event.as_ref().is_ok_and(|event| {
                matches!(
                    event.event,
                    nomifun_chat_model_broker::ChatModelEvent::OutputTextDelta { .. }
                )
            })
        }));
        assert!(events.last().is_some_and(|event| {
            event
                .as_ref()
                .is_ok_and(|event| event.event.is_terminal())
        }));
        let requests = server.received_requests().await.expect("requests");
        assert_eq!(requests.len(), 1);
        let body: serde_json::Value =
            serde_json::from_slice(&requests[0].body).expect("request JSON");
        assert_eq!(body["temperature"], 0.25);
        assert_eq!(body["max_tokens"], 64);
    }

    #[tokio::test]
    async fn production_host_chat_broker_reports_provider_unavailable_without_fake_output() {
        let (stream, server) = production_chat_fixture(503).await;
        let events = stream.collect::<Vec<_>>().await;
        let error = events
            .last()
            .expect("terminal broker error")
            .as_ref()
            .expect_err("provider failure");
        assert_eq!(error.code, ChatModelErrorCode::ProviderUnavailable);
        assert!(events.iter().all(|event| event.is_err()));
        assert_eq!(
            server.received_requests().await.expect("requests").len(),
            1
        );
    }

    #[tokio::test]
    #[ignore = "requires NOMIFUN_LIVE_STEPFUN_API_KEY; run explicitly with --ignored"]
    async fn production_host_chat_broker_streams_configured_stepfun_plan_response() {
        let api_key = std::env::var("NOMIFUN_LIVE_STEPFUN_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .expect("set NOMIFUN_LIVE_STEPFUN_API_KEY in the process environment");
        let base_url = std::env::var("NOMIFUN_LIVE_STEPFUN_BASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://api.stepfun.com/step_plan/v1".to_owned());
        let model = std::env::var("NOMIFUN_LIVE_STEPFUN_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "step-router-v1".to_owned());

        let directory = tempfile::tempdir().expect("live Step Plan temp root");
        let v4_dir = directory.path().join("v4");
        nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&v4_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .expect("live Step Plan v4 root");
        let pool = super::open_validated_pool(
            &v4_dir.join(FRESH_V4_DATABASE_FILE),
        )
        .await
        .expect("live Step Plan v4 pool");
        let credentials = encrypt_string(
            &serde_json::json!({ "api_keys": [api_key] }).to_string(),
            &[0x41; 32],
        )
        .expect("encrypt live Step Plan credentials");
        let capabilities = [NewProviderModelCapability {
            task: "chat",
            traits: "[]",
            protocol: "openai.chat_text",
            connection_role: "default",
            provider_params: r#"{"temperature":0.1}"#,
            output_limit: Some(128),
            ..Default::default()
        }];
        let (provider, _) = SqliteProviderRepository::new(pool.clone())
            .create(
                CreateProviderParams {
                    provider_id: None,
                    platform: "stepfun-plan",
                    name: "live Step Plan smoke provider",
                    base_url: &base_url,
                    auth_scheme: "bearer",
                    credentials_encrypted: &credentials,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: Some(0),
                },
                &NewProviderModel {
                    model: &model,
                    enabled: true,
                    sort_order: 0,
                    description: None,
                    capabilities: &capabilities,
                },
                &[],
            )
            .await
            .expect("live Step Plan provider graph");
        let provider_id = ProviderIdRef::from(provider.provider_id.clone());
        let provider_record = super::super::chat_broker_host::ProductionProviderRepository::new(
            pool.clone(),
        )
        .find_provider(&provider_id)
        .await
        .expect("live Step Plan provider digest")
        .expect("live Step Plan provider row");
        let preset_revision_id = "live-stepfun-plan@1";
        sqlx::query(
            "INSERT INTO agent_presets \
             (preset_id, owner_ref_json, source_json, display_json, \
              current_stable_revision, created_at) \
             VALUES (?, '{}', '{}', '{}', 1, 0)",
        )
        .bind("live-stepfun-plan")
        .execute(&pool)
        .await
        .expect("live Step Plan preset row");
        sqlx::query(
            "INSERT INTO agent_preset_revisions \
             (revision_id, preset_id, revision_no, schema_version, \
              editor_document_json, revision_digest, created_by, created_at, reason) \
             VALUES (?, ?, 1, '1.0.0', '{}', ?, 'live-stepfun-owner', 0, '')",
        )
        .bind(preset_revision_id)
        .bind("live-stepfun-plan")
        .bind("a".repeat(64))
        .execute(&pool)
        .await
        .expect("live Step Plan revision row");
        let route_record = ChatRouteRecord {
            schema: ChatRouteRecordSchema::V1,
            task: ChatRouteTask::AgentChat,
            primary: ChatRouteCandidate {
                model_route_id: "live-stepfun-route".into(),
                model_route_revision: 1,
                provider_id: provider.provider_id,
                model,
                protocol: ChatRouteProtocol::OpenaiChat,
                connection_config_ref: "default".into(),
                config_revision_digest: provider_record.config_revision_digest,
                credential_ref: "live-stepfun-credential".to_owned(),
                features: BTreeSet::from([
                    ChatRouteFeature::TextInput,
                    ChatRouteFeature::TextOutput,
                    ChatRouteFeature::Reasoning,
                    ChatRouteFeature::ToolCalls,
                ]),
            },
            failovers: Vec::new(),
        };
        sqlx::query(
            "INSERT INTO agent_preset_model_routes \
             (revision_id, model_task, route_json) VALUES (?, ?, ?)",
        )
        .bind(preset_revision_id)
        .bind("agent_chat")
        .bind(route_record.to_canonical_json().expect("live route JSON"))
        .execute(&pool)
        .await
        .expect("live Step Plan route row");

        let composition = ChatBrokerHostComposition::new(
            pool.clone(),
            pool.clone(),
            [0x41; 32],
            ConnectionCredentialLeaseRegistry::new(),
        );
        let broker = composition
            .build_broker(
                Arc::new(AllowChatGate),
                composition
                    .build_model_invoke(
                        nomifun_net::http_client_no_redirect()
                            .expect("live Step Plan HTTP client"),
                    ),
                BrokerRetryPolicy {
                    max_total_attempts: 1,
                    max_attempts_per_route: 1,
                },
            )
            .expect("live Step Plan production broker");
        let fixture = nomifun_chat_model_broker::recorded_conformance_fixtures()
            .into_iter()
            .find(|fixture| fixture.protocol == ChatProtocol::OpenaiChat)
            .expect("OpenAI Chat fixture");
        let mut request = fixture.request;
        let identity = ChatRouteIdentity::new(
            preset_revision_id,
            nomifun_agent_contracts::CHAT_MODEL_TASK_AGENT_CHAT,
            "live-stepfun-route".into(),
            1,
        );
        request.route = identity.clone();
        request.causality.route_identity = identity;
        request.input.instructions = vec!["Reply with exactly OK.".to_owned()];
        request.input.messages = vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ChatContentPart::Text {
                text: "Reply with exactly OK.".to_owned(),
            }],
            provider_round_id: None,
        }];
        request.input.tools.clear();
        request.input.tool_choice = ChatToolChoice::None;
        request.input.reasoning = None;
        request.input.prompt_cache = PromptCachePolicy::Disabled;
        request.input.response_format = ChatResponseFormat::Text;
        request.input.requested_output_modalities =
            BTreeSet::from([nomifun_chat_model_broker::ChatModality::Text]);
        request.input.provider_round_parent = None;
        request.input.preserve_native_responses_items = false;
        request.input.max_output_tokens = Some(128);
        let events = tokio::time::timeout(
            Duration::from_secs(60),
            broker
                .open_chat_stream(request)
                .await
                .expect("live Step Plan stream")
                .collect::<Vec<_>>(),
        )
        .await
        .expect("live Step Plan response exceeded the 60 second deadline");
        if let Some(error) = events.iter().find_map(|event| event.as_ref().err()) {
            panic!(
                "live Step Plan returned typed broker error code={:?} provider_status={:?}",
                error.code, error.provider_status
            );
        }
        assert!(
            events.iter().any(|event| {
                event.as_ref().is_ok_and(|event| {
                    matches!(
                        event.event,
                        nomifun_chat_model_broker::ChatModelEvent::OutputTextDelta { .. }
                    )
                })
            }),
            "live Step Plan response contained no output text deltas"
        );
        assert!(
            events.last().is_some_and(|event| {
                event
                    .as_ref()
                    .is_ok_and(|event| event.event.is_terminal())
            }),
            "live Step Plan response did not end with a canonical terminal event"
        );
        pool.close().await;
    }

    #[test]
    fn wave1_knowledge_binding_resolution_rechecks_host_authority() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir_all(&root).unwrap();
        let knowledge_base_id = KnowledgeBaseId::new();
        let binding = knowledge_binding(&knowledge_base_id, &root);
        let capability_id =
            CapabilityId::from(nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH);
        let action_id = nomifun_agent_domain_wave1::action_id(
            nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
        )
        .unwrap();
        let principal_id = principal().principal_id;
        let resolve = |bindings: &[TypedResourceBinding]| {
            resolve_bound_knowledge_base_parts(
                &principal_id,
                &capability_id,
                &action_id,
                bindings,
                nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
                "search",
            )
        };
        let error_code = |bindings: &[TypedResourceBinding]| {
            resolve(bindings).unwrap_err().code.as_ref().to_owned()
        };

        let resolved =
            resolve(std::slice::from_ref(&binding)).expect("valid binding");
        assert_eq!(
            resolved.knowledge_base_id().as_str(),
            knowledge_base_id.as_str()
        );

        let mut wrong_owner = binding.clone();
        wrong_owner.owner_id = "different-owner".to_owned();
        assert_eq!(
            error_code(&[wrong_owner]),
            "RESOURCE_OWNER_MISMATCH"
        );

        let mut missing_grant = binding.clone();
        missing_grant.operations.remove("search");
        assert_eq!(
            error_code(&[missing_grant]),
            "PRESET_RESOURCE_NOT_BOUND"
        );

        let mut invalid_id = binding.clone();
        invalid_id.resource_id = ResourceId::from("not-a-uuidv7");
        assert_eq!(error_code(&[invalid_id]), "INVALID_PAYLOAD");

        let mut missing_root = binding.clone();
        missing_root
            .typed_parameters
            .remove(KNOWLEDGE_ROOT_PARAMETER);
        assert_eq!(
            error_code(&[missing_root]),
            "PRESET_RESOURCE_NOT_BOUND"
        );

        let mut second_binding = binding.clone();
        second_binding.binding_id = ResourceBindingId::from("knowledge-secondary");
        assert_eq!(
            error_code(&[binding, second_binding]),
            "PRESET_RESOURCE_NOT_BOUND"
        );
    }

    #[tokio::test]
    async fn wave1_knowledge_owner_searches_and_reads_real_bound_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir_all(root.join("release")).unwrap();
        let content = "# Release rollback\nRun the signed rollback plan.";
        std::fs::write(root.join("release").join("rollback.md"), content)
            .unwrap();

        let fixture = KnowledgeKernelFixture::new();
        let knowledge_base_id = KnowledgeBaseId::new();
        let binding = knowledge_binding(&knowledge_base_id, &root);
        let (snapshot, active, binding) =
            fixture.compile_snapshot(binding);
        let search = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
                serde_json::json!({
                    "query": "rollback",
                    "limit": 5,
                }),
                "search",
            )
            .await
            .expect("bound Knowledge search");
        assert_eq!(search.0["total"], serde_json::json!(1));
        assert_eq!(
            search.0["hits"][0]["resource_id"],
            serde_json::json!(knowledge_base_id)
        );
        assert_eq!(
            search.0["hits"][0]["relative_path"],
            serde_json::json!("release/rollback.md")
        );
        assert!(
            !search
                .0
                .to_string()
                .contains(&root.to_string_lossy().to_string()),
            "Knowledge search must not expose an absolute host path"
        );
        let handle = search.0["hits"][0]["handle"]
            .as_str()
            .expect("search handle")
            .to_owned();

        let read = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_READ,
                serde_json::json!({ "handle": handle }),
                "read",
            )
            .await
            .expect("bound Knowledge read");
        assert_eq!(read.0["content"], serde_json::json!(content));
        assert_eq!(
            read.0["relative_path"],
            serde_json::json!("release/rollback.md")
        );
        assert_eq!(
            read.0["content_sha256"],
            serde_json::json!(digest_bytes(content.as_bytes()))
        );
    }

    #[tokio::test]
    async fn wave1_knowledge_owner_rejects_scope_escape_and_missing_files() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("knowledge");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("note.md"), "# In scope").unwrap();
        std::fs::write(directory.path().join("outside.md"), "# Outside")
            .unwrap();

        let fixture = KnowledgeKernelFixture::new();
        let knowledge_base_id = KnowledgeBaseId::new();
        let binding = knowledge_binding(&knowledge_base_id, &root);
        let (snapshot, active, binding) =
            fixture.compile_snapshot(binding);

        let wrong_resource = nomifun_knowledge::encode_doc_handle(
            &KnowledgeBaseId::new(),
            "note.md",
        );
        let wrong_resource_error = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_READ,
                serde_json::json!({ "handle": wrong_resource }),
                "wrong-resource",
            )
            .await
            .expect_err("a handle cannot widen the bound resource scope");
        assert!(
            wrong_resource_error
                .to_string()
                .contains("PRESET_RESOURCE_NOT_BOUND"),
            "unexpected wrong-resource error: {wrong_resource_error}"
        );

        let traversal = nomifun_knowledge::encode_doc_handle(
            &knowledge_base_id,
            "../outside.md",
        );
        let traversal_error = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_READ,
                serde_json::json!({ "handle": traversal }),
                "traversal",
            )
            .await
            .expect_err("a handle cannot traverse outside its bound root");
        assert!(
            traversal_error.to_string().contains("INVALID_PAYLOAD"),
            "unexpected traversal error: {traversal_error}"
        );

        let missing = nomifun_knowledge::encode_doc_handle(
            &knowledge_base_id,
            "missing.md",
        );
        let missing_error = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_READ,
                serde_json::json!({ "handle": missing }),
                "missing-file",
            )
            .await
            .expect_err("a missing file must not produce synthetic content");
        assert!(
            missing_error.to_string().contains("RESOURCE_NOT_FOUND"),
            "unexpected missing-file error: {missing_error}"
        );
    }

    #[tokio::test]
    async fn wave1_knowledge_owner_fails_closed_for_missing_bound_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("missing-knowledge-root");
        let fixture = KnowledgeKernelFixture::new();
        let knowledge_base_id = KnowledgeBaseId::new();
        let binding = knowledge_binding(&knowledge_base_id, &root);
        let (snapshot, active, binding) =
            fixture.compile_snapshot(binding);

        let error = fixture
            .invoke(
                &snapshot,
                &active,
                &binding,
                nomifun_agent_domain_wave1::KNOWLEDGE_SEARCH,
                serde_json::json!({ "query": "anything" }),
                "missing-root",
            )
            .await
            .expect_err("a missing root must fail instead of returning no hits");
        assert!(
            error.to_string().contains("CAPABILITY_UNAVAILABLE"),
            "unexpected missing-root error: {error}"
        );
        assert!(
            !error
                .to_string()
                .contains(&root.to_string_lossy().to_string()),
            "Knowledge errors must not expose an absolute host path"
        );
    }

    #[tokio::test]
    async fn wave1_memory_owner_persists_and_replays_by_request_identity() {
        let fixture = MemoryKernelFixture::new();
        let (snapshot, active, binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-replay",
            );
        let request = memory_invocation(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "memory-replay-1",
            "remember this",
            "memory-operation-1",
        );
        let first = fixture
            .registry
            .invoke(&snapshot, &active, request.clone())
            .await
            .expect("first memory mutation");
        assert_eq!(first.0["persisted"], serde_json::json!(true));
        assert_eq!(first.0["revision"], serde_json::json!(1));
        assert_eq!(first.0["entry_count"], serde_json::json!(1));

        let state = fixture.persistence.snapshot().expect("state snapshot");
        let stored = state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-replay"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("project memory state");
        assert_eq!(stored.revision, 1);
        assert_eq!(stored.value.0["entries"].as_array().unwrap().len(), 1);
        assert_eq!(
            stored.value.0["entries"][0]["request"]["content"],
            serde_json::json!("remember this")
        );

        let mut replay_request = request.clone();
        replay_request.operation_id = OperationId::from("memory-operation-retry");
        replay_request.correlation_id = CorrelationId::from("memory-correlation-retry");
        let replay = fixture
            .registry
            .invoke(&snapshot, &active, replay_request)
            .await
            .expect("idempotent replay");
        assert_eq!(replay, first);
        let replayed_state = fixture.persistence.snapshot().expect("state snapshot");
        let replayed = replayed_state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-replay"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("project memory state after replay");
        assert_eq!(replayed.revision, 1);
        assert_eq!(replayed.value.0["entries"].as_array().unwrap().len(), 1);

        let conflicting_request = memory_invocation(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "memory-replay-1",
            "different content",
            "memory-operation-conflict",
        );
        let conflict = fixture
            .registry
            .invoke(&snapshot, &active, conflicting_request)
            .await
            .expect_err("different input must conflict");
        assert!(
            conflict
                .to_string()
                .contains("IDEMPOTENCY_CONFLICT"),
            "unexpected conflict: {conflict}"
        );
        let unchanged = fixture.persistence.snapshot().expect("state snapshot");
        let unchanged_entry = unchanged
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-replay"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("project memory state after conflict");
        assert_eq!(unchanged_entry.revision, 1);
        assert_eq!(unchanged_entry.value.0["entries"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn wave1_memory_owner_isolates_resources_and_package_mounts() {
        let fixture = MemoryKernelFixture::new();
        let (project_a, active_a, binding_a) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "memory-a",
            );
        let (project_b, active_b, binding_b) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "memory-b",
            );
        let (companion, active_companion, companion_binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
                "memory-a",
            );

        for (snapshot, active, binding, capability_id, content) in [
            (
                project_a,
                active_a,
                binding_a,
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project a",
            ),
            (
                project_b,
                active_b,
                binding_b,
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project b",
            ),
            (
                companion,
                active_companion,
                companion_binding,
                nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
                "companion a",
            ),
        ] {
            fixture
                .registry
                .invoke(
                    &snapshot,
                    &active,
                    memory_invocation(
                        &snapshot,
                        &active,
                        &binding,
                        capability_id,
                        "shared-idempotency-key",
                        content,
                        content,
                    ),
                )
                .await
                .expect("isolated memory mutation");
        }

        let state = fixture.persistence.snapshot().expect("state snapshot");
        for (package_id, mount_id, scope, expected_content) in [
            (
                nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID,
                nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID,
                "resource:memory-a",
                "project a",
            ),
            (
                nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID,
                nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID,
                "resource:memory-b",
                "project b",
            ),
            (
                nomifun_agent_domain_wave1::COMPANION_MEMORY_PACKAGE_ID,
                nomifun_agent_domain_wave1::COMPANION_MEMORY_MOUNT_ID,
                "resource:memory-a",
                "companion a",
            ),
        ] {
            let entry = state
                .entry(
                    &package_id.into(),
                    &mount_id.into(),
                    &ScopeKey::from(scope),
                    &StateKey::from(MEMORY_STATE_KEY),
                )
                .expect("isolated state entry");
            assert_eq!(entry.value.0["entries"].as_array().unwrap().len(), 1);
            assert_eq!(
                entry.value.0["entries"][0]["request"]["content"],
                serde_json::json!(expected_content)
            );
        }
    }

    #[tokio::test]
    async fn wave1_memory_owner_dispatches_all_mutation_variants() {
        let fixture = MemoryKernelFixture::new();
        for (index, (capability_id, resource_id, expected_operation)) in [
            (
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "variant-project",
                "project.write",
            ),
            (
                nomifun_agent_domain_wave1::MEMORY_PROJECT_DISTILL,
                "variant-project",
                "project.distill",
            ),
            (
                nomifun_agent_domain_wave1::MEMORY_COMPANION_WRITE,
                "variant-companion",
                "companion.write",
            ),
            (
                nomifun_agent_domain_wave1::MEMORY_COMPANION_MERGE,
                "variant-companion",
                "companion.merge",
            ),
            (
                nomifun_agent_domain_wave1::MEMORY_COMPANION_EVOLVE,
                "variant-companion",
                "companion.evolve",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (snapshot, active, binding) = fixture
                .compile_memory_snapshot(capability_id, resource_id);
            let output = fixture
                .registry
                .invoke(
                    &snapshot,
                    &active,
                    memory_invocation(
                        &snapshot,
                        &active,
                        &binding,
                        capability_id,
                        &format!("variant-key-{index}"),
                        &format!("variant-{index}"),
                        &format!("variant-operation-{index}"),
                    ),
                )
                .await
                .expect("memory mutation variant");
            assert_eq!(output.0["operation"], serde_json::json!(expected_operation));
        }
    }

    #[tokio::test]
    async fn wave1_memory_owner_survives_kernel_restart_and_concurrent_cas() {
        let fixture = MemoryKernelFixture::new();
        let (snapshot, active, binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-restart",
            );
        let first_request = memory_invocation(
            &snapshot,
            &active,
            &binding,
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "restart-key",
            "before restart",
            "restart-operation",
        );
        let first = fixture
            .registry
            .invoke(&snapshot, &active, first_request.clone())
            .await
            .expect("pre-restart mutation");
        let persisted_snapshot = fixture.persistence.snapshot().expect("persisted state");

        let restarted_persistence = Arc::new(
            InMemoryPluginStatePersistence::reopen(persisted_snapshot),
        );
        let restarted = MemoryKernelFixture::with_persistence(restarted_persistence.clone());
        let (restarted_snapshot, restarted_active, restarted_binding) = restarted
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-restart",
            );
        let replay = restarted
            .registry
            .invoke(
                &restarted_snapshot,
                &restarted_active,
                memory_invocation(
                    &restarted_snapshot,
                    &restarted_active,
                    &restarted_binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    "restart-key",
                    "before restart",
                    "restart-operation-retry",
                ),
            )
            .await
            .expect("post-restart replay");
        assert_eq!(replay, first);

        let task_count = 12;
        let mut tasks = Vec::with_capacity(task_count);
        for index in 0..task_count {
            let registry = Arc::clone(&restarted.registry);
            let snapshot = Arc::clone(&restarted_snapshot);
            let active = restarted_active.clone();
            let binding = restarted_binding.clone();
            tasks.push(tokio::spawn(async move {
                registry
                    .invoke(
                        &snapshot,
                        &active,
                        memory_invocation(
                            &snapshot,
                            &active,
                            &binding,
                            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                            &format!("concurrent-key-{index}"),
                            &format!("concurrent-{index}"),
                            &format!("concurrent-operation-{index}"),
                        ),
                    )
                    .await
            }));
        }
        for task in tasks {
            task.await
                .expect("concurrent task")
                .expect("concurrent CAS mutation");
        }
        let state = restarted_persistence.snapshot().expect("restarted state");
        let entry = state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-restart"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("restarted project memory state");
        assert_eq!(
            entry.value.0["entries"].as_array().unwrap().len(),
            task_count + 1
        );
        assert_eq!(entry.revision, task_count as u64 + 1);
    }

    #[tokio::test]
    async fn wave1_memory_owner_enforces_bounded_state_without_partial_append() {
        let fixture = MemoryKernelFixture::new();
        let (snapshot, active, binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-capacity",
            );
        let content = "x".repeat(1_024);
        let mut successful = 0usize;
        let mut terminal_error = None;
        for index in 0..MAX_MEMORY_ENTRIES {
            let result = fixture
                .registry
                .invoke(
                    &snapshot,
                    &active,
                    memory_invocation(
                        &snapshot,
                        &active,
                        &binding,
                        nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                        &format!("capacity-key-{index}"),
                        &content,
                        &format!("capacity-operation-{index}"),
                    ),
                )
                .await;
            match result {
                Ok(_) => successful += 1,
                Err(error) => {
                    terminal_error = Some(error);
                    break;
                }
            }
        }
        let terminal_error = terminal_error.expect("bounded state must eventually reject");
        assert!(
            terminal_error
                .to_string()
                .contains("CAPABILITY_UNAVAILABLE"),
            "unexpected capacity error: {terminal_error}"
        );
        assert!(successful > 0 && successful < MAX_MEMORY_ENTRIES);

        let state = fixture.persistence.snapshot().expect("state snapshot");
        let entry = state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-capacity"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("capacity state");
        assert_eq!(entry.revision, successful as u64);
        assert_eq!(
            entry.value.0["entries"].as_array().unwrap().len(),
            successful
        );
    }

    #[tokio::test]
    async fn wave1_memory_owner_survives_sqlite_platform_restart() {
        let directory = tempfile::tempdir().expect("v4 temp root");
        let data_dir = directory.path().join("data");
        let outcome = nomifun_v4_root::FreshV4Coordinator::default()
            .bootstrap(&data_dir, APPLICATION_BUILD_IDENTITY, &[])
            .await
            .expect("fresh v4 root");
        let schema_digest = canonical_schema_manifest_digest().unwrap();
        let host_ports = || {
            AgentDomainHostPorts::with_wave1_and_wave2(
                Arc::new(Wave1ApplicationHost::default()),
                Arc::new(Wave2ApplicationHost::new()),
            )
        };
        let first_pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
            .await
            .expect("first v4 pool");
        let first_platform = initialize_platform_with_cleanup_and_host_ports(
            first_pool,
            data_dir.join(FRESH_V4_READY_MARKER_FILE),
            outcome.ready_marker.clone(),
            schema_digest.clone(),
            None,
            [0; 32],
            host_ports(),
        )
        .await
        .expect("first Agent platform");
        let (snapshot, active, binding) = compile_memory_snapshot_for_registry(
            first_platform.kernel_registry(),
            nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            "sqlite-project",
        );
        let first = first_platform
            .kernel_registry()
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    "sqlite-restart-key",
                    "durable sqlite memory",
                    "sqlite-operation-1",
                ),
            )
            .await
            .expect("SQLite-backed memory mutation");
        assert_eq!(first.0["revision"], serde_json::json!(1));
        let first_row: (String, i64) = sqlx::query_as(
            "SELECT value_json, cas_revision FROM plugin_states \
             WHERE package_id = ? AND mount_id = ? AND scope_key = ? AND state_key = ?",
        )
        .bind(nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID)
        .bind(nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID)
        .bind("resource:sqlite-project")
        .bind(MEMORY_STATE_KEY)
        .fetch_one(first_platform.pool())
        .await
        .expect("SQLite PluginState row");
        assert_eq!(first_row.1, 1);
        let first_value: serde_json::Value =
            serde_json::from_str(&first_row.0).expect("stored state JSON");
        assert_eq!(
            first_value["entries"][0]["request"]["content"],
            serde_json::json!("durable sqlite memory")
        );

        first_platform.shutdown().await.expect("first platform shutdown");
        first_platform.pool().close().await;

        let second_pool = open_validated_pool(&data_dir.join(FRESH_V4_DATABASE_FILE))
            .await
            .expect("second v4 pool");
        let second_platform = initialize_platform_with_cleanup_and_host_ports(
            second_pool,
            data_dir.join(FRESH_V4_READY_MARKER_FILE),
            outcome.ready_marker,
            schema_digest,
            None,
            [0; 32],
            host_ports(),
        )
        .await
        .expect("restarted Agent platform");
        let (restarted_snapshot, restarted_active, restarted_binding) =
            compile_memory_snapshot_for_registry(
                second_platform.kernel_registry(),
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "sqlite-project",
            );
        let replay = second_platform
            .kernel_registry()
            .invoke(
                &restarted_snapshot,
                &restarted_active,
                memory_invocation(
                    &restarted_snapshot,
                    &restarted_active,
                    &restarted_binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    "sqlite-restart-key",
                    "durable sqlite memory",
                    "sqlite-operation-2",
                ),
            )
            .await
            .expect("SQLite-backed idempotent replay");
        assert_eq!(replay, first);
        let second_revision: i64 = sqlx::query_scalar(
            "SELECT cas_revision FROM plugin_states \
             WHERE package_id = ? AND mount_id = ? AND scope_key = ? AND state_key = ?",
        )
        .bind(nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID)
        .bind(nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID)
        .bind("resource:sqlite-project")
        .bind(MEMORY_STATE_KEY)
        .fetch_one(second_platform.pool())
        .await
        .expect("replayed SQLite PluginState row");
        assert_eq!(second_revision, 1);
        second_platform.shutdown().await.expect("second platform shutdown");
        second_platform.pool().close().await;
    }

    #[tokio::test]
    async fn wave1_memory_owner_rejects_corrupt_plugin_state_without_overwrite() {
        let persistence = malformed_memory_persistence();
        let fixture = MemoryKernelFixture::with_persistence(Arc::clone(&persistence));
        let (snapshot, active, binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-corrupt",
            );
        let error = fixture
            .registry
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    "corrupt-repair-attempt",
                    "must not overwrite",
                    "corrupt-operation",
                ),
            )
            .await
            .expect_err("corrupt state must fail closed");
        assert!(
            error
                .to_string()
                .contains("CAPABILITY_UNAVAILABLE"),
            "unexpected corrupt-state error: {error}"
        );
        let state = persistence.snapshot().expect("state snapshot");
        let entry = state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-corrupt"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("corrupt state remains");
        assert_eq!(entry.revision, 1);
        assert_eq!(entry.value.0["entries"], serde_json::json!("not-an-array"));
    }

    #[tokio::test]
    async fn wave1_memory_owner_rejects_unsupported_state_format() {
        let persistence = memory_persistence_with_state(
            serde_json::json!({"entries": []}),
            "2.0.0",
            "project-format",
        );
        let fixture = MemoryKernelFixture::with_persistence(Arc::clone(&persistence));
        let (snapshot, active, binding) = fixture
            .compile_memory_snapshot(
                nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                "project-format",
            );
        let error = fixture
            .registry
            .invoke(
                &snapshot,
                &active,
                memory_invocation(
                    &snapshot,
                    &active,
                    &binding,
                    nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
                    "format-key",
                    "must not migrate implicitly",
                    "format-operation",
                ),
            )
            .await
            .expect_err("unsupported state format must fail closed");
        assert!(
            error
                .to_string()
                .contains("CAPABILITY_UNAVAILABLE"),
            "unexpected state-format error: {error}"
        );
        let state = persistence.snapshot().expect("state snapshot");
        let entry = state
            .entry(
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID.into(),
                &nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID.into(),
                &ScopeKey::from("resource:project-format"),
                &StateKey::from(MEMORY_STATE_KEY),
            )
            .expect("format state remains");
        assert_eq!(entry.state_format_version.as_ref(), "2.0.0");
        assert_eq!(entry.value.0["entries"], serde_json::json!([]));
    }

    #[test]
    fn wave1_application_errors_keep_typed_owner_codes() {
        let invalid = wave1_application_error(nomifun_common::AppError::BadRequest(
            "bad URL".to_owned(),
        ));
        assert_eq!(invalid.code.as_ref(), "INVALID_PAYLOAD");

        let unavailable = wave1_application_error(nomifun_common::AppError::Timeout(
            "network timeout".to_owned(),
        ));
        assert_eq!(unavailable.code.as_ref(), "CAPABILITY_UNAVAILABLE");

        let hidden_path = PathBuf::from(r"C:\Users\owner\private-knowledge");
        let knowledge = wave1_bound_knowledge_error(
            nomifun_common::AppError::Internal(format!(
                "failed to inspect {}",
                hidden_path.display()
            )),
        );
        assert_eq!(knowledge.code.as_ref(), "CAPABILITY_UNAVAILABLE");
        assert!(!knowledge.message.contains("private-knowledge"));
    }
}
