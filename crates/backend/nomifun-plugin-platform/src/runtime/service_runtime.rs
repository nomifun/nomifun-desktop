use async_trait::async_trait;
use std::sync::Arc;
use nomifun_agent_contracts::{
    CanonicalErrorCode, DigestHex, PluginBridgeCallId, PluginProductId, PluginMigration,
    PluginReadyRelease, PluginReleaseRef, PluginServiceReleaseDescriptor,
    PluginServiceRuntimeFingerprint, PluginServiceStorageDescriptor,
    PluginServiceTestCredentialMode, PluginServiceTestOutcome,
    PluginServiceTestReceipt, PluginServiceTestReceiptId, ResolvedPluginServiceSpec,
    ResolvedPluginServiceSpecInputs, StrictJsonValue, PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
    PLUGIN_SERVICE_SDK_CONTRACT_VERSION, PLUGIN_SERVICE_TEST_CONTRACT_VERSION,
};
use nomifun_js_runtime::{CommittedRuntimeProvider, ResolvedNodeRuntime};
use crate::runtime::{
    InMemoryPluginRuntimeServiceHost, PluginRuntimeCallCancellation, PluginRuntimePlatformError,
    PluginRuntimePlatformResult, PluginRuntimeServiceHostPort, PluginRuntimeServiceHostState,
    PluginRuntimeServiceModuleRegistry, PluginRuntimeServiceProcessFactory,
    RuntimeAwarePluginRuntimeServiceProcessFactory, PluginRuntimeServiceLaunch,
    NodePluginRuntimeServiceProcessFactory, PluginRuntimeServiceStoragePort,
    PluginRuntimeServiceStorageRequest, PluginRuntimeServiceStorageResolution,
    PluginRuntimeServiceTestStorageResolution, PluginRuntimeMigrationLedger,
};

/// All immutable inputs needed to resolve one Service Host run.
///
/// The application service builds this value from the exact persisted Release,
/// Product configuration, and owner-scoped bindings. Runtime selection and
/// process ownership stay behind [`PluginRuntimeServiceRuntimeBinding`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeServiceSpecInput {
    pub plugin_product_id: PluginProductId,
    pub release: PluginReleaseRef,
    pub active_release_epoch: u64,
    pub descriptor: PluginServiceReleaseDescriptor,
    pub config_schema_digest: DigestHex,
    pub config_snapshot_digest: DigestHex,
    pub credential_slots_digest: DigestHex,
    pub resource_contract_digest: DigestHex,
    pub resource_bindings_digest: DigestHex,
    pub bridge_contract_digest: DigestHex,
    pub contribution_set_digest: DigestHex,
    pub storage: PluginServiceStorageDescriptor,
}

