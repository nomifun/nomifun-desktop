use async_trait::async_trait;
use std::sync::Arc;
use nomifun_agent_contracts::{
    DigestHex, MiniAppBridgeCallId, MiniAppServiceReleaseDescriptor,
    MiniAppServiceRuntimeFingerprint, MiniAppServiceStorageDescriptor, MiniAppId,
    MiniAppReleaseRef, ResolvedMiniAppServiceSpec, ResolvedMiniAppServiceSpecInputs,
    StrictJsonValue,
};
use nomifun_js_runtime::CommittedRuntimeProvider;
use crate::{
    InMemoryMiniAppServiceHost, MiniAppCallCancellation, MiniAppPlatformError,
    MiniAppPlatformResult, MiniAppServiceHostPort, MiniAppServiceHostState,
    MiniAppServiceModuleRegistry, MiniAppServiceProcessFactory,
    RuntimeAwareMiniAppServiceProcessFactory,
};

/// All immutable inputs needed to resolve one Service Host run.
///
/// The application service builds this value from the exact persisted Release,
/// Product configuration, and owner-scoped bindings. Runtime selection and
/// process ownership stay behind [`MiniAppServiceRuntimeBinding`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppServiceSpecInput {
    pub miniapp_id: MiniAppId,
    pub release: MiniAppReleaseRef,
    pub active_release_epoch: u64,
    pub descriptor: MiniAppServiceReleaseDescriptor,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: MiniAppServiceStorageDescriptor,
}

impl MiniAppServiceSpecInput {
    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        if self.active_release_epoch == 0 {
            return Err(MiniAppPlatformError::InvalidState(
                "MiniApp Service spec requires a positive Active Release epoch".into(),
            ));
        }
        self.descriptor
            .clone()
            .validate_for_platform()
            .map_err(MiniAppPlatformError::Contract)?;
        self.storage
            .validate_for_platform(&self.miniapp_id)
            .map_err(MiniAppPlatformError::Contract)?;
        for (field, digest) in [
            ("config_schema_digest", &self.config_schema_digest),
            ("config_snapshot_digest", &self.config_snapshot_digest),
            ("credential_slots_digest", &self.credential_slots_digest),
            ("resource_contract_digest", &self.resource_contract_digest),
            ("resource_bindings_digest", &self.resource_bindings_digest),
            ("bridge_contract_digest", &self.bridge_contract_digest),
            ("contribution_set_digest", &self.contribution_set_digest),
        ] {
            validate_digest(digest, field)?;
        }
        Ok(())
    }
}

/// Runtime/process boundary consumed by the MiniApp application service.
///
/// Implementations must hold the committed Runtime lease for every resident
/// process and must bind every invocation to the exact resolved spec.
#[async_trait]
pub trait MiniAppServiceRuntimeBinding: Send + Sync {
    async fn resolve_spec(
        &self,
        input: MiniAppServiceSpecInput,
    ) -> MiniAppPlatformResult<ResolvedMiniAppServiceSpec>;

    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> MiniAppPlatformResult<()>;

    async fn start(
        &self,
        spec: ResolvedMiniAppServiceSpec,
    ) -> MiniAppPlatformResult<()>;

    async fn invoke(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
        call_id: MiniAppBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue>;

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId);

    async fn stop(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()>;

    async fn retry(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()>;

    async fn state(&self, miniapp_id: &MiniAppId) -> Option<MiniAppServiceHostState>;

    async fn maintain(&self, now_ms: i64) -> MiniAppPlatformResult<()>;

    async fn register_module(
        &self,
        miniapp_id: MiniAppId,
        release_digest: DigestHex,
        module_path: std::path::PathBuf,
    ) -> MiniAppPlatformResult<()>;
}

/// Default boundary used by unit tests and compositions that only support
/// UI-only MiniApps. It never starts Node or accepts a Service invocation.
#[derive(Debug, Default)]
pub struct NoopMiniAppServiceRuntime;

#[async_trait]
impl MiniAppServiceRuntimeBinding for NoopMiniAppServiceRuntime {
    async fn resolve_spec(
        &self,
        _input: MiniAppServiceSpecInput,
    ) -> MiniAppPlatformResult<ResolvedMiniAppServiceSpec> {
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Runtime is not configured".into(),
        ))
    }

