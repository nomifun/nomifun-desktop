//! Engine-neutral Knowledge and memory adapters owned by the application.
//! Engines consume Kernel ports; they do not own these domain services.

use super::agent_wave1_companion_host::Wave1CompanionMemoryHost;
use nomifun_agent_contracts::{canonical_json_bytes, digest_payload};
use nomifun_agent_domain_wave1::{
    Wave1CapabilityOperation, Wave1FetchRequest, Wave1HostPort, Wave1HostPortError,
    Wave1HostRequest, Wave1KnowledgeAutogenRequest, Wave1KnowledgeReadRequest,
    Wave1KnowledgeWriteRequest, Wave1MemoryMutationRequest, Wave1ProjectMemoryReadRequest,
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
    knowledge: Arc<nomifun_knowledge::KnowledgeService>,
    search: Option<Arc<dyn nomifun_ai_agent::web_search::SearchProvider>>,
    companion_memory: Option<Wave1CompanionMemoryHost>,
}

impl Wave1ApplicationHost {
    fn new(
        knowledge: Arc<nomifun_knowledge::KnowledgeService>,
        search: Option<Arc<dyn nomifun_ai_agent::web_search::SearchProvider>>,
    ) -> Self {
        Self {
            fetcher: nomifun_knowledge::source_url::HttpFetcher::default(),
            knowledge,
            search,
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
}

impl Wave1MemoryOperation {
    fn label(self) -> &'static str {
        match self {
            Self::ProjectWrite => "project.write",
        }
    }

    fn capability_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => nomifun_agent_domain_wave1::PROJECT_MEMORY_MODULE_ID,
        }
    }

    fn action_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => nomifun_agent_domain_wave1::PROJECT_MEMORY_WRITE_ACTION_ID,
        }
    }

    fn package_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_PACKAGE_ID
            }
        }
    }

    fn mount_id(self) -> &'static str {
        match self {
            Self::ProjectWrite => {
                nomifun_agent_domain_wave1::PROJECT_MEMORY_MOUNT_ID
            }
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "project.write" => Some(Self::ProjectWrite),
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
            Wave1CapabilityOperation::ResearchSearch(request) => {
                self.search_web(context, request).await
            }
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
            Wave1CapabilityOperation::KnowledgeWrite(request) => {
                self.write_knowledge(context, request).await
            }
            Wave1CapabilityOperation::KnowledgeAutogen(request) => {
                self.autogen_knowledge(context, request).await
            }
            Wave1CapabilityOperation::ProjectMemoryRead(request) => {
                self.read_memory(context, request).await
            }
            Wave1CapabilityOperation::ProjectMemoryWrite(request) => {
                self.persist_memory(context, Wave1MemoryOperation::ProjectWrite, request)
                    .await
            }
            Wave1CapabilityOperation::CompanionMemoryRecall(request) => {
                self.companion_memory()?.recall(context, request).await
            }
            Wave1CapabilityOperation::CompanionMemoryWrite(request) => {
                self.companion_memory()?.write(context, request).await
            }
        }
    }

}

impl Wave1ApplicationHost {
    fn with_companion_memory(
        knowledge: Arc<nomifun_knowledge::KnowledgeService>,
        search: Option<Arc<dyn nomifun_ai_agent::web_search::SearchProvider>>,
        service: Arc<nomifun_companion::CompanionService>,
        receipt_pool: SqlitePool,
    ) -> Self {
        Self {
            companion_memory: Some(Wave1CompanionMemoryHost::new(service, receipt_pool)),
            ..Self::new(knowledge, search)
        }
    }

    fn companion_memory(&self) -> Result<&Wave1CompanionMemoryHost, Wave1HostPortError> {
        self.companion_memory.as_ref().ok_or_else(|| {
            Wave1HostPortError::unavailable(
                "the persistent Companion memory owner is not configured",
            )
        })
    }

    async fn search_web(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1SearchRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let provider = self.search.as_ref().ok_or_else(|| {
            Wave1HostPortError::unavailable(
                "Web Research has no configured provider on this host",
            )
        })?;
        let citations = nomifun_ai_agent::web_search::SessionCitationStore::for_scope(
            context.agent_session_id.as_ref(),
        )
        .map_err(Wave1HostPortError::invalid_request)?;
        nomifun_ai_agent::web_search::search_with_derived_citations(
            provider.as_ref(),
            &citations,
            &request.query,
            request.limit.unwrap_or(5),
        )
        .await
        .map(nomifun_agent_contracts::StrictJsonValue)
        .map_err(wave1_search_error)
    }

