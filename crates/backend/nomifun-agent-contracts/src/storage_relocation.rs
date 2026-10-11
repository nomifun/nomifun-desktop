//! Current-generation storage location transitions; no historical data import.
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use crate::{AgentSessionId, TypedResourceBinding, resource_definition_id};

pub const MANAGED_WORKSPACE_LOCATION_PARAMETER: &str = "workspace_location";
pub const MANAGED_SESSION_WORKSPACE_LOCATION: &str = "managed-session";
pub const DEFAULT_WORKSPACE_RESOURCE_ID: &str = "default-workspace";
pub const MANAGED_PROCESS_SESSION_RESOURCE_ID: &str = "managed-process-session";

pub fn managed_workspace_parameters() -> BTreeMap<String, String> {
    BTreeMap::from([(MANAGED_WORKSPACE_LOCATION_PARAMETER.into(), MANAGED_SESSION_WORKSPACE_LOCATION.into())])
}

pub fn is_default_managed_workspace(resource: &TypedResourceBinding) -> bool {
    (resource.resource_kind.as_ref() == "workspace" && resource.resource_id.as_ref() == DEFAULT_WORKSPACE_RESOURCE_ID)
        || (resource.resource_kind.as_ref() == "process_session" && resource.resource_id.as_ref() == MANAGED_PROCESS_SESSION_RESOURCE_ID)
}

fn normalized_path(raw: &str) -> String {
    #[cfg(windows)]
    let raw = raw.replace('\\', "/");
    #[cfg(not(windows))]
    let raw = raw.to_owned();
    raw.strip_prefix("//?/").unwrap_or(&raw).trim_end_matches('/').to_owned()
}

fn path_key(raw: &str) -> String {
    let normalized = normalized_path(raw);
    #[cfg(windows)]
    { normalized.to_ascii_lowercase() }
    #[cfg(not(windows))]
    { normalized }
}

pub fn storage_paths_equal(left: &str, right: &str) -> bool { path_key(left) == path_key(right) }

/// Component-bounded relocation; an adjacent path with the same textual prefix
/// is never a managed child. Callers must separately prove product ownership.
pub fn relocate_storage_path(raw: &str, source: &str, target: &str) -> Option<String> {
    let normalized = normalized_path(raw);
    let source = normalized_path(source);
    let key = path_key(raw);
    let source_key = path_key(&source);
    if key == source_key { return Some(target.to_owned()); }
    if !key.starts_with(&format!("{source_key}/")) { return None; }
    let relative = &normalized[source.len() + 1..];
    if relative.split('/').any(|part| matches!(part, "" | "." | "..")) { return None; }
    let mut relocated = Path::new(target).to_path_buf();
    for component in relative.split('/') { relocated.push(component); }
    Some(relocated.to_string_lossy().into_owned())
}

pub fn validate_storage_root(raw: &str) -> Result<(), String> {
    if raw.is_empty() || raw.len() > 4096 || !Path::new(raw).is_absolute()
        || raw != raw.trim() || raw.chars().any(char::is_control)
        || Path::new(raw).components().any(|component| match component {
            std::path::Component::ParentDir | std::path::Component::CurDir => true,
            std::path::Component::Normal(name) => name.to_str().is_none_or(|name| name.trim() != name),
            _ => false,
        })
    { return Err("storage root must be a canonical absolute host path".into()); }
    Ok(())
}

/// The canonical event itself carries the exact before/after definitions.
/// This is bounded provenance, never another active resource binding ledger.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedSessionWorkspaceNamespaceV1 {
    pub agent_session_id: AgentSessionId,
    /// Exact immutable workspace definition from this Session's opening event.
    pub opening_workspace: TypedResourceBinding,
}