impl PluginRuntimeServiceSpecInput {
    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.active_release_epoch == 0 {
            return Err(PluginRuntimePlatformError::InvalidState(
                "Plugin Service spec requires a positive Active Release epoch".into(),
            ));
        }
        self.descriptor
            .clone()
            .validate_for_platform()
            .map_err(PluginRuntimePlatformError::Contract)?;
        self.storage
            .validate_for_platform(&self.plugin_product_id)
            .map_err(PluginRuntimePlatformError::Contract)?;
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

#[derive(Clone, Debug)]
pub struct PluginRuntimeServiceTestRunInput {
    pub receipt_id: PluginServiceTestReceiptId,
    pub ready: PluginReadyRelease,
    pub spec: ResolvedPluginServiceSpec,
    pub resolved_test_input_digest: DigestHex,
    pub copied_kv_digest: DigestHex,
    pub copied_private_database_digest: Option<DigestHex>,
    pub empty_files_dir: Option<bool>,
    pub migration_ledger_digest: Option<DigestHex>,
    pub requires_managed_input: bool,
}

/// Runtime/process boundary consumed by the Plugin application service.
///
/// Implementations must hold the committed Runtime lease for every resident
/// process and must bind every invocation to the exact resolved spec.
#[async_trait]
pub trait PluginRuntimeServiceRuntimeBinding: Send + Sync {
    async fn resolve_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceStorageResolution> {
        if uses_files || uses_private_database {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin managed Service storage is not configured".into(),
            ));
        }
        let _ = owner_user_id;
        Ok(PluginRuntimeServiceStorageResolution::host_kv(plugin_product_id.clone()))
    }

    async fn apply_storage_migrations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        storage: &PluginServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &PluginReleaseRef,
        migrations: &[PluginMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
        if migrations.is_empty() {
            return Err(PluginRuntimePlatformError::InvalidState(
                "storage migration set cannot be empty at this boundary".into(),
            ));
        }
        let _ = (
            owner_user_id,
            plugin_product_id,
            storage,
            expected_ledger_digest,
            release,
            applied_at_ms,
        );
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin managed Service storage is not configured".into(),
        ))
    }

    async fn handle_storage_request(
        &self,
        _plugin_product_id: &PluginProductId,
        _storage: &PluginServiceStorageDescriptor,
        _request: PluginRuntimeServiceStorageRequest,
        _cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Plugin managed Service storage is not configured".into(),
        ))
    }

    async fn purge_storage(
        &self,
        _owner_user_id: &str,
        _plugin_product_id: &PluginProductId,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn export_backup_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<crate::runtime::PluginRuntimeBackupStorage> {
        let _ = (owner_user_id, plugin_product_id);
        if uses_files || uses_private_database {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin backup storage is not configured".into(),
            ));
        }
        Ok(crate::runtime::PluginRuntimeBackupStorage {
            kv: Vec::new(),
            files: Vec::new(),
            private_database: None,
            migration_ledger: None,
        })
    }

    async fn import_backup_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        storage: crate::runtime::PluginRuntimeBackupStorage,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<()> {
        let _ = (owner_user_id, plugin_product_id);
        if uses_files
            || uses_private_database
            || !storage.files.is_empty()
            || storage.private_database.is_some()
            || storage.migration_ledger.is_some()
        {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin backup storage is not configured".into(),
            ));
        }
        Ok(())
    }

    async fn create_service_test_storage(
        &self,
        _owner_user_id: &str,
        _plugin_product_id: &PluginProductId,
        _test_id: &str,
        _uses_files: bool,
        _uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceTestStorageResolution> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Plugin Service Test storage is not configured".into(),
        ))
    }

    async fn purge_service_test_storage(
        &self,
        _owner_user_id: &str,
        _plugin_product_id: &PluginProductId,
        _test_id: &str,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn run_service_test(
        &self,
        _input: PluginRuntimeServiceTestRunInput,
    ) -> PluginRuntimePlatformResult<PluginServiceTestReceipt> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Plugin Service Test Host is not configured".into(),
        ))
    }

    async fn current_runtime_fingerprint(
        &self,
    ) -> PluginRuntimePlatformResult<Option<PluginServiceRuntimeFingerprint>> {
        Ok(None)
    }

    async fn resolve_spec(
        &self,
        input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedPluginServiceSpec>;

    async fn bind_active(
        &self,
        spec: ResolvedPluginServiceSpec,
        enabled: bool,
    ) -> PluginRuntimePlatformResult<()>;

    async fn start(
        &self,
        spec: ResolvedPluginServiceSpec,
    ) -> PluginRuntimePlatformResult<()>;

    async fn invoke(
        &self,
        spec: &ResolvedPluginServiceSpec,
        call_id: PluginBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue>;

    async fn cancel(&self, plugin_product_id: &PluginProductId, call_id: &PluginBridgeCallId);

    async fn stop(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()>;

    async fn retry(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()>;

    async fn state(&self, plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState>;

    async fn maintain(&self, now_ms: i64) -> PluginRuntimePlatformResult<()>;

    /// Validates the candidate Runtime against all currently enabled Service
    /// bindings and returns their exact Plugin identities.
    async fn validate_candidate(
        &self,
        candidate: &ResolvedNodeRuntime,
    ) -> PluginRuntimePlatformResult<Vec<PluginProductId>> {
        let _ = candidate;
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Runtime candidate validation is not configured".into(),
        ))
    }

    async fn register_module(
        &self,
        plugin_product_id: PluginProductId,
        release_digest: DigestHex,
        module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()>;
}

/// Default boundary used by unit tests and compositions that only support
/// UI-only Plugins. It never starts Node or accepts a Service invocation.
#[derive(Debug, Default)]
pub struct NoopPluginRuntimeServiceRuntime;

#[async_trait]
impl PluginRuntimeServiceRuntimeBinding for NoopPluginRuntimeServiceRuntime {
    async fn resolve_spec(
        &self,
        _input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedPluginServiceSpec> {
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Runtime is not configured".into(),
        ))
    }

    async fn bind_active(
        &self,
        _spec: ResolvedPluginServiceSpec,
        _enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Runtime is not configured".into(),
        ))
    }

    async fn start(
        &self,
        _spec: ResolvedPluginServiceSpec,
    ) -> PluginRuntimePlatformResult<()> {
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Runtime is not configured".into(),
        ))
    }

    async fn invoke(
        &self,
        _spec: &ResolvedPluginServiceSpec,
        _call_id: PluginBridgeCallId,
        _method: String,
        _payload: StrictJsonValue,
        _cancellation: PluginRuntimeCallCancellation,
        _now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        Err(PluginRuntimePlatformError::ServiceUnavailable(
            "Plugin Service Runtime is not configured".into(),
        ))
    }

    async fn cancel(&self, _plugin_product_id: &PluginProductId, _call_id: &PluginBridgeCallId) {}

    async fn stop(&self, _plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn retry(&self, _plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Runtime is not configured".into(),
        ))
    }

    async fn state(&self, _plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState> {
        None
    }

    async fn maintain(&self, _now_ms: i64) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn validate_candidate(
        &self,
        _candidate: &ResolvedNodeRuntime,
    ) -> PluginRuntimePlatformResult<Vec<PluginProductId>> {
        Ok(Vec::new())
    }

    async fn register_module(
        &self,
        _plugin_product_id: PluginProductId,
        _release_digest: DigestHex,
        _module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }
}