    async fn bind_active(
        &self,
        _spec: ResolvedMiniAppServiceSpec,
        _enabled: bool,
    ) -> MiniAppPlatformResult<()> {
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Runtime is not configured".into(),
        ))
    }

    async fn start(
        &self,
        _spec: ResolvedMiniAppServiceSpec,
    ) -> MiniAppPlatformResult<()> {
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Runtime is not configured".into(),
        ))
    }

    async fn invoke(
        &self,
        _spec: &ResolvedMiniAppServiceSpec,
        _call_id: MiniAppBridgeCallId,
        _method: String,
        _payload: StrictJsonValue,
        _cancellation: MiniAppCallCancellation,
        _now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        Err(MiniAppPlatformError::ServiceUnavailable(
            "MiniApp Service Runtime is not configured".into(),
        ))
    }

    async fn cancel(&self, _miniapp_id: &MiniAppId, _call_id: &MiniAppBridgeCallId) {}

    async fn stop(&self, _miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn retry(&self, _miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Runtime is not configured".into(),
        ))
    }

    async fn state(&self, _miniapp_id: &MiniAppId) -> Option<MiniAppServiceHostState> {
        None
    }

    async fn maintain(&self, _now_ms: i64) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn register_module(
        &self,
        _miniapp_id: MiniAppId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }
}

/// Production Service runtime binding. It owns the global Service Host
/// coordinator and resolves each process against the committed JavaScript
/// Runtime authority and exact Release module registry.
pub struct ProductionMiniAppServiceRuntimeBinding {
    authority: Arc<dyn CommittedRuntimeProvider>,
    registry: Arc<MiniAppServiceModuleRegistry>,
    host: Arc<InMemoryMiniAppServiceHost>,
}

impl std::fmt::Debug for ProductionMiniAppServiceRuntimeBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionMiniAppServiceRuntimeBinding")
            .field("registry_root", &self.registry.root())
            .finish_non_exhaustive()
    }
}

impl ProductionMiniAppServiceRuntimeBinding {
    pub fn new(
        authority: Arc<dyn CommittedRuntimeProvider>,
        registry: Arc<MiniAppServiceModuleRegistry>,
        max_active_service_hosts: usize,
    ) -> MiniAppPlatformResult<Self> {
        let factory: Arc<dyn MiniAppServiceProcessFactory> = Arc::new(
            RuntimeAwareMiniAppServiceProcessFactory::new(
                Arc::clone(&authority),
                Arc::clone(&registry) as Arc<dyn crate::MiniAppServiceModuleResolver>,
            ),
        );
        let host = Arc::new(InMemoryMiniAppServiceHost::with_capacity(
            factory,
            max_active_service_hosts,
        )?);
        Ok(Self {
            authority,
            registry,
            host,
        })
    }

    pub fn registry(&self) -> &Arc<MiniAppServiceModuleRegistry> {
        &self.registry
    }

    pub fn host(&self) -> &Arc<InMemoryMiniAppServiceHost> {
        &self.host
    }
}