    async fn search_knowledge(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1SearchRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let service = self.authorized_knowledge(&context)?;
        let hits = service
            .search(&request.query, request.limit.unwrap_or(DEFAULT_KNOWLEDGE_SEARCH_LIMIT))
            .await
            .map_err(wave1_bound_knowledge_error)?;
        Ok(nomifun_agent_contracts::StrictJsonValue(
            serde_json::json!({
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
        let document = self
            .authorized_knowledge(&context)?
            .read(&request.handle)
            .await
            .map_err(wave1_bound_knowledge_error)?;
        Ok(nomifun_agent_contracts::StrictJsonValue(serde_json::json!(
            document
        )))
    }

    async fn write_knowledge(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1KnowledgeWriteRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let target = match (request.handle, request.base, request.rel_path) {
            (Some(handle), None, None) => nomifun_knowledge::WriteTargetSpec::Handle(handle),
            (None, Some(base), Some(rel_path)) => nomifun_knowledge::WriteTargetSpec::Path {
                kb_id: nomifun_common::KnowledgeBaseId::parse(base)
                    .map_err(|error| {
                        Wave1HostPortError::invalid_request(format!(
                            "knowledge base identity is invalid: {error}"
                        ))
                    })?,
                rel_path,
            },
            _ => {
                return Err(Wave1HostPortError::invalid_request(
                    "knowledge/write requires either handle or base plus rel_path",
                ));
            }
        };
        let result = self
            .authorized_knowledge(&context)?
            .write(nomifun_knowledge::AgentKnowledgeWriteRequest {
                target,
                content: request.content,
            })
            .await
            .map_err(wave1_application_error)?;
        serde_json::to_value(result)
            .map(nomifun_agent_contracts::StrictJsonValue)
            .map_err(|error| Wave1HostPortError::unavailable(error.to_string()))
    }

    async fn autogen_knowledge(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1KnowledgeAutogenRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        let service = self.authorized_knowledge(&context)?;
        let resources = service
            .authority()
            .resource_ids_for(nomifun_knowledge::KnowledgeAction::Autogen);
        let [resource_id] = resources.as_slice() else {
            return Err(Wave1HostPortError::new(
                "PRESET_RESOURCE_NOT_BOUND",
                "knowledge/autogen requires exactly one writable Knowledge resource",
            ));
        };
        let result = service
            .autogen(resource_id, request.overwrite_readme)
            .await
            .map_err(wave1_application_error)?;
        serde_json::to_value(result)
            .map(nomifun_agent_contracts::StrictJsonValue)
            .map_err(|error| Wave1HostPortError::unavailable(error.to_string()))
    }

    fn authorized_knowledge(
        &self,
        context: &nomifun_agent_domain_wave1::Wave1HostContext,
    ) -> Result<nomifun_knowledge::AuthorizedKnowledgeService, Wave1HostPortError> {
        let resources = context
            .resource_bindings
            .iter()
            .filter(|binding| {
                binding.resource_kind.as_ref()
                    == nomifun_agent_domain_wave1::KNOWLEDGE_BASE_RESOURCE_KIND
            })
            .map(agent_knowledge_resource)
            .collect::<Result<Vec<_>, _>>()?;
        let authority = nomifun_knowledge::AgentKnowledgeAuthority::new(
            context.principal.principal_id.clone(),
            resources,
        )
        .map_err(wave1_application_error)?;
        Ok(nomifun_knowledge::AuthorizedKnowledgeService::new(
            Arc::clone(&self.knowledge),
            authority,
        ))
    }

    async fn read_memory(
        &self,
        context: nomifun_agent_domain_wave1::Wave1HostContext,
        request: Wave1ProjectMemoryReadRequest,
    ) -> Result<nomifun_agent_contracts::StrictJsonValue, Wave1HostPortError> {
        use nomifun_agent_contracts::{StateKey, VersionString};

        if context.capability_id.as_ref()
            != nomifun_agent_domain_wave1::PROJECT_MEMORY_MODULE_ID
            || context.action_id.as_ref()
                != nomifun_agent_domain_wave1::PROJECT_MEMORY_READ_ACTION_ID
        {
            return Err(Wave1HostPortError::invalid_request(
                "project.memory/read identity does not match the host context",
            ));
        }
        let operation = Wave1MemoryOperation::ProjectWrite;
        let descriptor = context.state.descriptor();
        if descriptor.package_id.as_ref() != operation.package_id()
            || descriptor.mount_id.as_ref() != operation.mount_id()
        {
            return Err(Wave1HostPortError::unavailable(
                "project memory state handle is mounted in a different namespace",
            ));
        }
        let binding = exact_memory_binding(
            &context,
            super::agent_memory_authority::ProductMemoryAction::ProjectRead,
        )?;
        let scope = nomifun_agent_contracts::ScopeKey::from(format!(
            "resource:{}",
            binding.resource_id.as_ref()
        ));
        let current = context
            .state
            .get(&scope, &StateKey::from(MEMORY_STATE_KEY))
            .await
            .map_err(|error| {
                Wave1HostPortError::unavailable(format!(
                    "project memory state could not be read: {error}"
                ))
            })?;
        let mut entries = decode_memory_state(
            current.as_ref(),
            &VersionString::from(MEMORY_STATE_FORMAT_VERSION),
            operation,
            &binding,
        )?;
        let limit = request.limit.unwrap_or(MAX_MEMORY_ENTRIES).min(MAX_MEMORY_ENTRIES);
        if entries.len() > limit {
            entries.drain(..entries.len() - limit);
        }
        Ok(nomifun_agent_contracts::StrictJsonValue(serde_json::json!({
            "resource_id": binding.resource_id,
            "entries": entries,
        })))
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
        let expected_action = nomifun_agent_contracts::ActionId::from(operation.action_id());
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
        // transient Session. The independent memory authority validates the
        // owner, action, resource kind, cardinality, and operation grant.
        let binding = exact_memory_binding(
            &context,
            super::agent_memory_authority::ProductMemoryAction::ProjectWrite,
        )?;

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
        let request_digest = memory_request_digest(operation, &binding, &request_value)?;
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
            let mut entries = decode_memory_state(current.as_ref(), &format, operation, &binding)?;
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

fn agent_knowledge_resource(
    binding: &nomifun_agent_contracts::TypedResourceBinding,
) -> Result<nomifun_knowledge::AgentKnowledgeResource, Wave1HostPortError> {
    let knowledge_base_id = nomifun_common::KnowledgeBaseId::parse(
        binding.resource_id.as_ref().to_owned(),
    )
    .map_err(|error| {
        Wave1HostPortError::invalid_request(format!(
            "knowledge resource identity is invalid: {error}"
        ))
    })?;
    let root = binding
        .typed_parameters
        .get(KNOWLEDGE_ROOT_PARAMETER)
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
    let name = binding
        .typed_parameters
        .get(KNOWLEDGE_NAME_PARAMETER)
        .map(String::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| knowledge_base_id.as_str().to_owned());
    nomifun_knowledge::AgentKnowledgeResource::from_operation_names(
        binding.binding_id.as_ref(),
        binding.owner_id.clone(),
        knowledge_base_id,
        name,
        PathBuf::from(root),
        binding.operations.iter(),
    )
    .map_err(wave1_application_error)
}

fn exact_memory_binding(
    context: &nomifun_agent_domain_wave1::Wave1HostContext,
    action: super::agent_memory_authority::ProductMemoryAction,
) -> Result<nomifun_agent_contracts::TypedResourceBinding, Wave1HostPortError> {
    super::agent_memory_authority::authorize_memory_resource(
        &context.principal.principal_id,
        action,
        &context.resource_bindings,
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
        "action_id": operation.action_id(),
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

fn wave1_search_error(
    error: nomifun_ai_agent::web_search::SearchProviderError,
) -> Wave1HostPortError {
    use nomifun_ai_agent::web_search::SearchProviderErrorKind;
    tracing::warn!(kind = ?error.kind, "Web Research provider failed");
    let (code, message) = match error.kind {
        SearchProviderErrorKind::NotConfigured => (
            "WEB_RESEARCH_NOT_CONFIGURED",
            "Web Research has no configured provider on this host",
        ),
        SearchProviderErrorKind::AmbiguousModel => (
            "WEB_RESEARCH_ROUTE_AMBIGUOUS",
            "Web Research has no unique configured route",
        ),
        SearchProviderErrorKind::Timeout => (
            "WEB_RESEARCH_TIMEOUT",
            "The authorized web search timed out",
        ),
        SearchProviderErrorKind::ResponseTooLarge => (
            "WEB_RESEARCH_RESPONSE_TOO_LARGE",
            "The authorized web search response exceeded its safe limit",
        ),
        SearchProviderErrorKind::InvalidResponse | SearchProviderErrorKind::NoSources => (
            "WEB_RESEARCH_RESPONSE_INVALID",
            "The authorized web search returned no usable sources",
        ),
        SearchProviderErrorKind::Transport | SearchProviderErrorKind::UpstreamRejected => (
            "WEB_RESEARCH_UNAVAILABLE",
            "The authorized web search provider is temporarily unavailable",
        ),
    };
    Wave1HostPortError::new(code, message)
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
    knowledge_service: Arc<nomifun_knowledge::KnowledgeService>,
    search_provider: Option<Arc<dyn nomifun_ai_agent::web_search::SearchProvider>>,
    companion_service: Arc<nomifun_companion::CompanionService>,
    receipt_pool: SqlitePool,
) -> anyhow::Result<Vec<nomifun_agent_kernel::PluginRegistration>> {
    nomifun_agent_domain_wave1::registrations_with_host_port(Arc::new(
        Wave1ApplicationHost::with_companion_memory(
            knowledge_service,
            search_provider,
            companion_service,
            receipt_pool,
        ),
    ))
    .map_err(anyhow::Error::msg)
}

#[cfg(test)]
#[path = "agent_wave1_host_tests.rs"]
mod tests;
