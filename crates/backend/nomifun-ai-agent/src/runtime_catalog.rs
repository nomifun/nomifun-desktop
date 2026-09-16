//! Exact build identity for the single official Nomi Runtime provider.
//!
//! Production exposes one immutable provider and no family/channel selector.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use nomifun_common::AppError;

use crate::runtime_driver::OFFICIAL_NOMI_RUNTIME_FAMILY_ID;
use crate::runtime_registry::AgentRuntimeFactory;
use crate::types::AgentRuntimeBuildOptions;
use crate::{AgentRuntimeHandle, RegisteredAgentRuntime, RuntimeEngineAdmission};

/// Source-integrated Driver factory used only by the fixed composition root.
pub type RuntimeEngineFactory = Arc<
    dyn Fn(
            AgentRuntimeBuildOptions,
            RuntimeEngineBinding,
        ) -> BoxFuture<'static, Result<Arc<dyn RegisteredAgentRuntime>, AppError>>
        + Send
        + Sync,
>;

pub use nomifun_api_types::{
    RUNTIME_HOST_CONTRACT_VERSION, RuntimeEngineBinding, RuntimeEngineDescriptor,
};

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(format!("Runtime engine contract: {message}"))
}

/// The single source-installed Runtime provider used by production.
#[derive(Clone)]
pub struct NomiRuntimeProvider {
    descriptor: RuntimeEngineDescriptor,
    binding: RuntimeEngineBinding,
    factory: AgentRuntimeFactory,
    admission: Arc<dyn RuntimeEngineAdmission>,
}

impl NomiRuntimeProvider {
    pub fn install(
        descriptor: RuntimeEngineDescriptor,
        factory: AgentRuntimeFactory,
        admission: Arc<dyn RuntimeEngineAdmission>,
    ) -> Result<Self, AppError> {
        descriptor.validate()?;
        if descriptor.family_id != OFFICIAL_NOMI_RUNTIME_FAMILY_ID {
            return Err(invalid("only the official nomifun.nomi family may be installed"));
        }
        if descriptor.supported_profiles != ["default"] {
            return Err(invalid(
                "the official Runtime must expose exactly the default internal profile",
            ));
        }
        let binding = RuntimeEngineBinding {
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

    pub fn descriptor(&self) -> &RuntimeEngineDescriptor {
        &self.descriptor
    }

    pub fn binding(&self) -> RuntimeEngineBinding {
        self.binding.clone()
    }

    pub fn list(&self) -> Vec<RuntimeEngineDescriptor> {
        vec![self.descriptor.clone()]
    }

    pub fn validate_binding(&self, binding: &RuntimeEngineBinding) -> Result<(), AppError> {
        binding.validate()?;
        if binding != &self.binding {
            return Err(invalid(
                "bound Runtime identity is not the installed official build",
            ));
        }
        Ok(())
    }

    pub fn uses_private_session_codec(
        &self,
        binding: &RuntimeEngineBinding,
    ) -> Result<bool, AppError> {
        binding.validate()?;
        if self.validate_binding(binding).is_err() {
            return Ok(false);
        }
        Ok(self.admission.uses_nomi_session(binding))
    }

    pub fn uses_platform_history_context(
        &self,
        binding: &RuntimeEngineBinding,
    ) -> Result<bool, AppError> {
        self.validate_binding(binding)?;
        Ok(self.admission.uses_platform_history_context(binding))
    }

    pub fn validate_snapshot(
        &self,
        binding: &RuntimeEngineBinding,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        self.validate_binding(binding)?;
        if !self.admission.supports_tool_hooks(binding)
            && snapshot.content.contributions().any(|capability| {
                capability.actions.iter().any(|action| {
                    nomifun_agent_contracts::tool_middleware::is_tool_hook(&action.action_id)
                })
            })
        {
            return Err(invalid(
                "the official Runtime build does not support Product tool hooks",
            ));
        }
        self.admission.validate_snapshot(binding, snapshot)
    }

    pub fn validate_session_extra(
        &self,
        binding: &RuntimeEngineBinding,
        extra: &serde_json::Value,
    ) -> Result<(), AppError> {
        self.validate_binding(binding)?;
        self.admission.validate_session_extra(binding, extra)
    }

    pub async fn open(
        &self,
        binding: &RuntimeEngineBinding,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        self.validate_binding(binding)?;
        self.admission.validate_session_extra(binding, &options.extra)?;
        (self.factory)(options).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn descriptor() -> RuntimeEngineDescriptor {
        RuntimeEngineDescriptor {
            family_id: OFFICIAL_NOMI_RUNTIME_FAMILY_ID.to_owned(),
            build_id: "official-build-1".to_owned(),
            build_digest: "f".repeat(64),
            display_name: "Nomi".to_owned(),
            host_contract_version: RUNTIME_HOST_CONTRACT_VERSION,
            supported_profiles: vec!["default".to_owned()],
        }
    }

    fn admission() -> Arc<dyn RuntimeEngineAdmission> {
        Arc::new(crate::RuntimeEngineSupport::platform())
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

    #[test]
    fn rejects_non_official_family_and_multiple_profiles() {
        let never: AgentRuntimeFactory = Arc::new(|_| {
            Box::pin(async {
                Err::<AgentRuntimeHandle, AppError>(AppError::Internal(
                    "rejected provider must not open".into(),
                ))
            })
        });
        let mut foreign = descriptor();
        foreign.family_id = "community.runtime".into();
        assert!(NomiRuntimeProvider::install(foreign, never.clone(), admission()).is_err());

        let mut profiles = descriptor();
        profiles.supported_profiles.push("coding".into());
        assert!(NomiRuntimeProvider::install(profiles, never, admission()).is_err());
    }

    #[tokio::test]
    async fn freezes_one_exact_build_and_one_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let factory: AgentRuntimeFactory = Arc::new({
            let calls = calls.clone();
            move |_| {
                let calls = calls.clone();
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::AcqRel);
                    Err(AppError::Conflict("fake official factory".into()))
                })
            }
        });
        let provider =
            NomiRuntimeProvider::install(descriptor(), factory, admission()).unwrap();
        assert_eq!(provider.list(), vec![descriptor()]);
        let binding = provider.binding();
        provider.validate_binding(&binding).unwrap();
        for mutation in ["family", "build", "digest", "profile"] {
            let mut changed = binding.clone();
            match mutation {
                "family" => changed.family_id = "community.runtime".into(),
                "build" => changed.build_id = "other".into(),
                "digest" => changed.build_digest = "e".repeat(64),
                _ => changed.profile = "coding".into(),
            }
            assert!(provider.validate_binding(&changed).is_err(), "{mutation}");
        }
        assert!(provider.open(&binding, options()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }
}
