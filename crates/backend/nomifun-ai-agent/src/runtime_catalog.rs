//! Exact build identity for the single official Nomi Runtime provider.
//!
//! Production exposes one immutable provider and no family/channel selector.
//! The former catalog is compiled only for migration tests until UARC-052
//! removes old Coding fixtures; it is not available to product composition.

use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use std::collections::BTreeMap;

use futures_util::future::BoxFuture;
use nomifun_common::AppError;

use crate::runtime_registry::AgentRuntimeFactory;
use crate::types::AgentRuntimeBuildOptions;
use crate::{AgentRuntimeHandle, RegisteredAgentRuntime, RuntimeEngineAdmission};
use crate::runtime_driver::OFFICIAL_NOMI_RUNTIME_FAMILY_ID;

/// A host-installed implementation, not a client-supplied executable path.
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
#[cfg(any(test, feature = "test-support"))]
pub use nomifun_api_types::RuntimeEngineSelector;

#[cfg(any(test, feature = "test-support"))]
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b':')
        })
}

#[cfg(any(test, feature = "test-support"))]
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(format!("Runtime engine contract: {message}"))
}

/// The single source-installed Runtime provider used by production.
///
/// The migration-only catalog exists only in tests. This production type
/// cannot register a second build, resolve a channel, or select a family. It
/// freezes one exact `nomifun.nomi` build and one factory when the application
/// composition root is constructed.
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

#[derive(Clone)]
#[cfg(any(test, feature = "test-support"))]
struct Registration {
    descriptor: RuntimeEngineDescriptor,
    factory: RuntimeEngineFactory,
    admission: Arc<dyn RuntimeEngineAdmission>,
}

/// Migration/test-support catalog retained until UARC-052 removes the old
/// Coding fixtures. It is absent from production builds; product composition
/// uses [`NomiRuntimeProvider`] exclusively.
#[derive(Clone, Default)]
#[cfg(any(test, feature = "test-support"))]
pub struct RuntimeEngineCatalog {
    builds: BTreeMap<(String, String), Registration>,
    channels: BTreeMap<(String, String), String>,
}

#[cfg(any(test, feature = "test-support"))]
impl RuntimeEngineCatalog {
    pub fn register(
        &mut self,
        descriptor: RuntimeEngineDescriptor,
        factory: RuntimeEngineFactory,
        admission: Arc<dyn RuntimeEngineAdmission>,
    ) -> Result<(), AppError> {
        descriptor.validate()?;
        let key = (descriptor.family_id.clone(), descriptor.build_id.clone());
        if self.builds.contains_key(&key) {
            // Even an identical descriptor must not replace an existing factory.
            return Err(AppError::Conflict(
                "Runtime engine build is already registered".to_owned(),
            ));
        }
        self.builds.insert(
            key,
            Registration {
                descriptor,
                factory,
                admission,
            },
        );
        Ok(())
    }

    /// Declare an alias once during composition. Unlike `set_channel`, this
    /// rejects duplicate declarations, including an identical target.
    pub fn register_channel(
        &mut self,
        family_id: &str,
        channel: &str,
        build_id: &str,
    ) -> Result<(), AppError> {
        if self
            .channels
            .contains_key(&(family_id.to_owned(), channel.to_owned()))
        {
            return Err(AppError::Conflict(
                "Runtime engine channel is already registered".into(),
            ));
        }
        self.set_channel(family_id, channel, build_id)
    }

    pub fn set_channel(
        &mut self,
        family_id: &str,
        channel: &str,
        build_id: &str,
    ) -> Result<(), AppError> {
        if !identifier(family_id) || !identifier(channel) || !identifier(build_id) {
            return Err(invalid("invalid channel target"));
        }
        if !self
            .builds
            .contains_key(&(family_id.to_owned(), build_id.to_owned()))
        {
            return Err(invalid("channel target is not installed"));
        }
        self.channels.insert(
            (family_id.to_owned(), channel.to_owned()),
            build_id.to_owned(),
        );
        Ok(())
    }

    pub fn list(&self) -> Vec<RuntimeEngineDescriptor> {
        self.builds
            .values()
            .map(|entry| entry.descriptor.clone())
            .collect()
    }

