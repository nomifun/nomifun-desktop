//! Trusted, open runtime composition underneath the existing Session owner.
use std::sync::{Arc, Mutex, OnceLock};

use nomifun_ai_agent::runtime_registry::AgentRuntimeFactory;
use nomifun_ai_agent::{
    AgentRuntimeHandle, RuntimeEngineCatalog, RuntimeEngineDescriptor, RuntimeEngineFactory,
};
use nomifun_api_types::{RUNTIME_ENGINE_BINDING_KEY, RuntimeEngineBinding, RuntimeEngineSelector};
use nomifun_common::AppError;
use sha2::{Digest, Sha256};

/// Extensions are registered by the embedding host before router assembly.
/// Neither HTTP requests nor model output can install executable code.
#[derive(Default)]
pub struct RuntimeEngineHost {
    catalog: OnceLock<Arc<RuntimeEngineCatalog>>,
    extensions: Mutex<Vec<(RuntimeEngineDescriptor, RuntimeEngineFactory)>>,
    nomi_factory: OnceLock<AgentRuntimeFactory>,
}

impl RuntimeEngineHost {
    pub fn register(
        &self,
        descriptor: RuntimeEngineDescriptor,
        factory: RuntimeEngineFactory,
    ) -> Result<(), AppError> {
        descriptor.validate()?;
        let mut extensions = self
            .extensions
            .lock()
            .map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict(
                "Runtime registration is closed after host assembly".into(),
            ));
        }
        if extensions.iter().any(|(item, _)| {
            item.family_id == descriptor.family_id && item.build_id == descriptor.build_id
        }) {
            return Err(AppError::Conflict(
                "Runtime build is already registered".into(),
            ));
        }
        extensions.push((descriptor, factory));
        Ok(())
    }

    pub(crate) fn dispatch(self: &Arc<Self>, nomi: AgentRuntimeFactory) -> AgentRuntimeFactory {
        self.nomi_factory
            .set(nomi.clone())
            .unwrap_or_else(|_| panic!("runtime factory installed twice"));
        let host = self.clone();
        Arc::new(move |options| {
            let host = host.clone();
            let legacy = nomi.clone();
            Box::pin(async move {
                match binding_from_extra(&options.extra)? {
                    Some(binding) => host.catalog()?.open(&binding, options).await,
                    // Pre-existing conversations have no engine binding. This
                    // is the explicit legacy Nomi path, never a fallback for
                    // an unavailable or malformed bound implementation.
                    None => legacy(options).await,
                }
            })
        })
    }

    pub(crate) fn install(&self, coding: RuntimeEngineFactory) -> Result<(), AppError> {
        let extensions = self
            .extensions
            .lock()
            .map_err(|_| AppError::Internal("Runtime registration lock poisoned".into()))?;
        if self.catalog.get().is_some() {
            return Err(AppError::Conflict(
                "Runtime catalog already installed".into(),
            ));
        }
        let mut catalog = RuntimeEngineCatalog::default();
        let nomi = self
            .nomi_factory
            .get()
            .ok_or_else(|| AppError::Internal("Nomi factory not installed".into()))?
            .clone();
        let descriptor = nomi_descriptor();
        let build_id = descriptor.build_id.clone();
        catalog.register(
            descriptor,
            Arc::new(move |options, _| {
                let nomi = nomi.clone();
                Box::pin(async move {
                    #[allow(unreachable_patterns)]
                    match nomi(options).await? {
                        AgentRuntimeHandle::Registered(runtime) => Ok(runtime),
                        AgentRuntimeHandle::Nomi(runtime) => {
                            Ok(runtime as Arc<dyn nomifun_ai_agent::RegisteredAgentRuntime>)
                        }
                        _ => Err(AppError::Internal(
                            "Built-in Nomi factory returned a test-only runtime".into(),
                        )),
                    }
                })
            }),
        )?;
        catalog.set_channel("nomifun.nomi", "stable", &build_id)?;
        let descriptor = super::coding_runtime_host::descriptor();
        let build_id = descriptor.build_id.clone();
        catalog.register(descriptor, coding)?;
        catalog.set_channel("nomifun.coding", "stable", &build_id)?;
        for (descriptor, factory) in extensions.iter() {
            catalog.register(descriptor.clone(), factory.clone())?;
        }
        self.catalog
            .set(Arc::new(catalog))
            .map_err(|_| AppError::Conflict("Runtime catalog already installed".into()))
    }

    pub fn catalog(&self) -> Result<&Arc<RuntimeEngineCatalog>, AppError> {
        self.catalog
            .get()
            .ok_or_else(|| AppError::Conflict("Runtime host is not assembled".into()))
    }

    pub(crate) fn default_binding(&self) -> Result<RuntimeEngineBinding, AppError> {
        self.catalog()?.resolve(
            &RuntimeEngineSelector::Channel {
                family_id: "nomifun.nomi".into(),
                channel: "stable".into(),
            },
            "default",
        )
    }
}

pub(crate) fn nomi_descriptor() -> RuntimeEngineDescriptor {
    RuntimeEngineDescriptor {
        family_id: "nomifun.nomi".into(),
        build_id: format!("{}-host1", env!("CARGO_PKG_VERSION")),
        build_digest: format!(
            "{:x}",
            Sha256::digest(
                concat!(
                    include_str!("../../../../../Cargo.lock"),
                    include_str!("../../../nomifun-ai-agent/src/factory/nomi.rs"),
                    include_str!("../../../nomifun-ai-agent/src/runtime_extension.rs")
                )
                .as_bytes()
            )
        ),
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
            let binding: RuntimeEngineBinding =
                serde_json::from_value(value.clone()).map_err(|error| {
                    AppError::Conflict(format!("Invalid persisted runtime binding: {error}"))
                })?;
            binding.validate()?;
            Ok(binding)
        })
        .transpose()
}