#[async_trait]
impl MiniAppServiceRuntimeBinding for ProductionMiniAppServiceRuntimeBinding {
    async fn resolve_spec(
        &self,
        input: MiniAppServiceSpecInput,
    ) -> MiniAppPlatformResult<ResolvedMiniAppServiceSpec> {
        input.validate()?;
        let runtime = self
            .authority
            .committed_runtime()
            .await
            .map_err(|error| MiniAppPlatformError::Runtime(error.to_string()))?
            .ok_or_else(|| {
                MiniAppPlatformError::ServiceUnavailable(
                    "no committed JavaScript Runtime is selected".into(),
                )
            })?;
        let runtime = MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: runtime.fingerprint.runtime_installation_id,
            runtime_target: runtime.fingerprint.runtime_target,
            runtime_executable_digest: runtime.fingerprint.executable_digest,
            node_version: runtime.fingerprint.node_version,
        };
        ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: input.miniapp_id,
            release: input.release,
            active_release_epoch: input.active_release_epoch,
            service_module_digest: input.descriptor.module_digest.clone(),
            lifecycle: input.descriptor.lifecycle,
            host_protocol_version: input.descriptor.host_protocol_version.clone(),
            sdk_contract_version: input.descriptor.sdk_contract_version.clone(),
            runtime,
            config_schema_digest: input.config_schema_digest,
            config_snapshot_digest: input.config_snapshot_digest,
            credential_slots_digest: input.credential_slots_digest,
            resource_contract_digest: input.resource_contract_digest,
            resource_bindings_digest: input.resource_bindings_digest,
            runtime_requirements_digest: input.descriptor.runtime_requirements_digest.clone(),
            bridge_contract_digest: input.bridge_contract_digest,
            contribution_set_digest: input.contribution_set_digest,
            storage: input.storage,
        })
        .map_err(MiniAppPlatformError::Contract)
    }

    async fn bind_active(
        &self,
        spec: ResolvedMiniAppServiceSpec,
        enabled: bool,
    ) -> MiniAppPlatformResult<()> {
        self.host.bind_active(spec, enabled).await
    }

    async fn start(
        &self,
        spec: ResolvedMiniAppServiceSpec,
    ) -> MiniAppPlatformResult<()> {
        let miniapp_id = spec.miniapp_id.clone();
        self.host.bind_active(spec, true).await?;
        if matches!(
            self.host.state(&miniapp_id).await,
            Some(MiniAppServiceHostState::Running { .. })
        ) {
            return Ok(());
        }
        self.host.retry(&miniapp_id).await
    }

    async fn invoke(
        &self,
        spec: &ResolvedMiniAppServiceSpec,
        call_id: MiniAppBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: MiniAppCallCancellation,
        now_ms: i64,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        self.host
            .invoke(spec, call_id, method, payload, cancellation, now_ms)
            .await
    }

    async fn cancel(&self, miniapp_id: &MiniAppId, call_id: &MiniAppBridgeCallId) {
        self.host.cancel(miniapp_id, call_id).await;
    }

    async fn stop(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        self.host.stop(miniapp_id).await
    }

    async fn retry(&self, miniapp_id: &MiniAppId) -> MiniAppPlatformResult<()> {
        self.host.retry(miniapp_id).await
    }

    async fn state(&self, miniapp_id: &MiniAppId) -> Option<MiniAppServiceHostState> {
        self.host.state(miniapp_id).await
    }

    async fn maintain(&self, now_ms: i64) -> MiniAppPlatformResult<()> {
        self.host
            .reap_idle(now_ms, 60_000)
            .await?;
        self.host
            .reconcile_continuous(now_ms)
            .await
            .map(|_| ())
    }

    async fn register_module(
        &self,
        miniapp_id: MiniAppId,
        release_digest: DigestHex,
        module_path: std::path::PathBuf,
    ) -> MiniAppPlatformResult<()> {
        self.registry
            .register(miniapp_id, release_digest, module_path)
            .await
            .map(|_| ())
    }
}

fn validate_digest(value: &DigestHex, field: &str) -> MiniAppPlatformResult<()> {
    if value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(MiniAppPlatformError::InvalidState(format!(
            "{field} must be a lowercase SHA-256 digest"
        )))
    }
}

// These validation helpers are intentionally kept at the platform boundary.
// The contract types keep their detailed validators private because they are
// also used by schema generation; this adapter needs the same exact checks
// without exposing mutable internals.
trait PlatformServiceDescriptorValidation {
    fn validate_for_platform(&self) -> Result<(), nomifun_agent_contracts::MiniAppM1ContractError>;
}