    /// Only for new Session/fork admission, never for an existing Session turn.
    pub fn resolve(
        &self,
        selector: &RuntimeEngineSelector,
        profile: &str,
    ) -> Result<RuntimeEngineBinding, AppError> {
        if !identifier(profile) {
            return Err(invalid("invalid profile"));
        }
        let (family, build, expected_digest) = match selector {
            RuntimeEngineSelector::Exact {
                family_id,
                build_id,
                build_digest,
            } => {
                if !identifier(family_id) || !identifier(build_id) || !digest(build_digest) {
                    return Err(invalid("invalid exact selector"));
                }
                (family_id, build_id, Some(build_digest))
            }
            RuntimeEngineSelector::Channel { family_id, channel } => {
                if !identifier(family_id) || !identifier(channel) {
                    return Err(invalid("invalid channel selector"));
                }
                let build = self
                    .channels
                    .get(&(family_id.clone(), channel.clone()))
                    .ok_or_else(|| invalid("selected channel is unavailable"))?;
                (family_id, build, None)
            }
        };
        let registration = self
            .builds
            .get(&(family.clone(), build.clone()))
            .ok_or_else(|| invalid("selected build is unavailable"))?;
        let descriptor = &registration.descriptor;
        if expected_digest.is_some_and(|digest| digest != &descriptor.build_digest) {
            return Err(invalid("selected build digest does not match"));
        }
        let binding = RuntimeEngineBinding {
            family_id: family.clone(),
            build_id: build.clone(),
            build_digest: descriptor.build_digest.clone(),
            host_contract_version: descriptor.host_contract_version,
            profile: profile.to_owned(),
        };
        self.registration(&binding)?;
        Ok(binding)
    }

    fn registration(&self, binding: &RuntimeEngineBinding) -> Result<&Registration, AppError> {
        binding.validate()?;
        let registration = self
            .builds
            .get(&(binding.family_id.clone(), binding.build_id.clone()))
            .ok_or_else(|| {
                invalid("bound build is not bundled; use an application build containing it (Fork preserves the exact binding)")
            })?;
        let descriptor = &registration.descriptor;
        if binding.build_digest != descriptor.build_digest
            || binding.host_contract_version != descriptor.host_contract_version
        {
            return Err(invalid("bound build has changed"));
        }
        if !descriptor.supported_profiles.contains(&binding.profile) {
            return Err(invalid("bound profile is unsupported"));
        }
        if registration.admission.uses_platform_history_context(binding)
            && registration.admission.uses_nomi_session(binding) {
            return Err(invalid("private Nomi and platform-only context policies conflict"));
        }
        Ok(registration)
    }

    /// Validate a restored binding without constructing or executing a runtime.
    pub fn validate_binding(&self, binding: &RuntimeEngineBinding) -> Result<(), AppError> {
        self.registration(binding).map(|_| ())
    }

    /// Private codec eligibility is an exact, source-registered policy. A
    /// missing/drifted build has no Nomi authority; boot recovery may still
    /// consult its separately registered exact-build recovery hook. This is
    /// not runnable-build admission: `open` continues to reject such bindings.
    pub fn uses_nomi_session(&self, binding: &RuntimeEngineBinding) -> Result<bool, AppError> {
        binding.validate()?;
        Ok(match self.registration(binding) {
            Ok(registration) => registration.admission.uses_nomi_session(binding),
            Err(_) => false,
        })
    }

    /// Unlike private-codec routing this is runnable-build maintenance: a
    /// missing build must fail, not claim that a historical codec was cleared.
    pub fn uses_platform_history_context(&self, binding: &RuntimeEngineBinding) -> Result<bool, AppError> {
        let registration = self.registration(binding)?;
        Ok(registration.admission.uses_platform_history_context(binding))
    }

    pub fn validate_snapshot(
        &self,
        binding: &RuntimeEngineBinding,
        snapshot: &nomifun_agent_contracts::ResolvedSnapshotEnvelope,
    ) -> Result<(), AppError> {
        let registration = self.registration(binding)?;
        if !registration.admission.supports_tool_hooks(binding)
            && snapshot.content.contributions().any(|capability| capability.actions.iter()
                .any(|action| nomifun_agent_contracts::tool_middleware::is_tool_hook(&action.action_id))) {
            return Err(invalid("The selected Engine does not support Product tool hooks; choose Nomi or remove the hook"));
        }
        registration.admission.validate_snapshot(binding, snapshot)
    }

    pub fn validate_session_extra(
        &self,
        binding: &RuntimeEngineBinding,
        extra: &serde_json::Value,
    ) -> Result<(), AppError> {
        self.registration(binding)?.admission.validate_session_extra(binding, extra)
    }