impl ManagedSessionWorkspaceNamespaceV1 {
    pub fn validate(&self) -> Result<(), String> {
        if uuid::Uuid::parse_str(self.agent_session_id.as_ref()).ok().is_none_or(|id| id.get_version_num() != 7)
            || self.opening_workspace.resource_kind.as_ref() != "workspace"
            || self.opening_workspace.resource_id.as_ref() != DEFAULT_WORKSPACE_RESOURCE_ID
            || resource_definition_id(&self.opening_workspace).map_err(|error| error.to_string())? != self.opening_workspace.binding_id
        { return Err("managed Session namespace has no exact opening identity".into()); }
        if self.opening_workspace.typed_parameters != managed_workspace_parameters() {
            if self.opening_workspace.typed_parameters.len() != 1 {
                return Err("managed Session namespace opening has mixed workspace authority".into());
            }
            validate_storage_root(self.opening_workspace.typed_parameters.get("workspace_root")
                .ok_or("managed Session namespace opening has no physical root")?)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedStorageRelocationV1 {
    pub source_data_root: String,
    pub source_work_root: String,
    pub target_data_root: String,
    pub managed_knowledge_ids: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_session_namespace: Option<ManagedSessionWorkspaceNamespaceV1>,
    pub previous_resources: Vec<TypedResourceBinding>,
    pub next_resources: Vec<TypedResourceBinding>,
}

impl ManagedStorageRelocationV1 {
    pub fn validate(&self) -> Result<(), String> {
        for root in [&self.source_data_root, &self.source_work_root, &self.target_data_root] { validate_storage_root(root)?; }
        if let Some(namespace) = &self.managed_session_namespace {
            namespace.validate()?;
            if self.previous_resources.iter().chain(&self.next_resources).any(|resource| resource.owner_id != namespace.opening_workspace.owner_id) {
                return Err("managed Session namespace differs from current resource ownership".into());
            }
        }
        if self.previous_resources.len() != self.next_resources.len()
            || self.previous_resources.len() > 128 || self.managed_knowledge_ids.len() > 128
        { return Err("storage transition has no bounded exact resource change".into()); }
        if self.previous_resources == self.next_resources
            && (storage_paths_equal(&self.source_data_root, &self.target_data_root)
                || (self.managed_session_namespace.is_none() && (self.previous_resources.iter().any(|resource| resource.resource_kind.as_ref() == "workspace"
                    && resource.resource_id.as_ref() != DEFAULT_WORKSPACE_RESOURCE_ID)
                || !self.previous_resources.iter().any(|resource| is_default_managed_workspace(resource)
                    && resource.typed_parameters == managed_workspace_parameters()))))
        { return Err("unchanged resources have no exact managed location change".into()); }
        let mut ids = BTreeSet::new();
        let knowledge_source = Path::new(&self.source_data_root).join("knowledge").to_string_lossy().into_owned();
        let knowledge_target = Path::new(&self.target_data_root).join("knowledge").to_string_lossy().into_owned();
        for (previous, next) in self.previous_resources.iter().zip(&self.next_resources) {
            if !ids.insert(previous.binding_id.as_ref()) || previous.resource_kind != next.resource_kind
                || previous.resource_id != next.resource_id || previous.owner_id != next.owner_id
                || previous.operations != next.operations || previous.connection_config_ref != next.connection_config_ref
                || resource_definition_id(previous).map_err(|e| e.to_string())? != previous.binding_id
                || resource_definition_id(next).map_err(|e| e.to_string())? != next.binding_id
            { return Err("storage transition changed resource identity or authority".into()); }
            if previous == next { continue; }
            let mut expected = previous.clone();
            let namespace_source = self.managed_session_namespace.as_ref().map(|namespace|
                Path::new(&self.source_work_root).join("conversations").join(namespace.agent_session_id.as_ref()));
            if matches!(previous.resource_kind.as_ref(), "workspace" | "process_session")
                && namespace_source.as_ref().is_some_and(|source| previous.typed_parameters.get("workspace_root")
                    .is_some_and(|root| storage_paths_equal(root, &source.to_string_lossy()))) {
                let namespace = self.managed_session_namespace.as_ref().expect("matched namespace");
                expected.typed_parameters.insert("workspace_root".into(), Path::new(&self.target_data_root)
                    .join("conversations").join(namespace.agent_session_id.as_ref()).to_string_lossy().into_owned());
            } else if is_default_managed_workspace(previous) {
                if previous.typed_parameters.len() != 1
                    || !previous.typed_parameters.get("workspace_root").is_some_and(|root| storage_paths_equal(root, &self.source_work_root))
                { return Err("managed Workspace has no exact source-root authority".into()); }
                expected.typed_parameters = managed_workspace_parameters();
            } else if previous.resource_kind.as_ref() == "knowledge_base"
                && self.managed_knowledge_ids.contains(previous.resource_id.as_ref()) {
                let source = previous.typed_parameters.get("knowledge_root")
                    .ok_or("managed Knowledge binding has no frozen root")?;
                let exact_source = Path::new(&knowledge_source).join(previous.resource_id.as_ref()).to_string_lossy().into_owned();
                if !storage_paths_equal(source, &exact_source) { return Err("managed Knowledge binding has no exact product-owned base root".into()); }
                let target = relocate_storage_path(source, &knowledge_source, &knowledge_target)
                    .ok_or("managed Knowledge binding is outside its source domain")?;
                expected.typed_parameters.insert("knowledge_root".into(), target);
            } else { return Err("storage transition tried to relocate an external or unsupported resource".into()); }
            expected.binding_id = resource_definition_id(&expected).map_err(|e| e.to_string())?;
            if expected != *next { return Err("storage transition differs from the exact location-only replacement".into()); }
        }
        Ok(())
    }
}
