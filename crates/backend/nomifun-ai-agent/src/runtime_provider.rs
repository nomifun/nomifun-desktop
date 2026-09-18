//! Exact build identity and factory for the one official Agent Runtime.
//!
//! This is deliberately a fixed provider, not a catalog: application
//! composition installs one source-integrated factory and it cannot be
//! selected, replaced, or extended at runtime.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use nomifun_common::AppError;

use crate::runtime_driver::OFFICIAL_NOMI_RUNTIME_FAMILY_ID;
use crate::types::AgentRuntimeBuildOptions;
use crate::{AgentRuntimeHandle, OfficialAgentRuntime, RuntimeAdmission};

pub use nomifun_api_types::{
    RUNTIME_HOST_CONTRACT_VERSION, RuntimeBuildBinding, RuntimeBuildDescriptor,
};

/// Source-integrated factory for the current official build.
pub type OfficialRuntimeFactory = Arc<
    dyn Fn(
            AgentRuntimeBuildOptions,
            RuntimeBuildBinding,
        ) -> BoxFuture<'static, Result<Arc<dyn OfficialAgentRuntime>, AppError>>
        + Send
        + Sync,
>;

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(format!("Official Runtime contract: {message}"))
}

#[derive(Clone)]
pub struct OfficialRuntimeProvider {
    descriptor: RuntimeBuildDescriptor,
    binding: RuntimeBuildBinding,
    factory: OfficialRuntimeFactory,
    admission: Arc<dyn RuntimeAdmission>,
}

impl OfficialRuntimeProvider {
    pub fn install(
        descriptor: RuntimeBuildDescriptor,
        factory: OfficialRuntimeFactory,
        admission: Arc<dyn RuntimeAdmission>,
    ) -> Result<Self, AppError> {
        descriptor.validate()?;
        if descriptor.family_id != OFFICIAL_NOMI_RUNTIME_FAMILY_ID {
            return Err(invalid("only the fixed nomifun.nomi family may be installed"));
        }
        if descriptor.supported_profiles != ["default"] {
            return Err(invalid(
                "the official build must expose exactly the default internal profile",
            ));
        }
        let binding = RuntimeBuildBinding {
            family_id: descriptor.family_id.clone(),
            build_id: descriptor.build_id.clone(),
            build_digest: descriptor.build_digest.clone(),
            host_contract_version: descriptor.host_contract_version,
            profile: "default".to_owned(),
        };
        binding.validate()?;
        Ok(Self {
            descriptor,
            binding,
            factory,
            admission,
        })
    }

    pub fn descriptor(&self) -> &RuntimeBuildDescriptor {
        &self.descriptor
    }

    pub fn binding(&self) -> RuntimeBuildBinding {
        self.binding.clone()
    }

    pub fn validate_binding(&self, binding: &RuntimeBuildBinding) -> Result<(), AppError> {
        binding.validate()?;
        if binding != &self.binding {
            return Err(invalid(
                "Session build identity is not the installed official build",
            ));
        }
        Ok(())
    }

    pub fn validate_snapshot(
        &self,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        if !self.admission.supports_tool_hooks()
            && snapshot.content.contributions().any(|capability| {
                capability.actions.iter().any(|action| {
                    nomifun_agent_contracts::tool_middleware::is_tool_hook(&action.action_id)
                })
            })
        {
            return Err(invalid(
                "the official build does not support Product tool hooks",
            ));
        }
        self.admission.validate_snapshot(snapshot)
    }

    pub fn validate_session_extra(&self, extra: &serde_json::Value) -> Result<(), AppError> {
        self.admission.validate_session_extra(extra)
    }

    pub async fn open(
        &self,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        self.admission.validate_session_extra(&options.extra)?;
        (self.factory)(options, self.binding())
            .await
            .map(AgentRuntimeHandle::official)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn descriptor() -> RuntimeBuildDescriptor {
        RuntimeBuildDescriptor {
            family_id: OFFICIAL_NOMI_RUNTIME_FAMILY_ID.to_owned(),
            build_id: "official-build-1".to_owned(),
            build_digest: "f".repeat(64),
            display_name: "Nomi".to_owned(),
            host_contract_version: RUNTIME_HOST_CONTRACT_VERSION,
            supported_profiles: vec!["default".to_owned()],
        }
    }

    fn options() -> AgentRuntimeBuildOptions {
        AgentRuntimeBuildOptions {
            user_id: nomifun_common::generate_id(),
            conversation_id: nomifun_common::generate_id(),
            agent_type: nomifun_common::AgentType::Nomi,
            workspace: "workspace".into(),
            model: None,
            delegation_policy: Default::default(),
            device_mcp_servers: Vec::new(),
            extra: serde_json::json!({}),
            conversation_created_at: None,
            workspace_binding_lease: None,
        }
    }

    #[tokio::test]
    async fn freezes_one_exact_build_and_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory: OfficialRuntimeFactory = Arc::new({
            let calls = Arc::clone(&calls);
            move |_, _| {
                let calls = Arc::clone(&calls);
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::AcqRel);
                    Err(AppError::Conflict("fake official factory".into()))
                })
            }
        });
        let provider = OfficialRuntimeProvider::install(
            descriptor(),
            factory,
            Arc::new(crate::RuntimeSupport::platform()),
        )
        .unwrap();
        let binding = provider.binding();
        provider.validate_binding(&binding).unwrap();
        let mut foreign = binding;
        foreign.build_id = "other".into();
        assert!(provider.validate_binding(&foreign).is_err());
        assert!(provider.open(options()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }
}
