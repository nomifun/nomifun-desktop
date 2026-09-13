//! Engine-neutral registration and exact build resolution for trusted hosts.
//!
//! The catalog owns implementations, never Session state or permissions. Resolve
//! a selector at Session creation/fork and persist the returned exact binding in
//! the Session owner's transaction. Resume uses `open` with that binding; it
//! must not resolve a channel again. There is no implicit engine fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures_util::future::BoxFuture;
use nomifun_common::AppError;
use serde::{Deserialize, Serialize};

use crate::runtime_registry::AgentRuntimeFactory;
use crate::types::AgentRuntimeBuildOptions;
use crate::{AgentRuntimeHandle, RegisteredAgentRuntime};

pub const RUNTIME_HOST_CONTRACT_VERSION: u32 = 1;

/// A host-installed implementation, not a client-supplied executable path.
pub type RuntimeEngineFactory = Arc<
    dyn Fn(
            AgentRuntimeBuildOptions,
            RuntimeEngineBinding,
        ) -> BoxFuture<'static, Result<Arc<dyn RegisteredAgentRuntime>, AppError>>
        + Send
        + Sync,
>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEngineDescriptor {
    pub family_id: String,
    pub build_id: String,
    pub build_digest: String,
    pub display_name: String,
    pub host_contract_version: u32,
    /// Open identifiers, not an enum tied to the built-in engines.
    pub supported_profiles: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEngineBinding {
    pub family_id: String,
    pub build_id: String,
    pub build_digest: String,
    pub host_contract_version: u32,
    pub profile: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "selection", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeEngineSelector {
    Exact {
        family_id: String,
        build_id: String,
        build_digest: String,
    },
    Channel {
        family_id: String,
        channel: String,
    },
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b':')
        })
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(format!("Runtime engine contract: {message}"))
}

impl RuntimeEngineDescriptor {
    pub fn validate(&self) -> Result<(), AppError> {
        if !identifier(&self.family_id)
            || !identifier(&self.build_id)
            || !digest(&self.build_digest)
        {
            return Err(invalid("invalid family/build identity or SHA-256 digest"));
        }
        if self.display_name.trim().is_empty() || self.display_name.len() > 256 {
            return Err(invalid(
                "display name is required and must be at most 256 bytes",
            ));
        }
        validate_contract_version(self.host_contract_version)?;
        let mut profiles = BTreeSet::new();
        if self.supported_profiles.is_empty()
            || self
                .supported_profiles
                .iter()
                .any(|profile| !identifier(profile) || !profiles.insert(profile))
        {
            return Err(invalid("profiles must be nonempty, valid and unique"));
        }
        Ok(())
    }
}

impl RuntimeEngineBinding {
    pub fn validate(&self) -> Result<(), AppError> {
        if !identifier(&self.family_id)
            || !identifier(&self.build_id)
            || !digest(&self.build_digest)
            || !identifier(&self.profile)
        {
            return Err(invalid("malformed exact binding"));
        }
        validate_contract_version(self.host_contract_version)
    }
}

fn validate_contract_version(version: u32) -> Result<(), AppError> {
    if version != RUNTIME_HOST_CONTRACT_VERSION {
        return Err(invalid("unsupported host contract version"));
    }
    Ok(())
}

struct Registration {
    descriptor: RuntimeEngineDescriptor,
    factory: RuntimeEngineFactory,
}

/// Mutated during trusted host composition, then shared immutably using Arc.
/// Registration does not grant any model, workspace, tool or process authority.
#[derive(Default)]
pub struct RuntimeEngineCatalog {
    builds: BTreeMap<(String, String), Registration>,
    channels: BTreeMap<(String, String), String>,
}

impl RuntimeEngineCatalog {
    pub fn register(
        &mut self,
        descriptor: RuntimeEngineDescriptor,
        factory: RuntimeEngineFactory,
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
            },
        );
        Ok(())
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
                invalid("bound build is unavailable; install it or explicitly fork the Session")
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
        Ok(registration)
    }

    /// Validate a restored binding without constructing or executing a runtime.
    pub fn validate_binding(&self, binding: &RuntimeEngineBinding) -> Result<(), AppError> {
        self.registration(binding).map(|_| ())
    }

    /// Called inside the existing runtime registry's guarded factory future.
    /// The caller supplies the authoritative Session options and exact binding.
    pub async fn open(
        &self,
        binding: &RuntimeEngineBinding,
        options: AgentRuntimeBuildOptions,
    ) -> Result<AgentRuntimeHandle, AppError> {
        let registration = self.registration(binding)?;
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

    fn channel(family: &str) -> RuntimeEngineSelector {
        RuntimeEngineSelector::Channel {
            family_id: family.to_owned(),
            channel: "user.preview".to_owned(),
        }
    }

    #[test]
    fn arbitrary_families_and_profiles_are_discoverable_without_engine_variants() {
        let mut catalog = RuntimeEngineCatalog::default();
        for family in ["example.local", "example.remote", "example.workflow"] {
            catalog
                .register(descriptor(family, "build-1"), never_open())
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
                .register(descriptor("custom", build), never_open())
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
        catalog.register(original.clone(), never_open()).unwrap();
        assert!(catalog.register(original.clone(), never_open()).is_err());
        let mut changed = original.clone();
        changed.build_digest = "b".repeat(64);
        assert!(catalog.register(changed, never_open()).is_err());
        let mut incompatible = descriptor("other", "one");
        incompatible.host_contract_version += 1;
        assert!(catalog.register(incompatible, never_open()).is_err());
        assert_eq!(catalog.list(), vec![original]);
    }

    #[test]
    fn restored_bindings_fail_closed_on_missing_build_drift_or_profile() {
        let mut catalog = RuntimeEngineCatalog::default();
        catalog
            .register(descriptor("custom", "one"), never_open())
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
}