    /// Called inside the existing runtime registry's guarded factory future.
    /// The caller supplies the authoritative Session options and exact binding.
    pub async fn open(
        &self,
        binding: &RuntimeEngineBinding,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        let registration = self.registration(binding)?;
        registration.admission.validate_session_extra(binding, &options.extra)?;
        (registration.factory)(options, binding.clone())
            .await
            .map(AgentRuntimeHandle::Registered)
    }

    /// Adapt an exact binding to the existing registry for a single-engine host.
    /// Multi-engine hosts instead call `open` after reading each Session's
    /// persisted binding. This function intentionally has no mutable default.
    pub fn bound_factory(
        self: &Arc<Self>,
        binding: RuntimeEngineBinding,
    ) -> Result<AgentRuntimeFactory, AppError> {
        self.validate_binding(&binding)?;
        let catalog = Arc::clone(self);
        Ok(Arc::new(move |options| {
            let catalog = Arc::clone(&catalog);
            let binding = binding.clone();
            Box::pin(async move { catalog.open(&binding, options).await })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn official_descriptor() -> RuntimeEngineDescriptor {
        RuntimeEngineDescriptor {
            family_id: OFFICIAL_NOMI_RUNTIME_FAMILY_ID.to_owned(),
            build_id: "official-build-1".to_owned(),
            build_digest: "f".repeat(64),
            display_name: "Nomi".to_owned(),
            host_contract_version: RUNTIME_HOST_CONTRACT_VERSION,
            supported_profiles: vec!["default".to_owned()],
        }
    }

    fn build_options() -> AgentRuntimeBuildOptions {
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
    fn production_provider_rejects_non_official_family_and_multiple_profiles() {
        let never: AgentRuntimeFactory = Arc::new(|_| {
            Box::pin(async {
                Err::<AgentRuntimeHandle, AppError>(AppError::Internal(
                    "rejected provider must not open".into(),
                ))
            })
        });
        let mut descriptor = official_descriptor();
        descriptor.family_id = "community.runtime".into();
        assert!(NomiRuntimeProvider::install(
            descriptor,
            never.clone(),
            admission(),
        )
        .is_err());

        let mut descriptor = official_descriptor();
        descriptor.supported_profiles.push("coding".into());
        assert!(NomiRuntimeProvider::install(descriptor, never, admission()).is_err());
    }

    #[tokio::test]
    async fn production_provider_freezes_one_exact_build_and_one_factory() {
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
        let provider = NomiRuntimeProvider::install(
            official_descriptor(),
            factory,
            admission(),
        )
        .unwrap();
        assert_eq!(provider.list(), vec![official_descriptor()]);
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
        assert!(provider.open(&binding, build_options()).await.is_err());
        assert_eq!(calls.load(Ordering::Acquire), 1);
    }

    fn descriptor(family: &str, build: &str) -> RuntimeEngineDescriptor {
        RuntimeEngineDescriptor {
            family_id: family.to_owned(),
            build_id: build.to_owned(),
            build_digest: "a".repeat(64),
            display_name: family.to_owned(),
            host_contract_version: RUNTIME_HOST_CONTRACT_VERSION,
            supported_profiles: vec!["custom.workflow".to_owned()],
        }
    }

    fn never_open() -> RuntimeEngineFactory {
        Arc::new(|_, _| Box::pin(async { panic!("resolution must not construct a runtime") }))
    }

    fn admission() -> Arc<dyn RuntimeEngineAdmission> {
        Arc::new(crate::RuntimeEngineSupport::platform())
    }

    fn channel(family: &str) -> RuntimeEngineSelector {
        RuntimeEngineSelector::Channel {
            family_id: family.to_owned(),
            channel: "user.preview".to_owned(),
        }
    }

    #[test]
    fn migration_only_catalog_can_compile_old_fixture_families() {
        let mut catalog = RuntimeEngineCatalog::default();
        for family in ["example.local", "example.remote", "example.workflow"] {
            catalog
                .register(descriptor(family, "build-1"), never_open(), admission())
                .unwrap();
            catalog
                .set_channel(family, "user.preview", "build-1")
                .unwrap();
            let binding = catalog
                .resolve(&channel(family), "custom.workflow")
                .unwrap();
            assert_eq!(binding.family_id, family);
        }
        assert_eq!(catalog.list().len(), 3);
    }

    #[test]
    fn channel_changes_only_affect_new_bindings() {
        let mut catalog = RuntimeEngineCatalog::default();
        for build in ["one", "two"] {
            catalog
                .register(descriptor("custom", build), never_open(), admission())
                .unwrap();
        }
        catalog
            .set_channel("custom", "user.preview", "one")
            .unwrap();
        let original = catalog
            .resolve(&channel("custom"), "custom.workflow")
            .unwrap();
        catalog
            .set_channel("custom", "user.preview", "two")
            .unwrap();
        assert_eq!(original.build_id, "one");
        catalog.validate_binding(&original).unwrap();
        assert_eq!(
            catalog
                .resolve(&channel("custom"), "custom.workflow")
                .unwrap()
                .build_id,
            "two"
        );
    }

    #[test]
    fn registration_is_immutable_and_rejects_incompatible_contracts() {
        let mut catalog = RuntimeEngineCatalog::default();
        let original = descriptor("custom", "one");
        catalog.register(original.clone(), never_open(), admission()).unwrap();
        assert!(catalog.register(original.clone(), never_open(), admission()).is_err());
        let mut changed = original.clone();
        changed.build_digest = "b".repeat(64);
        assert!(catalog.register(changed, never_open(), admission()).is_err());
        let mut incompatible = descriptor("other", "one");
        incompatible.host_contract_version += 1;
        assert!(catalog.register(incompatible, never_open(), admission()).is_err());
        assert_eq!(catalog.list(), vec![original]);
    }

    #[test]
    fn restored_bindings_fail_closed_on_missing_build_drift_or_profile() {
        let mut catalog = RuntimeEngineCatalog::default();
        catalog
            .register(descriptor("custom", "one"), never_open(), admission())
            .unwrap();
        let selector = RuntimeEngineSelector::Exact {
            family_id: "custom".to_owned(),
            build_id: "one".to_owned(),
            build_digest: "a".repeat(64),
        };
        let binding = catalog.resolve(&selector, "custom.workflow").unwrap();
        for field in ["family", "build", "digest", "contract", "profile"] {
            let mut changed = binding.clone();
            match field {
                "family" => changed.family_id = "missing".to_owned(),
                "build" => changed.build_id = "missing".to_owned(),
                "digest" => changed.build_digest = "b".repeat(64),
                "contract" => changed.host_contract_version += 1,
                _ => changed.profile = "unsupported".to_owned(),
            }
            assert!(catalog.validate_binding(&changed).is_err(), "{field}");
        }
        assert!(catalog.resolve(&selector, "coding").is_err());
        assert!(catalog.set_channel("custom", "stable", "missing").is_err());
        let restored: RuntimeEngineBinding =
            serde_json::from_value(serde_json::to_value(&binding).unwrap()).unwrap();
        catalog.validate_binding(&restored).unwrap();
    }

    #[test]
    fn malformed_descriptors_and_unknown_binding_fields_are_rejected() {
        let mut invalid = descriptor("custom", "one");
        invalid
            .supported_profiles
            .push("custom.workflow".to_owned());
        assert!(invalid.validate().is_err());
        invalid = descriptor(" ../custom", "one");
        assert!(invalid.validate().is_err());
        invalid = descriptor("custom", "one");
        invalid.build_digest = "A".repeat(64);
        assert!(invalid.validate().is_err());
        assert!(
            serde_json::from_value::<RuntimeEngineBinding>(serde_json::json!({
                "family_id": "custom", "build_id": "one", "build_digest": "a".repeat(64),
                "host_contract_version": 1, "profile": "custom.workflow", "channel": "stable"
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn unsupported_session_overlay_is_rejected_before_factory_execution() {
        let mut catalog = RuntimeEngineCatalog::default();
        catalog.register(descriptor("community.planner", "one"), never_open(),
            Arc::new(crate::RuntimeEngineSupport::enabled_only([]))).unwrap();
        let binding = catalog.resolve(&RuntimeEngineSelector::Exact {
            family_id: "community.planner".into(), build_id: "one".into(), build_digest: "a".repeat(64),
        }, "custom.workflow").unwrap();
        let options = AgentRuntimeBuildOptions {
            user_id: nomifun_common::generate_id(),
            conversation_id: nomifun_common::generate_id(),
            agent_type: nomifun_common::AgentType::Nomi,
            workspace: "workspace".into(), model: None,
            delegation_policy: Default::default(),
            device_mcp_servers: Vec::new(),
            extra: serde_json::json!({"skills":["pdf"]}),
            conversation_created_at: None, workspace_binding_lease: None,
        };
        let error = catalog.open(&binding, options).await.err().expect("admission must reject before opening");
        assert!(error.to_string().contains("community.planner"));
        assert!(error.to_string().contains("session Skills"));
    }
}
