//! Engine-neutral Knowledge and memory adapters owned by the application.
//! Engines consume Kernel ports; they do not own these domain services.

use super::agent_wave1_companion_host::Wave1CompanionMemoryHost;
use nomifun_agent_contracts::{canonical_json_bytes, digest_payload};
use nomifun_agent_domain_wave1::{
    Wave1CapabilityOperation, Wave1ContextHostRequest, Wave1FetchRequest, Wave1HostPort,
    Wave1HostPortError, Wave1HostRequest, Wave1KnowledgeReadRequest, Wave1MemoryMutationRequest,
    Wave1SearchRequest,
};
use nomifun_agent_kernel::{MAX_PLUGIN_STATE_BYTES, MAX_PLUGIN_STATE_KEY_BYTES};
use sqlx::SqlitePool;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

/// Shared Wave 1 domain owner for Conversation-backed engines.
///
/// URL fetching already has a standalone, SSRF-checked domain owner. This
/// adapter exposes that real operation, binding-backed Knowledge reads,
/// bounded project-memory coordination in Kernel PluginState, and—when the
/// app supplies it—the persistent CompanionStore memory owner. Research
/// search, Knowledge mutations, and Skill actions remain fail-closed until
/// their domain owners are available through this port.
#[derive(Clone)]
struct Wave1ApplicationHost {
    fetcher: nomifun_knowledge::source_url::HttpFetcher,
    knowledge_reader: nomifun_knowledge::BoundKnowledgeReadService,
    companion_memory: Option<Wave1CompanionMemoryHost>,
}

impl Default for Wave1ApplicationHost {
    fn default() -> Self {
        Self {
            fetcher: nomifun_knowledge::source_url::HttpFetcher::default(),
            knowledge_reader: nomifun_knowledge::BoundKnowledgeReadService::default(),
            companion_memory: None,
        }
    }
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
}

impl Wave1MemoryOperation {
    fn label(self) -> &'static str {
        match self {
            Self::ProjectWrite => "project.write",
            Self::ProjectDistill => "project.distill",
        }
    }

    fn capability_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => nomifun_agent_domain_wave1::MEMORY_PROJECT_WRITE,
            Self::ProjectDistill => nomifun_agent_domain_wave1::MEMORY_PROJECT_DISTILL,
        }
    }

    fn package_id(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID
            }
        }
    }

    fn mount_id(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID
            }
        }
    }

    fn resource_kind(self) -> &'static str {
        match self {
            Self::ProjectWrite | Self::ProjectDistill => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_RESOURCE_KIND
            }
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "project.write" => Some(Self::ProjectWrite),
            "project.distill" => Some(Self::ProjectDistill),
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
                Ok(nomifun_agent_contracts::StrictJsonValue(
                    serde_json::json!({
                        "url": page.final_url,
                        "title": page.title,
                        "markdown": page.markdown,
                        "truncated": page.truncated
                    }),
                ))
            }
            Wave1CapabilityOperation::KnowledgeSearch(request) => {
                self.search_knowledge(context, request).await
            }
            Wave1CapabilityOperation::KnowledgeRead(request) => {
                self.read_knowledge(context, request).await
            }
            Wave1CapabilityOperation::ProjectMemoryWrite(request) => {
                self.persist_memory(context, Wave1MemoryOperation::ProjectWrite, request)
                    .await
            }
            Wave1CapabilityOperation::ProjectMemoryDistill(request) => {
                self.persist_memory(context, Wave1MemoryOperation::ProjectDistill, request)
                    .await
            }
            Wave1CapabilityOperation::CompanionMemoryWrite(request) => {
                self.companion_memory()?.write(context, request).await
            }
            Wave1CapabilityOperation::CompanionMemoryMerge(request) => {
                self.companion_memory()?.merge(context, request).await
            }
            Wave1CapabilityOperation::CompanionMemoryEvolve(request) => {
                self.companion_memory()?.evolve(context, request).await
            }
            operation => Err(Wave1HostPortError::unavailable(format!(
                "no Nomi Wave 1 owner is wired for {}",
                operation.capability_id().as_ref()
            ))),
        }
    }

    async fn contribute_context(
        &self,
        request: Wave1ContextHostRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        if request.context.capability_id.as_ref()
            != nomifun_agent_domain_wave1::MEMORY_COMPANION_RECALL
        {
            return Err(Wave1HostPortError::unavailable(format!(
                "no Nomi Wave 1 context owner is wired for {}",
                request.context.capability_id.as_ref()
            )));
        }
        self.companion_memory()?.recall(request.context).await
    }
}

