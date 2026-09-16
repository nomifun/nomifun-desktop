//! One source-installed Nomi Runtime provider underneath the Session owner.
use std::sync::{Arc, OnceLock};

use nomifun_ai_agent::runtime_registry::AgentRuntimeFactory;
use nomifun_ai_agent::{
    AgentRuntimeHandle, NomiRuntimeProvider, RuntimeEngineAdmission, RuntimeEngineDescriptor,
    RuntimeEngineFactory, RuntimeEngineSupport,
};
use nomifun_api_types::{RUNTIME_ENGINE_BINDING_KEY, RuntimeEngineBinding};
use nomifun_common::AppError;
use sha2::{Digest, Sha256};

/// Composition owner for exactly one immutable official Runtime provider.
#[derive(Default)]
pub(crate) struct RuntimeEngineHost {
    provider: OnceLock<Arc<NomiRuntimeProvider>>,
}

impl RuntimeEngineHost {
    /// Install the one source-integrated Driver. The factory already captures
    /// `EngineSessionHost`, so no database, credential, or Domain handler is
    /// exposed through this provider boundary.
    pub(crate) fn install_official_driver(
        &self,
        factory: RuntimeEngineFactory,
    ) -> Result<(), AppError> {
        let descriptor = nomi_descriptor();
        let binding = RuntimeEngineBinding {
            family_id: descriptor.family_id.clone(),
            build_id: descriptor.build_id.clone(),
            build_digest: descriptor.build_digest.clone(),
            host_contract_version: descriptor.host_contract_version,
            profile: "default".into(),
        };
        let factory: AgentRuntimeFactory = Arc::new(move |options| {
            let factory = factory.clone();
            let binding = binding.clone();
            Box::pin(async move {
                factory(options, binding)
                    .await
                    .map(AgentRuntimeHandle::Registered)
            })
        });
        let provider = NomiRuntimeProvider::install(
            descriptor,
            factory,
            Arc::new(NomiAdmission),
        )?;
        self.provider
            .set(Arc::new(provider))
            .map_err(|_| AppError::Conflict("official Runtime factory installed twice".into()))
    }

    /// Retain the existing registry callback shape until UARC-052 deletes that
    /// registry. The captured legacy factory is deliberately unreachable.
    pub(crate) fn dispatch(
        self: &Arc<Self>,
        _retired_nomi_factory: AgentRuntimeFactory,
    ) -> AgentRuntimeFactory {
        let host = self.clone();
        Arc::new(move |options| {
            let host = host.clone();
            Box::pin(async move {
                let provider = Arc::clone(host.provider()?);
                let binding = provider.binding();
                if let Some(persisted) = binding_from_extra(&options.extra)? {
                    provider.validate_binding(&persisted)?;
                }
                provider.open(&binding, options).await
            })
        })
    }

    pub(crate) fn provider(&self) -> Result<&Arc<NomiRuntimeProvider>, AppError> {
        self.provider
            .get()
            .ok_or_else(|| AppError::Conflict("official Runtime provider is not assembled".into()))
    }

    /// Temporary call-site name retained inside the crate until UARC-052
    /// deletes the old runtime registry. It returns the single provider.
    pub(crate) fn catalog(&self) -> Result<&Arc<NomiRuntimeProvider>, AppError> {
        self.provider()
    }

    pub(crate) fn restart_recovery_hooks(
        &self,
    ) -> Result<nomifun_conversation::terminal_proof::RegisteredEngineRecoveryMap, AppError> {
        Ok(Default::default())
    }

    pub(crate) fn default_binding(&self) -> Result<RuntimeEngineBinding, AppError> {
        Ok(self.provider()?.binding())
    }

    pub(crate) fn agent_binding(&self) -> Result<RuntimeEngineBinding, AppError> {
        self.default_binding()
    }

    pub(crate) fn validate_agent(
        &self,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<RuntimeEngineBinding, AppError> {
        let binding = self.agent_binding()?;
        self.provider()?.validate_snapshot(&binding, snapshot)?;
        Ok(binding)
    }
}

/// Platform-only admission for the source-integrated Driver. Capability
/// support comes from the compiled Snapshot and contribution consumers, never
/// from a Runtime-family feature table. This policy also never opts into the
/// retired private Nomi Session codec.
struct NomiAdmission;

impl RuntimeEngineAdmission for NomiAdmission {
    fn supports_tool_hooks(&self, _binding: &RuntimeEngineBinding) -> bool {
        true
    }

    fn uses_platform_history_context(&self, _binding: &RuntimeEngineBinding) -> bool {
        true
    }

    fn validate_snapshot(
        &self,
        binding: &RuntimeEngineBinding,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        RuntimeEngineSupport::platform().validate_snapshot(binding, snapshot)
    }

