use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    NodeProbeDisposition, NodeRuntimeFingerprint,
};
use tokio::sync::{
    OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock,
};

use crate::{
    ERR_RUNTIME_SELECTED_STALE, JavaScriptRuntimeError,
    NodeDiscoveryRequest, NodeProbeCandidate, NodeRuntimeManager,
    NodeRuntimeProbePort, RuntimeCandidate, RuntimeSelectionStore,
};

/// The only supported JavaScript execution roles in the first Runtime
/// Manager cut. They are intentionally diagnostic labels, not provider IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JavaScriptWorkKind {
    SharedExtensionHost,
    CandidateTestHost,
    BuildHost,
    MiniappServiceHost,
}

/// The exact executable and fingerprint that a JavaScript consumer is
/// authorized to use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedNodeRuntime {
    pub fingerprint: NodeRuntimeFingerprint,
    pub executable_path: PathBuf,
}

/// A read lease held for the complete lifetime of a runtime-backed operation
/// or resident Host. Runtime switch coordination must acquire the matching
/// write fence before stopping or replacing that Host.
#[derive(Clone)]
pub struct RuntimeUseLease {
    resolved: ResolvedNodeRuntime,
    _admission: Arc<OwnedRwLockReadGuard<()>>,
}

pub struct RuntimeSwitchFence {
    _admission: OwnedRwLockWriteGuard<()>,
}

impl std::fmt::Debug for RuntimeSwitchFence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeSwitchFence")
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for RuntimeUseLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeUseLease")
            .field("resolved", &self.resolved)
            .finish_non_exhaustive()
    }
}

impl RuntimeUseLease {
    pub fn runtime(&self) -> &ResolvedNodeRuntime {
        &self.resolved
    }

    pub fn fingerprint(&self) -> &NodeRuntimeFingerprint {
        &self.resolved.fingerprint
    }

    pub fn executable_path(&self) -> &std::path::Path {
        &self.resolved.executable_path
    }
}

#[async_trait]
pub trait CommittedRuntimeProvider: Send + Sync {
    async fn acquire_use(
        &self,
        kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError>;

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError>;
}

/// Process-wide Runtime authority shared by Plugin, authoring and future
/// MiniApp composition. It owns the admission fence; the persisted selection
/// remains owned by [`NodeRuntimeManager`].
pub struct RuntimeAuthority {
    manager: Arc<NodeRuntimeManager>,
    probe: Arc<dyn NodeRuntimeProbePort>,
    admission: Arc<RwLock<()>>,
}

impl std::fmt::Debug for RuntimeAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAuthority")
            .finish_non_exhaustive()
    }
}

impl RuntimeAuthority {
    pub fn new(
        manager: Arc<NodeRuntimeManager>,
        probe: Arc<dyn NodeRuntimeProbePort>,
    ) -> Arc<Self> {
        Arc::new(Self {
            manager,
            probe,
            admission: Arc::new(RwLock::new(())),
        })
    }

    pub(crate) fn manager(&self) -> &Arc<NodeRuntimeManager> {
        &self.manager
    }

    pub async fn acquire_switch_fence(&self) -> RuntimeSwitchFence {
        RuntimeSwitchFence {
            _admission: self.admission.clone().write_owned().await,
        }
    }

    /// Select the first compatible runtime through the same persisted manager
    /// used by the Runtime API. This is discovery, not a second host binding.
    /// An empty machine remains empty and can be configured later from the UI.
    pub async fn initialize_if_empty(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        let _write = self.admission.write().await;
        let current = self.manager.snapshot().await?;
        if current.selection.pending_candidate.is_some() {
            // Startup recovery owns interrupted switch state. Discovery must
            // not overwrite the durable pending candidate before every
            // participant has had a chance to restore the old selection.
            return self.manager.selected_runtime().await;
        }
        if let Some(selected) = self.manager.selected_runtime().await? {
            let probe = self
                .probe
                .probe(&NodeProbeCandidate::new(
                    selected.fingerprint.source_kind,
                    selected.executable_path.clone(),
                ))
                .await;
            if probe.fingerprint.as_ref() == Some(&selected.fingerprint)
                && matches!(
                    probe.disposition,
                    NodeProbeDisposition::CompatibleRecommended
                        | NodeProbeDisposition::CompatibleNonRecommended
                )
            {
                return Ok(Some(selected));
            }
            self.manager
                .clear_selected(
                    current.revision,
                    nomifun_agent_contracts::CanonicalErrorCode::from(
                        ERR_RUNTIME_SELECTED_STALE,
                    ),
                )
                .await?;
        }
        let resolution = self
            .probe
            .resolve(NodeDiscoveryRequest::default())
            .await?;
        let Some(fingerprint) = resolution.selected else {
            return Ok(None);
        };
        let Some(probe) = resolution
            .probes
            .iter()
            .find(|probe| probe.fingerprint.as_ref() == Some(&fingerprint))
        else {
            return Err(JavaScriptRuntimeError::CandidateStale);
        };
        let candidate = RuntimeCandidate {
            fingerprint,
            executable_path: PathBuf::from(&probe.executable_path),
            disposition: probe.disposition,
        };
        let selected = self
            .manager
            .select_initial(
                self.manager.snapshot().await?.revision,
                candidate.fingerprint,
                candidate.executable_path,
            )
            .await?;
        Ok(Some(selected))
    }