impl PlatformServiceDescriptorValidation for MiniAppServiceReleaseDescriptor {
    fn validate_for_platform(&self) -> Result<(), nomifun_agent_contracts::MiniAppM1ContractError> {
        if self.entrypoint != "service/main.mjs" {
            return Err(nomifun_agent_contracts::MiniAppM1ContractError::InvalidField {
                field: "service.entrypoint",
                reason: "Service entrypoint must be service/main.mjs".into(),
            });
        }
        if self.module_digest.as_ref().len() != 64
            || !self
                .module_digest
                .as_ref()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(nomifun_agent_contracts::MiniAppM1ContractError::InvalidField {
                field: "service.module_digest",
                reason: "Service module digest is invalid".into(),
            });
        }
        Ok(())
    }
}

trait PlatformStorageValidation {
    fn validate_for_platform(
        &self,
        miniapp_id: &MiniAppId,
    ) -> Result<(), nomifun_agent_contracts::MiniAppM1ContractError>;
}

impl PlatformStorageValidation for MiniAppServiceStorageDescriptor {
    fn validate_for_platform(
        &self,
        miniapp_id: &MiniAppId,
    ) -> Result<(), nomifun_agent_contracts::MiniAppM1ContractError> {
        if self.kv.miniapp_id != *miniapp_id
            || self
                .files_dir
                .as_ref()
                .is_some_and(|value| value.miniapp_id != *miniapp_id)
            || self
                .private_database
                .as_ref()
                .is_some_and(|value| value.miniapp_id != *miniapp_id)
        {
            return Err(nomifun_agent_contracts::MiniAppM1ContractError::InvalidField {
                field: "storage.miniapp_id",
                reason: "all Service storage handles must belong to the exact MiniApp".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{
        MiniAppKvHandleDescriptor, MiniAppKvHandleId, MiniAppServiceLifecycle,
        MiniAppServiceReleaseDescriptor, MiniAppServiceStorageDescriptor, VersionString,
        MINIAPP_SERVICE_HOST_PROTOCOL_VERSION, MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
        digest_bytes,
    };

    fn digest(seed: &str) -> DigestHex {
        digest_bytes(seed.as_bytes())
    }

    #[test]
    fn spec_input_rejects_foreign_storage_and_accepts_exact_digest_shape() {
        let miniapp_id = MiniAppId::from("miniapp-1");
        let mut input = MiniAppServiceSpecInput {
            miniapp_id: miniapp_id.clone(),
            release: MiniAppReleaseRef {
                release_id: "release-1".into(),
                artifact_id: "artifact-1".into(),
                release_digest: digest("release"),
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            descriptor: MiniAppServiceReleaseDescriptor {
                entrypoint: "service/main.mjs".into(),
                module_digest: digest("module"),
                lifecycle: MiniAppServiceLifecycle::OnDemand,
                uses_files: false,
                uses_private_database: false,
                service_contract_digest: digest("contract"),
                host_protocol_version: VersionString::from(
                    MINIAPP_SERVICE_HOST_PROTOCOL_VERSION,
                ),
                sdk_contract_version: VersionString::from(
                    MINIAPP_SERVICE_SDK_CONTRACT_VERSION,
                ),
                runtime_requirements_digest: digest("runtime"),
            },
            config_schema_digest: digest("schema"),
            config_snapshot_digest: digest("config"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            resource_bindings_digest: digest("bindings"),
            bridge_contract_digest: digest("bridge"),
            contribution_set_digest: digest("contributions"),
            storage: MiniAppServiceStorageDescriptor {
                kv: MiniAppKvHandleDescriptor {
                    handle_id: MiniAppKvHandleId::from("kv-1"),
                    miniapp_id: miniapp_id.clone(),
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        };
        input.validate().unwrap();
        input.storage.kv.miniapp_id = MiniAppId::from("foreign");
        assert!(input.validate().is_err());
    }
}