impl Wave1ApplicationHost {
    fn with_companion_memory(
        service: Arc<nomifun_companion::CompanionService>,
        receipt_pool: SqlitePool,
    ) -> Self {
        Self {
            companion_memory: Some(Wave1CompanionMemoryHost::new(service, receipt_pool)),
            ..Self::default()
        }
    }

    fn companion_memory(&self) -> Result<&Wave1CompanionMemoryHost, Wave1HostPortError> {
        self.companion_memory.as_ref().ok_or_else(|| {
            Wave1HostPortError::unavailable(
                "the persistent Companion memory owner is not configured",
            )
        })
    }

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
        let handle_resource_id = nomifun_knowledge::decode_doc_handle(&request.handle)
            .map(|(knowledge_base_id, _)| knowledge_base_id)
            .ok_or_else(|| {
                Wave1HostPortError::new("INVALID_PAYLOAD", "invalid knowledge document handle")
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
        Ok(nomifun_agent_contracts::StrictJsonValue(serde_json::json!(
            document
        )))
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

        // Project memory is shared by the exact bound resource, not by a
        // transient Session. Companion memory intentionally does not enter
        // this PluginState path; it is owned by the persistent CompanionStore
        // adapter above.
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
            let current = state.get(&scope, &state_key).await.map_err(|error| {
                Wave1HostPortError::new(
                    "CAPABILITY_UNAVAILABLE",
                    format!("memory state could not be read: {error}"),
                )
            })?;
            let revision = current.as_ref().map(|entry| entry.revision).unwrap_or(0);
            let mut entries = decode_memory_state(current.as_ref(), &format, operation, binding)?;
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
                    format!("memory state reached its {MAX_MEMORY_ENTRIES} entry limit"),
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
                    format!("memory entry exceeds {MAX_MEMORY_ENTRY_BYTES} bytes"),
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
                .compare_and_swap(&scope, &state_key, revision, &format, Some(next))
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
    if actual_capability_id.as_ref() != capability_id || actual_action_id != &expected_action {
        return Err(Wave1HostPortError::new(
            "INVALID_PAYLOAD",
            format!("{capability_id} operation identity does not match the host context"),
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
            format!("knowledge resource ID must be a canonical UUIDv7: {error}"),
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

    nomifun_knowledge::BoundKnowledgeBase::new(knowledge_base_id, name, PathBuf::from(root))
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
            Wave1HostPortError::unavailable("memory state has an invalid stored entries array")
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
            Wave1HostPortError::unavailable("memory state revision is older than its entry history")
        })?;
    let mut idempotency_keys = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let entry_object = entry.as_object().ok_or_else(|| {
            Wave1HostPortError::unavailable(format!("memory state entry {index} is not an object"))
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
            Wave1HostPortError::unavailable(format!("memory state entry {index} has no request"))
        })?;
        validate_stored_memory_request(request, operation.label(), index)?;
        let request_digest = entry_object
            .get("request_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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
            !matches!(
                key.as_str(),
                "persisted" | "operation" | "revision" | "entry_count"
            )
        }) || result.get("persisted") != Some(&serde_json::Value::Bool(true))
            || result.get("operation").and_then(serde_json::Value::as_str) != Some(entry_operation)
            || result.get("revision").and_then(serde_json::Value::as_u64)
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
        let last_operation = last_operation.as_str().ok_or_else(|| {
            Wave1HostPortError::unavailable("memory state last_operation is not a string")
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
        Wave1HostPortError::unavailable(format!("memory state could not be encoded: {error}"))
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
    Wave1HostPortError::new(CanonicalErrorCode::from(code), error.to_string())
}

fn wave1_bound_knowledge_error(error: nomifun_common::AppError) -> Wave1HostPortError {
    use nomifun_agent_contracts::CanonicalErrorCode;

    let (code, message) = match error {
        nomifun_common::AppError::BadRequest(_) => {
            ("INVALID_PAYLOAD", "bound knowledge request is invalid")
        }
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

/// Build only the real Wave 1 registrations for the current
/// Conversation-backed Nomi core. Other Waves stay with their own Nomi-specific
/// composition; this function deliberately does not construct Fresh-v4 MCP or
/// pass the legacy application pool into a Fresh repository adapter.
pub(crate) fn wave1_registrations_for_nomi_core(
    companion_service: Arc<nomifun_companion::CompanionService>,
    receipt_pool: SqlitePool,
) -> anyhow::Result<Vec<nomifun_agent_kernel::PluginRegistration>> {
    nomifun_agent_domain_wave1::registrations_with_host_port(Arc::new(
        Wave1ApplicationHost::with_companion_memory(companion_service, receipt_pool),
    ))
    .map_err(anyhow::Error::msg)
}

#[cfg(test)]
#[path = "agent_wave1_host_tests.rs"]
mod tests;
