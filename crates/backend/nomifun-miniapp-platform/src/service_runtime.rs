use async_trait::async_trait;
use std::sync::Arc;
use nomifun_agent_contracts::{
    DigestHex, MiniAppBridgeCallId, MiniAppServiceReleaseDescriptor,
    MiniAppMigration, MiniAppServiceRuntimeFingerprint,
    MiniAppServiceStorageDescriptor, MiniAppId, MiniAppReleaseRef, ResolvedMiniAppServiceSpec,
    ResolvedMiniAppServiceSpecInputs, StrictJsonValue,
};
use nomifun_js_runtime::{CommittedRuntimeProvider, ResolvedNodeRuntime};
use crate::{
    InMemoryMiniAppServiceHost, MiniAppCallCancellation, MiniAppPlatformError,
    MiniAppPlatformResult, MiniAppServiceHostPort, MiniAppServiceHostState,
    MiniAppServiceModuleRegistry, MiniAppServiceProcessFactory,
    RuntimeAwareMiniAppServiceProcessFactory, MiniAppServiceLaunch,
    NodeMiniAppServiceProcessFactory, MiniAppServiceStoragePort,
    MiniAppServiceStorageRequest, MiniAppServiceStorageResolution, MiniAppMigrationLedger,
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
    async fn resolve_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppServiceStorageResolution> {
        if uses_files || uses_private_database {
            return Err(MiniAppPlatformError::Runtime(
                "MiniApp managed Service storage is not configured".into(),
            ));
        }
        let _ = owner_user_id;
        Ok(MiniAppServiceStorageResolution::host_kv(miniapp_id.clone()))
    }

    async fn apply_storage_migrations(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        if migrations.is_empty() {
            return Err(MiniAppPlatformError::InvalidState(
                "storage migration set cannot be empty at this boundary".into(),
            ));
        }
        let _ = (
            owner_user_id,
            miniapp_id,
            storage,
            expected_ledger_digest,
            release,
            applied_at_ms,
        );
        Err(MiniAppPlatformError::Runtime(
            "MiniApp managed Service storage is not configured".into(),
        ))
    }

    async fn handle_storage_request(
        &self,
        _miniapp_id: &MiniAppId,
        _storage: &MiniAppServiceStorageDescriptor,
        _request: MiniAppServiceStorageRequest,
        _cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        Err(MiniAppPlatformError::ServiceUnavailable(
            "MiniApp managed Service storage is not configured".into(),
        ))
    }

    async fn purge_storage(
        &self,
        _owner_user_id: &str,
        _miniapp_id: &MiniAppId,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

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

    /// Validates the candidate Runtime against all currently enabled Service
    /// bindings and returns their exact MiniApp identities.
    async fn validate_candidate(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> MiniAppPlatformResult<Vec<MiniAppId>> {
        let _ = candidate;
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Runtime candidate validation is not configured".into(),
        ))
    }

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

    async fn validate_candidate(
        &self,
        _candidate: &ResolvedNodeRuntime,
    ) -> MiniAppPlatformResult<Vec<MiniAppId>> {
        Ok(Vec::new())
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
    storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
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
        Self::new_with_storage(authority, registry, None, max_active_service_hosts)
    }

    pub fn new_with_storage(
        authority: Arc<dyn CommittedRuntimeProvider>,
        registry: Arc<MiniAppServiceModuleRegistry>,
        storage: Option<Arc<dyn MiniAppServiceStoragePort>>,
        max_active_service_hosts: usize,
    ) -> MiniAppPlatformResult<Self> {
        let mut runtime_factory = RuntimeAwareMiniAppServiceProcessFactory::new(
            Arc::clone(&authority),
            Arc::clone(&registry) as Arc<dyn crate::MiniAppServiceModuleResolver>,
        );
        if let Some(storage) = storage.as_ref() {
            runtime_factory = runtime_factory.with_storage(Arc::clone(storage));
        }
        let factory: Arc<dyn MiniAppServiceProcessFactory> = Arc::new(runtime_factory);
        let host = Arc::new(InMemoryMiniAppServiceHost::with_capacity(
            factory,
            max_active_service_hosts,
        )?);
        Ok(Self {
            authority,
            registry,
            host,
            storage,
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
    async fn resolve_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> MiniAppPlatformResult<MiniAppServiceStorageResolution> {
        match (&self.storage, uses_files || uses_private_database) {
            (Some(storage), _) => {
                storage
                    .resolve_service_storage(
                        owner_user_id,
                        miniapp_id,
                        uses_files,
                        uses_private_database,
                    )
                    .await
            }
            (None, true) => Err(MiniAppPlatformError::Runtime(
                "MiniApp managed Service storage is not configured".into(),
            )),
            (None, false) => Ok(MiniAppServiceStorageResolution::host_kv(
                miniapp_id.clone(),
            )),
        }
    }

    async fn apply_storage_migrations(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &MiniAppReleaseRef,
        migrations: &[MiniAppMigration],
        applied_at_ms: i64,
    ) -> MiniAppPlatformResult<MiniAppMigrationLedger> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                MiniAppPlatformError::Runtime(
                    "MiniApp managed Service storage is not configured".into(),
                )
            })?
            .apply_additive_migrations(
                owner_user_id,
                miniapp_id,
                storage,
                expected_ledger_digest,
                release,
                migrations,
                applied_at_ms,
            )
            .await
    }

    async fn handle_storage_request(
        &self,
        miniapp_id: &MiniAppId,
        storage: &MiniAppServiceStorageDescriptor,
        request: MiniAppServiceStorageRequest,
        cancellation: MiniAppCallCancellation,
    ) -> MiniAppPlatformResult<StrictJsonValue> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                MiniAppPlatformError::ServiceUnavailable(
                    "MiniApp managed Service storage is not configured".into(),
                )
            })?
            .handle_service_request(miniapp_id, storage, request, cancellation)
            .await
    }

    async fn purge_storage(
        &self,
        owner_user_id: &str,
        miniapp_id: &MiniAppId,
    ) -> MiniAppPlatformResult<()> {
        if let Some(storage) = &self.storage {
            storage
                .purge_service_storage(owner_user_id, miniapp_id)
                .await?;
        }
        Ok(())
    }

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

    async fn validate_candidate(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> MiniAppPlatformResult<Vec<MiniAppId>> {
        let specs = self.host.enabled_service_specs().await;
        let factory = NodeMiniAppServiceProcessFactory::new(
            candidate.executable_path.clone(),
            Arc::clone(&self.registry) as Arc<dyn crate::MiniAppServiceModuleResolver>,
        )?;
        let factory = match &self.storage {
            Some(storage) => factory.with_storage(Arc::clone(storage)),
            None => factory,
        };
        for (index, spec) in specs.iter().enumerate() {
            let candidate_spec = spec_with_runtime(spec, candidate)?;
            let process = factory
                .start(MiniAppServiceLaunch {
                    spec: candidate_spec,
                    host_generation: u64::try_from(index + 1).map_err(|_| {
                        MiniAppPlatformError::Runtime(
                            "MiniApp Service candidate generation overflow".into(),
                        )
                    })?,
                })
                .await?;
            process.stop().await;
        }
        Ok(specs
            .into_iter()
            .map(|spec| spec.miniapp_id)
            .collect())
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

fn spec_with_runtime(
    spec: &ResolvedMiniAppServiceSpec,
    candidate: &ResolvedNodeRuntime,
) -> MiniAppPlatformResult<ResolvedMiniAppServiceSpec> {
    ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
        miniapp_id: spec.miniapp_id.clone(),
        release: spec.release.clone(),
        active_release_epoch: spec.active_release_epoch,
        service_module_digest: spec.service_module_digest.clone(),
        lifecycle: spec.lifecycle,
        host_protocol_version: spec.host_protocol_version.clone(),
        sdk_contract_version: spec.sdk_contract_version.clone(),
        runtime: MiniAppServiceRuntimeFingerprint {
            runtime_installation_id: candidate.fingerprint.runtime_installation_id.clone(),
            runtime_target: candidate.fingerprint.runtime_target.clone(),
            runtime_executable_digest: candidate.fingerprint.executable_digest.clone(),
            node_version: candidate.fingerprint.node_version.clone(),
        },
        config_schema_digest: spec.config_schema_digest.clone(),
        config_snapshot_digest: spec.config_snapshot_digest.clone(),
        credential_slots_digest: spec.credential_slots_digest.clone(),
        resource_contract_digest: spec.resource_contract_digest.clone(),
        resource_bindings_digest: spec.resource_bindings_digest.clone(),
        runtime_requirements_digest: spec.runtime_requirements_digest.clone(),
        bridge_contract_digest: spec.bridge_contract_digest.clone(),
        contribution_set_digest: spec.contribution_set_digest.clone(),
        storage: spec.storage.clone(),
    })
    .map_err(MiniAppPlatformError::Contract)
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
    use std::path::PathBuf;

    use async_trait::async_trait;
    use super::*;
    use nomifun_agent_contracts::{
        ArtifactId, MiniAppKvHandleDescriptor, MiniAppKvHandleId, MiniAppReleaseId,
        MiniAppReleaseRef, MiniAppServiceLifecycle,
        MiniAppServiceReleaseDescriptor, MiniAppServiceStorageDescriptor,
        NodeRuntimeFingerprint, NodeRuntimeSourceKind, ResolvedMiniAppServiceSpec,
        ResolvedMiniAppServiceSpecInputs, RuntimeInstallationId, RuntimeTarget, VersionString,
        MINIAPP_SERVICE_HOST_PROTOCOL_VERSION, MINIAPP_SERVICE_SDK_CONTRACT_VERSION, digest_bytes,
    };
    use nomifun_js_runtime::{
        CommittedRuntimeProvider, JavaScriptRuntimeError, JavaScriptWorkKind,
        RuntimeUseLease,
    };
    use tempfile::TempDir;

    fn digest(seed: &str) -> DigestHex {
        digest_bytes(seed.as_bytes())
    }

    struct UnusedRuntimeAuthority;

    #[async_trait]
    impl CommittedRuntimeProvider for UnusedRuntimeAuthority {
        async fn acquire_use(
            &self,
            _kind: JavaScriptWorkKind,
        ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
            Err(JavaScriptRuntimeError::SwitchNotCovered(
                "authority is unused by candidate validation".into(),
            ))
        }

        async fn committed_runtime(
            &self,
        ) -> Result<Option<nomifun_js_runtime::ResolvedNodeRuntime>, JavaScriptRuntimeError>
        {
            Ok(None)
        }
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

    #[test]
    fn candidate_runtime_rebind_recomputes_service_run_key() {
        let miniapp_id = MiniAppId::from("miniapp-1");
        let input = MiniAppServiceSpecInput {
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
                    miniapp_id,
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        };
        let original = ResolvedMiniAppServiceSpec::new(
            ResolvedMiniAppServiceSpecInputs {
                miniapp_id: input.miniapp_id.clone(),
                release: input.release.clone(),
                active_release_epoch: input.active_release_epoch,
                service_module_digest: input.descriptor.module_digest.clone(),
                lifecycle: input.descriptor.lifecycle,
                host_protocol_version: input.descriptor.host_protocol_version.clone(),
                sdk_contract_version: input.descriptor.sdk_contract_version.clone(),
                runtime: MiniAppServiceRuntimeFingerprint {
                    runtime_installation_id: "node-a".into(),
                    runtime_target: "windows-x86_64".into(),
                    runtime_executable_digest: digest("node-a"),
                    node_version: "24.0.0".into(),
                },
                config_schema_digest: input.config_schema_digest.clone(),
                config_snapshot_digest: input.config_snapshot_digest.clone(),
                credential_slots_digest: input.credential_slots_digest.clone(),
                resource_contract_digest: input.resource_contract_digest.clone(),
                resource_bindings_digest: input.resource_bindings_digest.clone(),
                runtime_requirements_digest: input
                    .descriptor
                    .runtime_requirements_digest
                    .clone(),
                bridge_contract_digest: input.bridge_contract_digest.clone(),
                contribution_set_digest: input.contribution_set_digest.clone(),
                storage: input.storage.clone(),
            },
        )
        .unwrap();
        let candidate = ResolvedNodeRuntime {
            fingerprint: nomifun_agent_contracts::NodeRuntimeFingerprint {
                runtime_installation_id: "node-b".into(),
                source_kind: nomifun_agent_contracts::NodeRuntimeSourceKind::Managed,
                node_version: "24.1.0".into(),
                node_major: 24,
                runtime_target: "windows-x86_64".into(),
                executable_digest: digest("node-b"),
                javascript_host_protocol_version:
                    nomifun_agent_contracts::JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version:
                    nomifun_agent_contracts::JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            },
            executable_path: std::path::PathBuf::from(r"C:\node-b\node.exe"),
        };
        let rebound = spec_with_runtime(&original, &candidate).unwrap();
        assert_ne!(original.runtime, rebound.runtime);
        assert_ne!(original.service_run_key, rebound.service_run_key);
        assert_eq!(
            rebound.runtime.runtime_executable_digest,
            candidate.fingerprint.executable_digest
        );
    }

    #[tokio::test]
    async fn production_candidate_validation_runs_bound_service_with_candidate_node() {
        let node = which::which("node").expect("Node is required for this production check");
        let node_digest = digest_bytes(
            &tokio::fs::read(&node)
                .await
                .expect("read Node executable"),
        );
        let temp = TempDir::new().unwrap();
        let module_path = temp
            .path()
            .join("release-a")
            .join("service")
            .join("main.mjs");
        tokio::fs::create_dir_all(module_path.parent().unwrap())
            .await
            .unwrap();
        let module = br#"
export async function start() {
  return {
    async invoke() { return null; },
    async dispose() {},
  };
}
"#;
        tokio::fs::write(&module_path, module).await.unwrap();

        let miniapp_id = MiniAppId::from("miniapp-candidate-validation");
        let registry = Arc::new(MiniAppServiceModuleRegistry::new(temp.path()).unwrap());
        registry
            .register(
                miniapp_id.clone(),
                digest("release"),
                module_path,
            )
            .await
            .unwrap();
        let spec = ResolvedMiniAppServiceSpec::new(ResolvedMiniAppServiceSpecInputs {
            miniapp_id: miniapp_id.clone(),
            release: MiniAppReleaseRef {
                release_id: MiniAppReleaseId::from("release-id"),
                artifact_id: ArtifactId::from("artifact-id"),
                release_digest: digest("release"),
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            service_module_digest: digest_bytes(module),
            lifecycle: MiniAppServiceLifecycle::OnDemand,
            host_protocol_version: MINIAPP_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: MINIAPP_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime: MiniAppServiceRuntimeFingerprint {
                runtime_installation_id: RuntimeInstallationId::from("old-node"),
                runtime_target: RuntimeTarget::from("windows-x86_64"),
                runtime_executable_digest: digest("old-node"),
                node_version: VersionString::from("24.0.0"),
            },
            config_schema_digest: digest("schema"),
            config_snapshot_digest: digest("config"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            resource_bindings_digest: digest("bindings"),
            runtime_requirements_digest: digest("requirements"),
            bridge_contract_digest: digest("bridge"),
            contribution_set_digest: digest("contributions"),
            storage: MiniAppServiceStorageDescriptor {
                kv: MiniAppKvHandleDescriptor {
                    handle_id: MiniAppKvHandleId::from("kv"),
                    miniapp_id: miniapp_id.clone(),
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        })
        .unwrap();
        let binding = ProductionMiniAppServiceRuntimeBinding::new(
            Arc::new(UnusedRuntimeAuthority),
            registry,
            1,
        )
        .unwrap();
        binding.host().bind_active(spec, true).await.unwrap();

        let candidate = nomifun_js_runtime::ResolvedNodeRuntime {
            fingerprint: NodeRuntimeFingerprint {
                runtime_installation_id: RuntimeInstallationId::from("candidate-node"),
                source_kind: NodeRuntimeSourceKind::ProcessPath,
                runtime_target: RuntimeTarget::from("windows-x86_64"),
                node_version: VersionString::from("24.0.0"),
                node_major: 24,
                executable_digest: node_digest,
                javascript_host_protocol_version:
                    nomifun_agent_contracts::JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
                javascript_sdk_contract_version:
                    nomifun_agent_contracts::JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
            },
            executable_path: PathBuf::from(node),
        };
        assert_eq!(
            binding.validate_candidate(&candidate).await.unwrap(),
            vec![miniapp_id.clone()]
        );
        assert!(matches!(
            binding.host().state(&miniapp_id).await,
            Some(MiniAppServiceHostState::Stopped)
        ));
    }
}
