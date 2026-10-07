//! Exact product Module/Action and Resource Binding authority for memory.
//!
//! Project citation/distillation and Companion merge/evolve are owner
//! maintenance derived from legitimate results or writes. Session scratch is
//! a runtime resource. None of those mechanisms is accepted as an Agent
//! authoring entry here.

use nomifun_agent_contracts::TypedResourceBinding;
use nomifun_common::AppError;

pub(crate) const PROJECT_MEMORY_MODULE_ID: &str = "project.memory";
pub(crate) const PROJECT_MEMORY_READ_ACTION_ID: &str = "project.memory/read";
pub(crate) const PROJECT_MEMORY_WRITE_ACTION_ID: &str = "project.memory/write";
pub(crate) const COMPANION_MEMORY_MODULE_ID: &str = "companion.memory";
pub(crate) const COMPANION_MEMORY_RECALL_ACTION_ID: &str = "companion.memory/recall";
pub(crate) const COMPANION_MEMORY_WRITE_ACTION_ID: &str = "companion.memory/write";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProductMemoryAction {
    ProjectRead,
    ProjectWrite,
    CompanionRecall,
    CompanionWrite,
}

impl ProductMemoryAction {
    pub(crate) const fn module_id(self) -> &'static str {
        match self {
            Self::ProjectRead | Self::ProjectWrite => PROJECT_MEMORY_MODULE_ID,
            Self::CompanionRecall | Self::CompanionWrite => COMPANION_MEMORY_MODULE_ID,
        }
    }

    pub(crate) const fn action_id(self) -> &'static str {
        match self {
            Self::ProjectRead => PROJECT_MEMORY_READ_ACTION_ID,
            Self::ProjectWrite => PROJECT_MEMORY_WRITE_ACTION_ID,
            Self::CompanionRecall => COMPANION_MEMORY_RECALL_ACTION_ID,
            Self::CompanionWrite => COMPANION_MEMORY_WRITE_ACTION_ID,
        }
    }

    pub(crate) const fn resource_kind(self) -> &'static str {
        match self {
            Self::ProjectRead | Self::ProjectWrite => "project_memory",
            Self::CompanionRecall | Self::CompanionWrite => "companion_memory",
        }
    }

    pub(crate) const fn resource_operation(self) -> &'static str {
        match self {
            Self::ProjectRead | Self::CompanionRecall => "read",
            Self::ProjectWrite | Self::CompanionWrite => "write",
        }
    }
}

/// Resolve the sole exact resource admitted for a sensitive memory action.
///
/// Capability authorization is checked by Kernel before this function. This
/// is the independent resource/owner half of the decision and therefore must
/// remain fail-closed.
pub(crate) fn authorize_memory_resource(
    principal_id: &str,
    action: ProductMemoryAction,
    resource_bindings: &[TypedResourceBinding],
) -> Result<TypedResourceBinding, AppError> {
    if principal_id.trim().is_empty() {
        return Err(AppError::BadRequest(
            "memory authority principal must not be blank".into(),
        ));
    }
    let matching = resource_bindings
        .iter()
        .filter(|binding| binding.resource_kind.as_ref() == action.resource_kind())
        .collect::<Vec<_>>();
    let binding = match matching.as_slice() {
        [binding] => *binding,
        [] => {
            return Err(AppError::Forbidden(format!(
                "{} has no bound {} resource",
                action.action_id(),
                action.resource_kind()
            )));
        }
        _ => {
            return Err(AppError::Forbidden(format!(
                "{} requires exactly one bound {} resource",
                action.action_id(),
                action.resource_kind()
            )));
        }
    };
    if binding.owner_id != principal_id {
        return Err(AppError::Forbidden(format!(
            "{} resource is owned by a different principal",
            action.module_id()
        )));
    }
    if binding.binding_id.as_ref().trim().is_empty()
        || binding.resource_id.as_ref().trim().is_empty()
    {
        return Err(AppError::BadRequest(
            "memory resource identities must not be blank".into(),
        ));
    }
    if binding
        .operations
        .iter()
        .any(|operation| !matches!(operation.as_str(), "read" | "write"))
    {
        return Err(AppError::BadRequest(
            "memory resource binding contains a non-product operation".into(),
        ));
    }
    if !binding.operations.contains(action.resource_operation()) {
        return Err(AppError::Forbidden(format!(
            "memory resource binding does not grant {} for {}",
            action.resource_operation(),
            action.action_id()
        )));
    }
    Ok(binding.clone())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use nomifun_agent_contracts::{ResourceBindingId, ResourceId, ResourceKind};

    use super::*;

    fn binding(kind: &str, owner: &str, operation: &str) -> TypedResourceBinding {
        TypedResourceBinding {
            binding_id: ResourceBindingId::from(format!("{kind}-binding")),
            resource_kind: ResourceKind::from(kind),
            resource_id: ResourceId::from(format!("{kind}-resource")),
            owner_id: owner.to_owned(),
            operations: BTreeSet::from([operation.to_owned()]),
            connection_config_ref: None,
            typed_parameters: BTreeMap::new(),
        }
    }

    #[test]
    fn authoring_surface_has_only_sensitive_product_actions() {
        assert_eq!(PROJECT_MEMORY_READ_ACTION_ID, "project.memory/read");
        assert_eq!(PROJECT_MEMORY_WRITE_ACTION_ID, "project.memory/write");
        assert_eq!(COMPANION_MEMORY_RECALL_ACTION_ID, "companion.memory/recall");
        assert_eq!(COMPANION_MEMORY_WRITE_ACTION_ID, "companion.memory/write");
    }

    #[test]
    fn authorization_keeps_read_and_write_independent() {
        let read = binding("project_memory", "owner", "read");
        assert!(
            authorize_memory_resource("owner", ProductMemoryAction::ProjectRead, &[read.clone()])
                .is_ok()
        );
        assert!(
            authorize_memory_resource("owner", ProductMemoryAction::ProjectWrite, &[read.clone()])
                .is_err()
        );
        assert!(
            authorize_memory_resource("another", ProductMemoryAction::ProjectRead, &[read])
                .is_err()
        );
    }

    #[test]
    fn authorization_rejects_ambiguous_resources() {
        let first = binding("companion_memory", "owner", "read");
        let mut second = first.clone();
        second.binding_id = ResourceBindingId::from("second-binding");
        second.resource_id = ResourceId::from("second-resource");
        assert!(
            authorize_memory_resource(
                "owner",
                ProductMemoryAction::CompanionRecall,
                &[first, second],
            )
            .is_err()
        );
    }
}