    fn validate_session_extra(
        &self,
        binding: &RuntimeEngineBinding,
        extra: &serde_json::Value,
    ) -> Result<(), AppError> {
        RuntimeEngineSupport::platform().validate_session_extra(binding, extra)
    }
}

pub(crate) fn nomi_descriptor() -> RuntimeEngineDescriptor {
    let implementation = super::coding_runtime_host::descriptor();
    let digest_input = format!(
        "uarc-nomi-driver-v1\n{}\n{}\n{}\n{}\n{}",
        implementation.build_digest,
        include_str!("../../../nomifun-ai-agent/src/runtime_driver.rs"),
        include_str!("../../../nomifun-ai-agent/src/runtime_catalog.rs"),
        include_str!("engine_session_host.rs"),
        include_str!("engine_journal.rs"),
    );
    RuntimeEngineDescriptor {
        family_id: nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID.into(),
        build_id: format!("{}-uarc-driver1", env!("CARGO_PKG_VERSION")),
        build_digest: format!("{:x}", Sha256::digest(digest_input.as_bytes())),
        display_name: "Nomi".into(),
        host_contract_version: nomifun_api_types::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["default".into()],
    }
}

pub(crate) fn binding_from_extra(
    extra: &serde_json::Value,
) -> Result<Option<RuntimeEngineBinding>, AppError> {
    extra
        .get(RUNTIME_ENGINE_BINDING_KEY)
        .map(|value| {
            let binding: RuntimeEngineBinding = serde_json::from_value(value.clone())
                .map_err(|error| {
                    AppError::Conflict(format!("Invalid persisted runtime binding: {error}"))
                })?;
            binding.validate()?;
            Ok(binding)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn production_composition_has_no_family_selector_or_nomi_handle_branch() {
        let source = include_str!("runtime_engines.rs");
        let composition = source
            .split_once("impl RuntimeEngineHost {")
            .unwrap()
            .1
            .split_once("/// Transitional admission")
            .unwrap()
            .0;
        for retired in [
            ["AgentRuntimeHandle::", "Nomi"].concat(),
            ["RuntimeEngine", "Selector"].concat(),
            ["nomifun", ".coding"].concat(),
            ["register_session", "_hosted"].concat(),
            ["register_", "channel("].concat(),
        ] {
            assert!(
                !composition.contains(&retired),
                "single-provider composition still reaches {retired}"
            );
        }
        assert!(composition.contains("NomiRuntimeProvider::install"));
        assert!(composition.contains("AgentRuntimeHandle::Registered"));
        let state = include_str!("state.rs");
        let host_pos = state.find("EngineSessionHost::new").unwrap();
        let driver_pos = state.find("install_official_driver").unwrap();
        assert!(host_pos < driver_pos, "typed host ports must precede Driver install");
        assert!(!state.contains("register_restart_recovery"));
    }

    fn options(extra: serde_json::Value) -> nomifun_ai_agent::types::AgentRuntimeBuildOptions {
        nomifun_ai_agent::types::AgentRuntimeBuildOptions {
            user_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            agent_type: nomifun_common::AgentType::Nomi,
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
            model: None,
            delegation_policy: Default::default(),
            extra,
            conversation_created_at: None,
            workspace_binding_lease: None,
            device_mcp_servers: Vec::new(),
        }
    }

    fn official_factory(calls: Arc<AtomicUsize>) -> RuntimeEngineFactory {
        Arc::new(move |_, binding| {
            let calls = calls.clone();
            Box::pin(async move {
                assert_eq!(
                    binding.family_id,
                    nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID
                );
                calls.fetch_add(1, Ordering::AcqRel);
                Err::<Arc<dyn nomifun_ai_agent::RegisteredAgentRuntime>, AppError>(
                    AppError::Conflict("fake official Driver reached".into()),
                )
            })
        })
    }

    #[tokio::test]
    async fn composition_exposes_one_official_factory_and_rejects_foreign_binding() {
        let retired_calls = Arc::new(AtomicUsize::new(0));
        let retired: AgentRuntimeFactory = Arc::new({
            let retired_calls = retired_calls.clone();
            move |_| {
                let retired_calls = retired_calls.clone();
                Box::pin(async move {
                    retired_calls.fetch_add(1, Ordering::AcqRel);
                    Err(AppError::Conflict("retired factory reached".into()))
                })
            }
        });
        let official_calls = Arc::new(AtomicUsize::new(0));
        let host = Arc::new(RuntimeEngineHost::default());
        let dispatch = host.dispatch(retired);
        assert!(dispatch(options(serde_json::json!({}))).await.is_err());
        assert_eq!(retired_calls.load(Ordering::Acquire), 0);

        host.install_official_driver(official_factory(official_calls.clone()))
            .unwrap();
        let descriptors = host.provider().unwrap().list();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(
            descriptors[0].family_id,
            nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID
        );
        let installed = host.default_binding().unwrap();
        assert!(
            host.provider()
                .unwrap()
                .uses_platform_history_context(&installed)
                .unwrap()
        );
        assert!(
            !host
                .provider()
                .unwrap()
                .uses_private_session_codec(&installed)
                .unwrap()
        );

        let mut foreign = installed;
        foreign.family_id = ["nomifun", ".coding"].concat();
        let extra = serde_json::json!({ RUNTIME_ENGINE_BINDING_KEY: foreign });
        assert!(dispatch(options(extra)).await.is_err());
        assert_eq!(official_calls.load(Ordering::Acquire), 0);

        let exact = host.default_binding().unwrap();
        let extra = serde_json::json!({ RUNTIME_ENGINE_BINDING_KEY: exact });
        assert!(dispatch(options(extra)).await.is_err());
        assert_eq!(official_calls.load(Ordering::Acquire), 1);
        assert_eq!(retired_calls.load(Ordering::Acquire), 0);
    }
}