/// Production Service runtime binding. It owns the global Service Host
/// coordinator and resolves each process against the committed JavaScript
/// Runtime authority and exact Release module registry.
pub struct ProductionPluginRuntimeServiceRuntimeBinding {
    authority: Arc<dyn CommittedRuntimeProvider>,
    registry: Arc<PluginRuntimeServiceModuleRegistry>,
    host: Arc<InMemoryPluginRuntimeServiceHost>,
    storage: Option<Arc<dyn PluginRuntimeServiceStoragePort>>,
}

impl std::fmt::Debug for ProductionPluginRuntimeServiceRuntimeBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProductionPluginRuntimeServiceRuntimeBinding")
            .field("registry_root", &self.registry.root())
            .finish_non_exhaustive()
    }
}

impl ProductionPluginRuntimeServiceRuntimeBinding {
    pub fn new(
        authority: Arc<dyn CommittedRuntimeProvider>,
        registry: Arc<PluginRuntimeServiceModuleRegistry>,
        max_active_service_hosts: usize,
    ) -> PluginRuntimePlatformResult<Self> {
        Self::new_with_storage(authority, registry, None, max_active_service_hosts)
    }

    pub fn new_with_storage(
        authority: Arc<dyn CommittedRuntimeProvider>,
        registry: Arc<PluginRuntimeServiceModuleRegistry>,
        storage: Option<Arc<dyn PluginRuntimeServiceStoragePort>>,
        max_active_service_hosts: usize,
    ) -> PluginRuntimePlatformResult<Self> {
        let mut runtime_factory = RuntimeAwarePluginRuntimeServiceProcessFactory::new(
            Arc::clone(&authority),
            Arc::clone(&registry) as Arc<dyn crate::runtime::PluginRuntimeServiceModuleResolver>,
        );
        if let Some(storage) = storage.as_ref() {
            runtime_factory = runtime_factory.with_storage(Arc::clone(storage));
        }
        let factory: Arc<dyn PluginRuntimeServiceProcessFactory> = Arc::new(runtime_factory);
        let host = Arc::new(InMemoryPluginRuntimeServiceHost::with_capacity(
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

    pub fn registry(&self) -> &Arc<PluginRuntimeServiceModuleRegistry> {
        &self.registry
    }

    pub fn host(&self) -> &Arc<InMemoryPluginRuntimeServiceHost> {
        &self.host
    }
}

#[async_trait]
impl PluginRuntimeServiceRuntimeBinding for ProductionPluginRuntimeServiceRuntimeBinding {
    async fn resolve_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceStorageResolution> {
        match (&self.storage, uses_files || uses_private_database) {
            (Some(storage), _) => {
                storage
                    .resolve_service_storage(
                        owner_user_id,
                        plugin_product_id,
                        uses_files,
                        uses_private_database,
                    )
                    .await
            }
            (None, true) => Err(PluginRuntimePlatformError::Runtime(
                "Plugin managed Service storage is not configured".into(),
            )),
            (None, false) => Ok(PluginRuntimeServiceStorageResolution::host_kv(
                plugin_product_id.clone(),
            )),
        }
    }

    async fn apply_storage_migrations(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        storage: &PluginServiceStorageDescriptor,
        expected_ledger_digest: &DigestHex,
        release: &PluginReleaseRef,
        migrations: &[PluginMigration],
        applied_at_ms: i64,
    ) -> PluginRuntimePlatformResult<PluginRuntimeMigrationLedger> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                PluginRuntimePlatformError::Runtime(
                    "Plugin managed Service storage is not configured".into(),
                )
            })?
            .apply_additive_migrations(
                owner_user_id,
                plugin_product_id,
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
        plugin_product_id: &PluginProductId,
        storage: &PluginServiceStorageDescriptor,
        request: PluginRuntimeServiceStorageRequest,
        cancellation: PluginRuntimeCallCancellation,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                PluginRuntimePlatformError::ServiceUnavailable(
                    "Plugin managed Service storage is not configured".into(),
                )
            })?
            .handle_service_request(plugin_product_id, storage, request, cancellation)
            .await
    }

    async fn purge_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
    ) -> PluginRuntimePlatformResult<()> {
        if let Some(storage) = &self.storage {
            storage
                .purge_service_storage(owner_user_id, plugin_product_id)
                .await?;
        }
        Ok(())
    }

    async fn export_backup_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<crate::runtime::PluginRuntimeBackupStorage> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                PluginRuntimePlatformError::Runtime(
                    "Plugin managed Service storage is not configured".into(),
                )
            })?
            .export_backup_storage(
                owner_user_id,
                plugin_product_id,
                uses_files,
                uses_private_database,
            )
            .await
    }

    async fn import_backup_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        storage: crate::runtime::PluginRuntimeBackupStorage,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<()> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                PluginRuntimePlatformError::Runtime(
                    "Plugin managed Service storage is not configured".into(),
                )
            })?
            .import_backup_storage(
                owner_user_id,
                plugin_product_id,
                storage,
                uses_files,
                uses_private_database,
            )
            .await
    }

    async fn create_service_test_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        test_id: &str,
        uses_files: bool,
        uses_private_database: bool,
    ) -> PluginRuntimePlatformResult<PluginRuntimeServiceTestStorageResolution> {
        self.storage
            .as_ref()
            .ok_or_else(|| {
                PluginRuntimePlatformError::ServiceUnavailable(
                    "Plugin managed Service storage is not configured".into(),
                )
            })?
            .create_service_test_storage(
                owner_user_id,
                plugin_product_id,
                test_id,
                uses_files,
                uses_private_database,
            )
            .await
    }

    async fn purge_service_test_storage(
        &self,
        owner_user_id: &str,
        plugin_product_id: &PluginProductId,
        test_id: &str,
    ) -> PluginRuntimePlatformResult<()> {
        if let Some(storage) = &self.storage {
            storage
                .purge_service_test_storage(owner_user_id, plugin_product_id, test_id)
                .await?;
        }
        Ok(())
    }

    async fn run_service_test(
        &self,
        input: PluginRuntimeServiceTestRunInput,
    ) -> PluginRuntimePlatformResult<PluginServiceTestReceipt> {
        input
            .spec
            .validate()
            .map_err(PluginRuntimePlatformError::Contract)?;
        let storage = self.storage.as_ref().ok_or_else(|| {
            PluginRuntimePlatformError::ServiceUnavailable(
                "Plugin managed Service storage is not configured".into(),
            )
        })?;
        let lease = self
            .authority
            .acquire_use(nomifun_js_runtime::JavaScriptWorkKind::PluginServiceTestHost)
            .await
            .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))?;
        let runtime = lease.runtime();
        let expected_runtime = PluginServiceRuntimeFingerprint {
            runtime_installation_id: runtime.fingerprint.runtime_installation_id.clone(),
            runtime_target: runtime.fingerprint.runtime_target.clone(),
            runtime_executable_digest: runtime.fingerprint.executable_digest.clone(),
            node_version: runtime.fingerprint.node_version.clone(),
        };
        if input.spec.runtime != expected_runtime {
            return Err(PluginRuntimePlatformError::StaleServiceGeneration);
        }
        let factory = NodePluginRuntimeServiceProcessFactory::new(
            runtime.executable_path.clone(),
            Arc::clone(&self.registry) as Arc<dyn crate::runtime::PluginRuntimeServiceModuleResolver>,
        )?
        .with_storage(Arc::clone(storage));
        let host_generation = 1;
        let launch = PluginRuntimeServiceLaunch {
            spec: input.spec.clone(),
            host_generation,
        };
        let (outcome, error_code) = match factory.start(launch).await {
            Ok(process) => {
                process.stop().await;
                (
                    if input.requires_managed_input {
                        PluginServiceTestOutcome::NeedsTestInput
                    } else {
                        PluginServiceTestOutcome::Passed
                    },
                    None,
                )
            }
            Err(_) => (
                PluginServiceTestOutcome::Failed,
                Some(CanonicalErrorCode::from(
                    "plugin_service_test_host_failed",
                )),
            ),
        };
        let receipt = PluginServiceTestReceipt {
            receipt_id: input.receipt_id,
            plugin_product_id: input.spec.plugin_product_id.clone(),
            release: input.spec.release.clone(),
            service_run_key: input.spec.service_run_key.clone(),
            outcome,
            error_code,
            runtime: input.spec.runtime.clone(),
            host_target: input.spec.runtime.runtime_target.clone(),
            host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            test_contract_version: PLUGIN_SERVICE_TEST_CONTRACT_VERSION.into(),
            resolved_test_input_digest: input.resolved_test_input_digest,
            copied_kv_digest: input.copied_kv_digest,
            copied_private_database_digest: input.copied_private_database_digest,
            empty_files_dir: input.empty_files_dir,
            migration_ledger_digest: input.migration_ledger_digest,
            credential_mode: PluginServiceTestCredentialMode::None,
            host_generation,
            issued_at_ms: nomifun_common::now_ms().max(1),
        };
        receipt
            .validate_for(&input.ready, &input.spec)
            .map_err(PluginRuntimePlatformError::Contract)?;
        Ok(receipt)
    }

    async fn current_runtime_fingerprint(
        &self,
    ) -> PluginRuntimePlatformResult<Option<PluginServiceRuntimeFingerprint>> {
        Ok(self
            .authority
            .committed_runtime()
            .await
            .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))?
            .map(|runtime| PluginServiceRuntimeFingerprint {
                runtime_installation_id: runtime.fingerprint.runtime_installation_id,
                runtime_target: runtime.fingerprint.runtime_target,
                runtime_executable_digest: runtime.fingerprint.executable_digest,
                node_version: runtime.fingerprint.node_version,
            }))
    }

    async fn resolve_spec(
        &self,
        input: PluginRuntimeServiceSpecInput,
    ) -> PluginRuntimePlatformResult<ResolvedPluginServiceSpec> {
        input.validate()?;
        let runtime = self
            .authority
            .committed_runtime()
            .await
            .map_err(|error| PluginRuntimePlatformError::Runtime(error.to_string()))?
            .ok_or_else(|| {
                PluginRuntimePlatformError::ServiceUnavailable(
                    "no committed JavaScript Runtime is selected".into(),
                )
            })?;
        let runtime = PluginServiceRuntimeFingerprint {
            runtime_installation_id: runtime.fingerprint.runtime_installation_id,
            runtime_target: runtime.fingerprint.runtime_target,
            runtime_executable_digest: runtime.fingerprint.executable_digest,
            node_version: runtime.fingerprint.node_version,
        };
        ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: input.plugin_product_id,
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
        .map_err(PluginRuntimePlatformError::Contract)
    }

    async fn bind_active(
        &self,
        spec: ResolvedPluginServiceSpec,
        enabled: bool,
    ) -> PluginRuntimePlatformResult<()> {
        self.host.bind_active(spec, enabled).await
    }

    async fn start(
        &self,
        spec: ResolvedPluginServiceSpec,
    ) -> PluginRuntimePlatformResult<()> {
        let plugin_product_id = spec.plugin_product_id.clone();
        self.host.bind_active(spec, true).await?;
        if matches!(
            self.host.state(&plugin_product_id).await,
            Some(PluginRuntimeServiceHostState::Running { .. })
        ) {
            return Ok(());
        }
        self.host.retry(&plugin_product_id).await
    }

    async fn invoke(
        &self,
        spec: &ResolvedPluginServiceSpec,
        call_id: PluginBridgeCallId,
        method: String,
        payload: StrictJsonValue,
        cancellation: PluginRuntimeCallCancellation,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<StrictJsonValue> {
        self.host
            .invoke(spec, call_id, method, payload, cancellation, now_ms)
            .await
    }

    async fn cancel(&self, plugin_product_id: &PluginProductId, call_id: &PluginBridgeCallId) {
        self.host.cancel(plugin_product_id, call_id).await;
    }

    async fn stop(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        self.host.stop(plugin_product_id).await
    }

    async fn retry(&self, plugin_product_id: &PluginProductId) -> PluginRuntimePlatformResult<()> {
        self.host.retry(plugin_product_id).await
    }

    async fn state(&self, plugin_product_id: &PluginProductId) -> Option<PluginRuntimeServiceHostState> {
        self.host.state(plugin_product_id).await
    }

    async fn maintain(&self, now_ms: i64) -> PluginRuntimePlatformResult<()> {
        self.host.observe_process_exits(now_ms).await;
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
    ) -> PluginRuntimePlatformResult<Vec<PluginProductId>> {
        let specs = self.host.enabled_service_specs().await;
        let factory = NodePluginRuntimeServiceProcessFactory::new(
            candidate.executable_path.clone(),
            Arc::clone(&self.registry) as Arc<dyn crate::runtime::PluginRuntimeServiceModuleResolver>,
        )?;
        let factory = match &self.storage {
            Some(storage) => factory.with_storage(Arc::clone(storage)),
            None => factory,
        };
        for (index, spec) in specs.iter().enumerate() {
            let candidate_spec = spec_with_runtime(spec, candidate)?;
            let process = factory
                .start(PluginRuntimeServiceLaunch {
                    spec: candidate_spec,
                    host_generation: u64::try_from(index + 1).map_err(|_| {
                        PluginRuntimePlatformError::Runtime(
                            "Plugin Service candidate generation overflow".into(),
                        )
                    })?,
                })
                .await?;
            process.stop().await;
        }
        Ok(specs
            .into_iter()
            .map(|spec| spec.plugin_product_id)
            .collect())
    }

    async fn register_module(
        &self,
        plugin_product_id: PluginProductId,
        release_digest: DigestHex,
        module_path: std::path::PathBuf,
    ) -> PluginRuntimePlatformResult<()> {
        self.registry
            .register(plugin_product_id, release_digest, module_path)
            .await
            .map(|_| ())
    }
}

