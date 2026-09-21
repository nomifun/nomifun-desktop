//! Orthogonal execution-discipline policy for frozen platform tools.
//!
//! `EngineEffectClass` answers ordering and uncertainty questions. It does not
//! say that every external effect is a long-horizon coding task, nor that every
//! effect mutates the workspace. Keeping those dimensions separate prevents an
//! atomic product action (for example submitting one media-generation job)
//! from activating the planning/completion ledger or invalidating repository
//! observations.

use crate::AgentToolBinding;

/// Tools whose use can grow into a multi-step task that needs source-anchored
/// requirements and completion accounting. Explicit `update_plan`, steering,
/// and task continuation can still activate the ledger independently.
pub(crate) fn requires_task_ledger(binding: &AgentToolBinding) -> bool {
    matches!(
        binding.capability_id.as_ref(),
        "workspace.files"
            | "workspace.vcs"
            | "workspace.process"
            | "workspace.artifacts"
            | "ssh"
            | "agent.collaboration"
            | "requirements"
            | "plugin.development"
    )
}

/// Whether an attempted tool can invalidate observations of the local
/// workspace/repository. External effects such as media generation, Browser,
/// Computer, notifications, devices, and schedules are deliberately excluded.
pub(crate) fn affects_workspace(binding: &AgentToolBinding) -> bool {
    match binding.capability_id.as_ref() {
        // A process can keep mutating after a read-shaped poll, so every
        // process action invalidates observations until cleanup proves idle.
        "workspace.process" => true,
        "workspace.files" | "workspace.vcs" | "workspace.artifacts" => {
            !matches!(
                binding.effect_class,
                nomifun_engine_core::EngineEffectClass::ReadOnly
            )
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        ActionId, CanonicalSchemaRef, CapabilityId, DigestHex, StrictJsonValue,
    };
    use nomifun_chat_model_broker::ChatToolDefinition;
    use nomifun_engine_core::{EngineEffectClass, EngineToolBinding};
    use std::collections::BTreeSet;

    fn binding_with_effect(
        capability: &str,
        action: &str,
        effect_class: EngineEffectClass,
    ) -> EngineToolBinding {
        let definition = ChatToolDefinition {
            name: "tool".into(),
            description: "fixture".into(),
            input_schema: StrictJsonValue(serde_json::json!({"type":"object"})),
            deferred: false,
        };
        EngineToolBinding {
            model_name: definition.name.clone(),
            schema_digest: crate::input_schema_digest(&definition.input_schema).unwrap(),
            canonical_input_schema_ref: CanonicalSchemaRef::from("schema://fixture/input"),
            capability_contract_digest: DigestHex::from("a".repeat(64)),
            definition,
            capability_id: CapabilityId::from(capability),
            action_id: ActionId::from(action),
            resource_binding_ids: BTreeSet::new(),
            effect_class,
            parallel_safe: false,
        }
    }

    fn binding(capability: &str, action: &str) -> EngineToolBinding {
        binding_with_effect(
            capability,
            action,
            EngineEffectClass::ExternalUncertainEffect,
        )
    }

    #[test]
    fn media_submission_is_atomic_and_not_a_workspace_mutation() {
        let media = binding("creation.media", "creation.media/image");
        assert!(!requires_task_ledger(&media));
        assert!(!affects_workspace(&media));
    }

    #[test]
    fn workspace_and_coordination_keep_long_horizon_accounting() {
        let workspace = binding("workspace.files", "workspace.files/write");
        assert!(requires_task_ledger(&workspace));
        assert!(affects_workspace(&workspace));

        let read = binding_with_effect(
            "workspace.files",
            "workspace.files/read",
            EngineEffectClass::ReadOnly,
        );
        assert!(requires_task_ledger(&read));
        assert!(!affects_workspace(&read));

        let process_poll = binding_with_effect(
            "workspace.process",
            "workspace.process/poll",
            EngineEffectClass::ReadOnly,
        );
        assert!(affects_workspace(&process_poll));

        let delegation = binding("agent.collaboration", "agent/delegate");
        assert!(requires_task_ledger(&delegation));
        assert!(!affects_workspace(&delegation));
    }
}
