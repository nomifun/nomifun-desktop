//! Composition owner for the one source-installed Agent Runtime.

use std::sync::{Arc, OnceLock};

use nomifun_ai_agent::runtime_sessions::OfficialRuntimeOpener;
use nomifun_ai_agent::{
    OfficialRuntimeFactory, OfficialRuntimeProvider, RuntimeAdmission, RuntimeBuildBinding,
    RuntimeBuildDescriptor, RuntimeSupport,
};
use nomifun_common::AppError;
use sha2::{Digest, Sha256};

#[derive(Default)]
pub(crate) struct OfficialRuntimeHost {
    provider: OnceLock<Arc<OfficialRuntimeProvider>>,
}

impl OfficialRuntimeHost {
    pub(crate) fn install(
        &self,
        factory: OfficialRuntimeFactory,
    ) -> Result<(), AppError> {
        let provider = OfficialRuntimeProvider::install(
            descriptor(),
            factory,
            Arc::new(OfficialAdmission),
        )?;
        self.provider
            .set(Arc::new(provider))
            .map_err(|_| AppError::Conflict("official Runtime installed twice".into()))
    }

    /// The Session lifecycle owner receives one fixed factory. The closure
    /// resolves the installed provider at call time because router assembly
    /// installs typed host ports after AppServices is created.
    pub(crate) fn factory(self: &Arc<Self>) -> OfficialRuntimeOpener {
        let host = Arc::clone(self);
        Arc::new(move |options| {
            let host = Arc::clone(&host);
            Box::pin(async move { host.provider()?.open(options).await })
        })
    }

    pub(crate) fn provider(&self) -> Result<&Arc<OfficialRuntimeProvider>, AppError> {
        self.provider
            .get()
            .ok_or_else(|| AppError::Conflict("official Runtime is not assembled".into()))
    }

    pub(crate) fn binding(&self) -> Result<RuntimeBuildBinding, AppError> {
        Ok(self.provider()?.binding())
    }

    pub(crate) fn validate_agent(
        &self,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<RuntimeBuildBinding, AppError> {
        self.provider()?.validate_snapshot(snapshot)?;
        self.binding()
    }
}

struct OfficialAdmission;

impl RuntimeAdmission for OfficialAdmission {
    fn validate_snapshot(
        &self,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        RuntimeSupport::platform().validate_snapshot(snapshot)
    }

    fn validate_session_extra(&self, extra: &serde_json::Value) -> Result<(), AppError> {
        RuntimeSupport::platform().validate_session_extra(extra)
    }
}

pub(crate) fn descriptor() -> RuntimeBuildDescriptor {
    let implementation = super::unified_runtime_host::descriptor();
    let digest_input = format!(
        "uarc-official-driver-v2\n{}\n{}\n{}\n{}\n{}",
        implementation.build_digest,
        include_str!("../../../nomifun-ai-agent/src/runtime_driver.rs"),
        include_str!("../../../nomifun-ai-agent/src/runtime_provider.rs"),
        include_str!("engine_session_host.rs"),
        include_str!("engine_journal.rs"),
    );
    RuntimeBuildDescriptor {
        family_id: nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID.into(),
        build_id: format!("{}-official2", env!("CARGO_PKG_VERSION")),
        build_digest: format!("{:x}", Sha256::digest(digest_input.as_bytes())),
        display_name: "Nomi".into(),
        host_contract_version: nomifun_api_types::RUNTIME_HOST_CONTRACT_VERSION,
        supported_profiles: vec!["default".into()],
    }
}

/// Decode the host-written build identity carried by the canonical Session.
/// This value is diagnostic/recovery evidence only; clients and Presets never
/// select a Runtime implementation.
pub(crate) fn binding_from_extra(
    extra: &serde_json::Value,
) -> Result<Option<RuntimeBuildBinding>, AppError> {
    extra
        .get(nomifun_api_types::RUNTIME_BUILD_BINDING_KEY)
        .map(|value| {
            let binding: RuntimeBuildBinding = serde_json::from_value(value.clone())
                .map_err(|error| AppError::Conflict(format!("Invalid Runtime build binding: {error}")))?;
            binding.validate()?;
            Ok(binding)
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn options() -> nomifun_ai_agent::types::AgentRuntimeBuildOptions {
        nomifun_ai_agent::types::AgentRuntimeBuildOptions {
            user_id: "0190f5fe-7c00-7a00-8000-000000000001".into(),
            conversation_id: "0190f5fe-7c00-7a00-8000-000000000002".into(),
            agent_type: nomifun_common::AgentType::Nomi,
            workspace: std::env::temp_dir().to_string_lossy().into_owned(),
            model: None,
            delegation_policy: Default::default(),
            extra: serde_json::json!({}),
            conversation_created_at: None,
            workspace_binding_lease: None,
            device_mcp_servers: Vec::new(),
        }
    }

    #[tokio::test]
    async fn composition_exposes_exactly_one_factory() {
        let calls = Arc::new(AtomicUsize::new(0));
        let driver: OfficialRuntimeFactory = Arc::new({
            let calls = Arc::clone(&calls);
            move |_, binding| {
                let calls = Arc::clone(&calls);
                Box::pin(async move {
                    assert_eq!(
                        binding.family_id,
                        nomifun_ai_agent::OFFICIAL_NOMI_RUNTIME_FAMILY_ID
                    );
                    calls.fetch_add(1, Ordering::AcqRel);
                    Err(AppError::Conflict("fake official Driver reached".into()))
                })
            }
        });
        let host = Arc::new(OfficialRuntimeHost::default());
        let factory = host.factory();
        assert!(factory(options()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 0);

        host.install(driver).unwrap();
        assert_eq!(host.provider().unwrap().descriptor().family_id, "nomifun.nomi");
        assert!(factory(options()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
        assert!(host.install(Arc::new(|_, _| Box::pin(async { unreachable!() }))).is_err());
    }
}