fn spec_with_runtime(
    spec: &ResolvedPluginServiceSpec,
    candidate: &ResolvedNodeRuntime,
) -> PluginRuntimePlatformResult<ResolvedPluginServiceSpec> {
    ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
        plugin_product_id: spec.plugin_product_id.clone(),
        release: spec.release.clone(),
        active_release_epoch: spec.active_release_epoch,
        service_module_digest: spec.service_module_digest.clone(),
        lifecycle: spec.lifecycle,
        host_protocol_version: spec.host_protocol_version.clone(),
        sdk_contract_version: spec.sdk_contract_version.clone(),
        runtime: PluginServiceRuntimeFingerprint {
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
    .map_err(PluginRuntimePlatformError::Contract)
}

fn validate_digest(value: &DigestHex, field: &str) -> PluginRuntimePlatformResult<()> {
    if value.as_ref().len() == 64
        && value
            .as_ref()
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(PluginRuntimePlatformError::InvalidState(format!(
            "{field} must be a lowercase SHA-256 digest"
        )))
    }
}

// These validation helpers are intentionally kept at the platform boundary.
// The contract types keep their detailed validators private because they are
// also used by schema generation; this adapter needs the same exact checks
// without exposing mutable internals.
trait PlatformServiceDescriptorValidation {
    fn validate_for_platform(&self) -> Result<(), nomifun_agent_contracts::PluginRuntimeContractError>;
}

