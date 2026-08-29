use std::collections::BTreeSet;

use nomifun_agent_contracts::{
    CapabilityId, DigestHex, FullAutoExecutionWire, RuntimeCreateParams, RuntimeFeatureId,
    RuntimeHelloPayload, RuntimeProfileKind, TypedResourceBindings, VersionString,
};
use serde::{Deserialize, Serialize};

use crate::error::RuntimeError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedRuntimeProfile {
    pub kind: RuntimeProfileKind,
    pub runtime_protocol_version: VersionString,
    pub profile_digest: DigestHex,
    pub enabled_runtime_features: BTreeSet<RuntimeFeatureId>,
    pub initial_capabilities: BTreeSet<CapabilityId>,
    pub on_demand_capabilities: BTreeSet<CapabilityId>,
    pub typed_resource_bindings: TypedResourceBindings,
}

impl PinnedRuntimeProfile {
    pub fn full_auto(&self) -> FullAutoExecutionWire {
        FullAutoExecutionWire::fixed()
    }

    pub fn launch_policy(&self) -> RuntimeProfileLaunchPolicy {
        match self.kind {
            RuntimeProfileKind::CodingNative => RuntimeProfileLaunchPolicy::coding_native(),
            RuntimeProfileKind::ManagedMinimal => RuntimeProfileLaunchPolicy::managed_minimal(),
        }
    }

    pub fn validate_hello(&self, hello: &RuntimeHelloPayload) -> Result<(), RuntimeError> {
        if !hello.supported_profiles.contains(&self.kind) {
            return Err(RuntimeError::HelloRejected(format!(
                "runtime does not support profile {:?}",
                self.kind
            )));
        }
        if hello.protocol_version != self.runtime_protocol_version {
            return Err(RuntimeError::HelloRejected(
                "runtime profile protocol version does not match hello".to_owned(),
            ));
        }
        if !self
            .enabled_runtime_features
            .is_subset(&hello.native_features)
        {
            return Err(RuntimeError::HelloRejected(
                "runtime profile requires native features absent from hello".to_owned(),
            ));
        }
        if hello.full_auto != FullAutoExecutionWire::fixed() {
            return Err(RuntimeError::HelloRejected(
                "runtime hello is not fixed FullAuto".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn validate_create(&self, params: &RuntimeCreateParams) -> Result<(), RuntimeError> {
        if params.profile_kind != self.kind
            || params.context.runtime_profile_digest != self.profile_digest
            || params.initial_capabilities != self.initial_capabilities
            || params.on_demand_capabilities != self.on_demand_capabilities
            || params.typed_resource_bindings != self.typed_resource_bindings
        {
            return Err(RuntimeError::Protocol(
                "create params do not match the compiled runtime profile".to_owned(),
            ));
        }
        if params.full_auto != FullAutoExecutionWire::fixed() {
            return Err(RuntimeError::Protocol(
                "runtime create must use the fixed FullAuto wire".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeProfileLaunchPolicy {
    pub codex_coding_base_instructions: bool,
    pub builtin_coding_tools: bool,
    pub workspace_discovery: bool,
    pub agents_instructions: bool,
    pub tool_search: bool,
    pub code_mode: bool,
    pub review_workflow: bool,
    pub subagents: bool,
}

impl RuntimeProfileLaunchPolicy {
    pub const fn coding_native() -> Self {
        Self {
            codex_coding_base_instructions: true,
            builtin_coding_tools: true,
            workspace_discovery: true,
            agents_instructions: true,
            tool_search: true,
            code_mode: true,
            review_workflow: true,
            subagents: true,
        }
    }

    pub const fn managed_minimal() -> Self {
        Self {
            codex_coding_base_instructions: false,
            builtin_coding_tools: false,
            workspace_discovery: false,
            agents_instructions: false,
            tool_search: false,
            code_mode: false,
            review_workflow: false,
            subagents: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_minimal_has_no_hidden_coding_surface() {
        let policy = RuntimeProfileLaunchPolicy::managed_minimal();
        assert!(!policy.codex_coding_base_instructions);
        assert!(!policy.builtin_coding_tools);
        assert!(!policy.workspace_discovery);
        assert!(!policy.agents_instructions);
        assert!(!policy.tool_search);
        assert!(!policy.code_mode);
        assert!(!policy.review_workflow);
        assert!(!policy.subagents);
    }

    #[test]
    fn coding_native_retains_native_review_and_process_surface() {
        let policy = RuntimeProfileLaunchPolicy::coding_native();
        assert!(policy.builtin_coding_tools);
        assert!(policy.review_workflow);
        assert!(policy.subagents);
    }
}