    async fn committed_under_read(
        &self,
    ) -> Result<ResolvedNodeRuntime, JavaScriptRuntimeError> {
        self.manager
            .selected_runtime()
            .await?
            .ok_or_else(|| {
                JavaScriptRuntimeError::SwitchNotCovered(
                    "no committed JavaScript Runtime is selected".to_owned(),
                )
            })
    }
}

#[async_trait]
impl CommittedRuntimeProvider for RuntimeAuthority {
    async fn acquire_use(
        &self,
        _kind: JavaScriptWorkKind,
    ) -> Result<RuntimeUseLease, JavaScriptRuntimeError> {
        let admission = Arc::new(self.admission.clone().read_owned().await);
        let resolved = self.committed_under_read().await?;
        Ok(RuntimeUseLease {
            resolved,
            _admission: admission,
        })
    }

    async fn committed_runtime(
        &self,
    ) -> Result<Option<ResolvedNodeRuntime>, JavaScriptRuntimeError> {
        let _read = self.admission.read().await;
        self.manager.selected_runtime().await
    }
}

/// Build an authority around a persisted selection store. Keeping this
/// constructor in the Runtime crate makes it impossible for App composition
/// to accidentally create a second in-memory selection manager.
pub fn authority_from_store(
    store: Arc<dyn RuntimeSelectionStore>,
    probe: Arc<dyn NodeRuntimeProbePort>,
) -> Arc<RuntimeAuthority> {
    RuntimeAuthority::new(Arc::new(NodeRuntimeManager::new(store)), probe)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use async_trait::async_trait;
    use nomifun_agent_contracts::{
        DigestHex, JAVASCRIPT_HOST_PROTOCOL_VERSION,
        JAVASCRIPT_SDK_CONTRACT_VERSION, NodeProbeDisposition,
        NodeRuntimeProbeResult, NodeRuntimeSourceKind, RuntimeTarget,
        RuntimeSelectionRecord, VersionString,
    };
    use tokio::sync::Mutex;

    use super::*;
    use crate::{
        RuntimeSelectionStore, RuntimeSelectionStoreError,
        VersionedRuntimeSelection,
    };

    struct Store {
        value: Mutex<VersionedRuntimeSelection>,
    }

    impl Default for Store {
        fn default() -> Self {
            Self {
                value: Mutex::new(VersionedRuntimeSelection::empty()),
            }
        }
    }

    #[async_trait]
    impl RuntimeSelectionStore for Store {
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

    struct Probe;

    #[async_trait]
    impl NodeRuntimeProbePort for Probe {
        async fn resolve(
            &self,
            _request: NodeDiscoveryRequest,
        ) -> Result<crate::NodeResolution, JavaScriptRuntimeError> {
            let fingerprint = runtime();
            Ok(crate::NodeResolution {
                selected: Some(fingerprint.clone()),
                probes: vec![NodeRuntimeProbeResult {
                    source_kind: NodeRuntimeSourceKind::ProcessPath,
                    executable_path: r"C:\node\node.exe".into(),
                    disposition: NodeProbeDisposition::CompatibleRecommended,
                    fingerprint: Some(fingerprint),
                    error_code: None,
                }],
            })
        }

        async fn probe(
            &self,
            _candidate: &crate::NodeProbeCandidate,
        ) -> NodeRuntimeProbeResult {
            unreachable!()
        }
    }

    fn runtime() -> NodeRuntimeFingerprint {
        NodeRuntimeFingerprint {
            runtime_installation_id: "node-initial".into(),
            source_kind: NodeRuntimeSourceKind::ProcessPath,
            node_version: VersionString::from("24.8.0"),
            node_major: 24,
            runtime_target: RuntimeTarget::from("x86_64-pc-windows-msvc"),
            executable_digest: DigestHex::from("a".repeat(64)),
            javascript_host_protocol_version:
                JAVASCRIPT_HOST_PROTOCOL_VERSION.into(),
            javascript_sdk_contract_version:
                JAVASCRIPT_SDK_CONTRACT_VERSION.into(),
        }
    }

    #[tokio::test]
    async fn initialization_and_lease_share_the_persisted_selection() {
        let authority = RuntimeAuthority::new(
            Arc::new(NodeRuntimeManager::new(Arc::new(Store::default()))),
            Arc::new(Probe),
        );
        let selected = authority.initialize_if_empty().await.unwrap().unwrap();
        assert_eq!(selected.fingerprint, runtime());
        let lease = authority
            .acquire_use(JavaScriptWorkKind::BuildHost)
            .await
            .unwrap();
        assert_eq!(lease.runtime(), &selected);
        assert_eq!(
            authority.committed_runtime().await.unwrap(),
            Some(selected)
        );
    }
}