impl PlatformServiceDescriptorValidation for PluginServiceReleaseDescriptor {
    fn validate_for_platform(&self) -> Result<(), nomifun_agent_contracts::PluginRuntimeContractError> {
        if self.entrypoint != "service/main.mjs" {
            return Err(nomifun_agent_contracts::PluginRuntimeContractError::InvalidField {
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
            return Err(nomifun_agent_contracts::PluginRuntimeContractError::InvalidField {
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
        plugin_product_id: &PluginProductId,
    ) -> Result<(), nomifun_agent_contracts::PluginRuntimeContractError>;
}

impl PlatformStorageValidation for PluginServiceStorageDescriptor {
    fn validate_for_platform(
        &self,
        plugin_product_id: &PluginProductId,
    ) -> Result<(), nomifun_agent_contracts::PluginRuntimeContractError> {
        if self.kv.plugin_product_id != *plugin_product_id
            || self
                .files_dir
                .as_ref()
                .is_some_and(|value| value.plugin_product_id != *plugin_product_id)
            || self
                .private_database
                .as_ref()
                .is_some_and(|value| value.plugin_product_id != *plugin_product_id)
        {
            return Err(nomifun_agent_contracts::PluginRuntimeContractError::InvalidField {
                field: "storage.plugin_product_id",
                reason: "all Service storage handles must belong to the exact Plugin".into(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use async_trait::async_trait;
    use super::*;
    use nomifun_agent_contracts::{
        ArtifactId, PluginKvHandleDescriptor, PluginKvHandleId, PluginReadyOrigin,
        PluginReadyRelease, PluginReleaseId, PluginReleaseRef, PluginServiceLifecycle,
        PluginReleaseSourceLineage,
        PluginServiceReleaseDescriptor, PluginServiceStorageDescriptor,
        NodeProbeDisposition, NodeRuntimeFingerprint, NodeRuntimeProbeResult,
        NodeRuntimeSourceKind, ResolvedPluginServiceSpec, RuntimeSelectionRecord,
        ResolvedPluginServiceSpecInputs, RuntimeInstallationId, RuntimeTarget, VersionString,
        PLUGIN_SERVICE_HOST_PROTOCOL_VERSION, PLUGIN_SERVICE_SDK_CONTRACT_VERSION, digest_bytes,
    };
    use nomifun_js_runtime::{
        CommittedRuntimeProvider, JavaScriptRuntimeError, JavaScriptWorkKind,
        NodeDiscoveryRequest, NodeProbeCandidate, NodeRuntimeManager, NodeRuntimeProbePort,
        RuntimeAuthority, RuntimeSelectionStore, RuntimeSelectionStoreError, RuntimeUseLease,
        VersionedRuntimeSelection,
    };
    use tempfile::TempDir;
    use tokio::sync::Mutex;

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

    struct TestRuntimeStore {
        value: Mutex<VersionedRuntimeSelection>,
    }

    impl Default for TestRuntimeStore {
        fn default() -> Self {
            Self {
                value: Mutex::new(VersionedRuntimeSelection::empty()),
            }
        }
    }

    #[async_trait]
    impl RuntimeSelectionStore for TestRuntimeStore {
        async fn load(
            &self,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            Ok(self.value.lock().await.clone())
        }

        async fn save_cas(
            &self,
            expected_revision: u64,
            selection: &RuntimeSelectionRecord,
            selected_executable_path: Option<&Path>,
            pending_candidate_executable_path: Option<&Path>,
            updated_at_ms: i64,
        ) -> Result<VersionedRuntimeSelection, RuntimeSelectionStoreError> {
            let mut value = self.value.lock().await;
            if value.revision != expected_revision {
                return Err(RuntimeSelectionStoreError::Conflict("stale".into()));
            }
            value.revision += 1;
            value.selection = selection.clone();
            value.selected_executable_path =
                selected_executable_path.map(Path::to_path_buf);
            value.pending_candidate_executable_path =
                pending_candidate_executable_path.map(Path::to_path_buf);
            value.updated_at_ms = updated_at_ms;
            Ok(value.clone())
        }
    }

    struct TestRuntimeProbe {
        fingerprint: NodeRuntimeFingerprint,
        executable_path: PathBuf,
    }

    #[async_trait]
    impl NodeRuntimeProbePort for TestRuntimeProbe {
        async fn resolve(
            &self,
            _request: NodeDiscoveryRequest,
        ) -> Result<nomifun_js_runtime::NodeResolution, JavaScriptRuntimeError> {
            Ok(nomifun_js_runtime::NodeResolution {
                selected: Some(self.fingerprint.clone()),
                probes: vec![NodeRuntimeProbeResult {
                    source_kind: self.fingerprint.source_kind,
                    executable_path: self.executable_path.display().to_string(),
                    disposition: NodeProbeDisposition::CompatibleRecommended,
                    fingerprint: Some(self.fingerprint.clone()),
                    error_code: None,
                }],
            })
        }

        async fn probe(&self, _candidate: &NodeProbeCandidate) -> NodeRuntimeProbeResult {
            unreachable!()
        }
    }

    #[test]
    fn spec_input_rejects_foreign_storage_and_accepts_exact_digest_shape() {
        let plugin_product_id = PluginProductId::from("plugin-1");
        let mut input = PluginRuntimeServiceSpecInput {
            plugin_product_id: plugin_product_id.clone(),
            release: PluginReleaseRef {
                release_id: "release-1".into(),
                artifact_id: "artifact-1".into(),
                release_digest: digest("release"),
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            descriptor: PluginServiceReleaseDescriptor {
                entrypoint: "service/main.mjs".into(),
                module_digest: digest("module"),
                lifecycle: PluginServiceLifecycle::OnDemand,
                uses_files: false,
                uses_private_database: false,
                service_contract_digest: digest("contract"),
                host_protocol_version: VersionString::from(
                    PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
                ),
                sdk_contract_version: VersionString::from(
                    PLUGIN_SERVICE_SDK_CONTRACT_VERSION,
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
            storage: PluginServiceStorageDescriptor {
                kv: PluginKvHandleDescriptor {
                    handle_id: PluginKvHandleId::from("kv-1"),
                    plugin_product_id: plugin_product_id.clone(),
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        };
        input.validate().unwrap();
        input.storage.kv.plugin_product_id = PluginProductId::from("foreign");
        assert!(input.validate().is_err());
    }

    #[test]
    fn candidate_runtime_rebind_recomputes_service_run_key() {
        let plugin_product_id = PluginProductId::from("plugin-1");
        let input = PluginRuntimeServiceSpecInput {
            plugin_product_id: plugin_product_id.clone(),
            release: PluginReleaseRef {
                release_id: "release-1".into(),
                artifact_id: "artifact-1".into(),
                release_digest: digest("release"),
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            descriptor: PluginServiceReleaseDescriptor {
                entrypoint: "service/main.mjs".into(),
                module_digest: digest("module"),
                lifecycle: PluginServiceLifecycle::OnDemand,
                uses_files: false,
                uses_private_database: false,
                service_contract_digest: digest("contract"),
                host_protocol_version: VersionString::from(
                    PLUGIN_SERVICE_HOST_PROTOCOL_VERSION,
                ),
                sdk_contract_version: VersionString::from(
                    PLUGIN_SERVICE_SDK_CONTRACT_VERSION,
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
            storage: PluginServiceStorageDescriptor {
                kv: PluginKvHandleDescriptor {
                    handle_id: PluginKvHandleId::from("kv-1"),
                    plugin_product_id,
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        };
        let original = ResolvedPluginServiceSpec::new(
            ResolvedPluginServiceSpecInputs {
                plugin_product_id: input.plugin_product_id.clone(),
                release: input.release.clone(),
                active_release_epoch: input.active_release_epoch,
                service_module_digest: input.descriptor.module_digest.clone(),
                lifecycle: input.descriptor.lifecycle,
                host_protocol_version: input.descriptor.host_protocol_version.clone(),
                sdk_contract_version: input.descriptor.sdk_contract_version.clone(),
                runtime: PluginServiceRuntimeFingerprint {
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

        let plugin_product_id = PluginProductId::from("plugin-candidate-validation");
        let registry = Arc::new(PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap());
        registry
            .register(
                plugin_product_id.clone(),
                digest("release"),
                module_path,
            )
            .await
            .unwrap();
        let spec = ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: plugin_product_id.clone(),
            release: PluginReleaseRef {
                release_id: PluginReleaseId::from("release-id"),
                artifact_id: ArtifactId::from("artifact-id"),
                release_digest: digest("release"),
                manifest_digest: digest("manifest"),
            },
            active_release_epoch: 1,
            service_module_digest: digest_bytes(module),
            lifecycle: PluginServiceLifecycle::OnDemand,
            host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime: PluginServiceRuntimeFingerprint {
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
            storage: PluginServiceStorageDescriptor {
                kv: PluginKvHandleDescriptor {
                    handle_id: PluginKvHandleId::from("kv"),
                    plugin_product_id: plugin_product_id.clone(),
                    namespace_revision: 1,
                },
                files_dir: None,
                private_database: None,
            },
        })
        .unwrap();
        let binding = ProductionPluginRuntimeServiceRuntimeBinding::new(
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
            vec![plugin_product_id.clone()]
        );
        assert!(matches!(
            binding.host().state(&plugin_product_id).await,
            Some(PluginRuntimeServiceHostState::Stopped)
        ));
    }

    #[tokio::test]
    async fn production_service_test_uses_one_shot_node_and_returns_host_receipt() {
        let node = which::which("node").expect("Node is required for this production check");
        let node_digest = digest_bytes(
            &tokio::fs::read(&node)
                .await
                .expect("read Node executable"),
        );
        let fingerprint = NodeRuntimeFingerprint {
            runtime_installation_id: RuntimeInstallationId::from("service-test-node"),
            source_kind: NodeRuntimeSourceKind::ProcessPath,
            runtime_target: RuntimeTarget::from("windows-x86_64"),
            node_version: VersionString::from("24.0.0"),
            node_major: 24,
            executable_digest: node_digest.clone(),
            javascript_host_protocol_version:
                nomifun_agent_contracts::JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                nomifun_agent_contracts::JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        };
        let authority = RuntimeAuthority::new(
            Arc::new(NodeRuntimeManager::new(Arc::new(
                TestRuntimeStore::default(),
            ))),
            Arc::new(TestRuntimeProbe {
                fingerprint: fingerprint.clone(),
                executable_path: node,
            }),
        );
        authority.initialize_if_empty().await.unwrap();

        let temp = TempDir::new().unwrap();
        let module_path = temp
            .path()
            .join("release-test")
            .join("service")
            .join("main.mjs");
        tokio::fs::create_dir_all(module_path.parent().unwrap())
            .await
            .unwrap();
        let module = br#"
export async function start() {
  return {
    async invoke({ payload }) { return payload; },
    async dispose() {},
  };
}
"#;
        tokio::fs::write(&module_path, module).await.unwrap();
        let plugin_product_id = PluginProductId::from("plugin-service-test");
        let release_digest = digest("service-test-release");
        let release = PluginReleaseRef {
            release_id: PluginReleaseId::from("release-service-test"),
            artifact_id: ArtifactId::from("artifact-service-test"),
            release_digest: release_digest.clone(),
            manifest_digest: digest("service-test-manifest"),
        };
        let registry = Arc::new(PluginRuntimeServiceModuleRegistry::new(temp.path()).unwrap());
        registry
            .register(
                plugin_product_id.clone(),
                release_digest,
                module_path,
            )
            .await
            .unwrap();
        let storage = Arc::new(crate::runtime::InMemoryPluginRuntimeManagedStorage::new());
        let binding = ProductionPluginRuntimeServiceRuntimeBinding::new_with_storage(
            authority,
            registry,
            Some(storage.clone()),
            1,
        )
        .unwrap();
        let test_id = uuid::Uuid::now_v7().to_string();
        let test_storage = binding
            .create_service_test_storage(
                "owner-service-test",
                &plugin_product_id,
                &test_id,
                true,
                false,
            )
            .await
            .unwrap();
        let spec = ResolvedPluginServiceSpec::new(ResolvedPluginServiceSpecInputs {
            plugin_product_id: plugin_product_id.clone(),
            release: release.clone(),
            active_release_epoch: 1,
            service_module_digest: digest_bytes(module),
            lifecycle: PluginServiceLifecycle::OnDemand,
            host_protocol_version: PLUGIN_SERVICE_HOST_PROTOCOL_VERSION.into(),
            sdk_contract_version: PLUGIN_SERVICE_SDK_CONTRACT_VERSION.into(),
            runtime: PluginServiceRuntimeFingerprint {
                runtime_installation_id: fingerprint.runtime_installation_id,
                runtime_target: fingerprint.runtime_target,
                runtime_executable_digest: node_digest,
                node_version: fingerprint.node_version,
            },
            config_schema_digest: digest("schema"),
            config_snapshot_digest: digest("config"),
            credential_slots_digest: digest("credentials"),
            resource_contract_digest: digest("resources"),
            resource_bindings_digest: digest("bindings"),
            runtime_requirements_digest: digest("requirements"),
            bridge_contract_digest: digest("bridge"),
            contribution_set_digest: digest("contributions"),
            storage: test_storage.descriptor,
        })
        .unwrap();
        let ready = PluginReadyRelease {
            plugin_product_id: plugin_product_id.clone(),
            release,
            origin_operation_id: "build-service-test".into(),
            origin: PluginReadyOrigin::Build,
            source_lineage: PluginReleaseSourceLineage::Managed {
                project_id: "project-service-test".into(),
                source_snapshot_digest: digest("source"),
                dependency_lock_digest: digest("lock"),
                build_profile_version:
                    nomifun_agent_contracts::PLUGIN_RELEASE_PROFILE_VERSION.into(),
                build_generation: 1,
            },
            matching_service_test_receipt: None,
            created_at_ms: 1,
        };
        let receipt = binding
            .run_service_test(PluginRuntimeServiceTestRunInput {
                receipt_id: PluginServiceTestReceiptId::from(test_id.clone()),
                ready,
                spec,
                resolved_test_input_digest: digest("test-input"),
                copied_kv_digest: test_storage.copied_kv_digest,
                copied_private_database_digest: None,
                empty_files_dir: Some(true),
                migration_ledger_digest: None,
                requires_managed_input: false,
            })
            .await
            .unwrap();
        assert_eq!(receipt.outcome, PluginServiceTestOutcome::Passed);
        assert_eq!(receipt.error_code, None);
        binding
            .purge_service_test_storage(
                "owner-service-test",
                &plugin_product_id,
                &test_id,
            )
            .await
            .unwrap();
    }
}
