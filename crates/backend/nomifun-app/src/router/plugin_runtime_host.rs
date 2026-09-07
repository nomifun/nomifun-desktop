use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_agent_contracts::{
    ActionId, CanonicalSchemaRef, PluginHostCommitFence,
    PluginHostContributionRef, PluginMountId, ResourceBindingId,
    ResourceKind, StrictJsonValue,
};
use nomifun_js_host::{
    ExtensionHostDemandPort, ExtensionHostSupervisor,
    JavaScriptHostConfig, JavaScriptHostError, JavaScriptHostState,
    JavaScriptResourceHandle, MountLoadDemand,
};
use nomifun_js_runtime::{
    CommittedRuntimeProvider, JavaScriptWorkKind, ResolvedNodeRuntime,
    RuntimeUseLease,
};
use nomifun_plugin_service::PluginServiceError;
use tokio::sync::Mutex;

struct BoundExtensionHost {
    runtime: ResolvedNodeRuntime,
    supervisor: Arc<ExtensionHostSupervisor>,
}

/// Shared Extension Host port bound to the process-wide committed Runtime.
///
/// Kernel registrations keep this stable port, not a concrete Supervisor.
/// Every demand holds a Runtime read lease for its complete execution. Runtime
/// switch coordination takes the matching write fence, waits for in-flight
/// demands, stops the current Supervisor, and leaves the next demand to create
/// a clean generation from the newly committed selection.
pub(crate) struct RuntimeBoundExtensionHost {
    runtime: Arc<dyn CommittedRuntimeProvider>,
    host_module: PathBuf,
    state: Mutex<Option<BoundExtensionHost>>,
}

impl std::fmt::Debug for RuntimeBoundExtensionHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeBoundExtensionHost")
            .field("host_module", &self.host_module)
            .finish_non_exhaustive()
    }
}

impl RuntimeBoundExtensionHost {
    pub(crate) fn new(
        runtime: Arc<dyn CommittedRuntimeProvider>,
        host_module: PathBuf,
    ) -> Result<Arc<Self>, JavaScriptHostError> {
        if !host_module.is_absolute() || !host_module.is_file() {
            return Err(JavaScriptHostError::InvalidConfiguration(format!(
                "Runtime-bound Host module must be an absolute regular file: {}",
                host_module.display()
            )));
        }
        Ok(Arc::new(Self {
            runtime,
            host_module,
            state: Mutex::new(None),
        }))
    }

    async fn demand_host(
        &self,
    ) -> Result<(RuntimeUseLease, Arc<ExtensionHostSupervisor>), JavaScriptHostError>
    {
        let lease = self
            .runtime
            .acquire_use(JavaScriptWorkKind::SharedExtensionHost)
            .await
            .map_err(|error| {
                JavaScriptHostError::RuntimeVerification(error.to_string())
            })?;
        let resolved = lease.runtime().clone();
        let mut state = self.state.lock().await;
        if let Some(bound) = state.as_ref() {
            if bound.runtime == resolved {
                return Ok((lease, Arc::clone(&bound.supervisor)));
            }
            if bound.supervisor.process_count() != 0 {
                return Err(JavaScriptHostError::RuntimeVerification(
                    "committed Runtime changed before the previous shared Host was stopped"
                        .to_owned(),
                ));
            }
        }
        let supervisor = Arc::new(ExtensionHostSupervisor::new(
            JavaScriptHostConfig::for_host_module(
                resolved.executable_path.clone(),
                resolved.fingerprint.clone(),
                self.host_module.clone(),
            ),
        )?);
        *state = Some(BoundExtensionHost {
            runtime: resolved,
            supervisor: Arc::clone(&supervisor),
        });
        Ok((lease, supervisor))
    }

    pub(crate) async fn runtime_available(
        &self,
    ) -> Result<bool, PluginServiceError> {
        Ok(self
            .runtime
            .committed_runtime()
            .await
            .map_err(|error| {
                PluginServiceError::integration(error.to_string())
            })?
            .is_some())
    }

    pub(crate) async fn stop_for_runtime_switch(
        &self,
    ) -> Result<(), JavaScriptHostError> {
        let bound = self.state.lock().await.take();
        let Some(bound) = bound else {
            return Ok(());
        };
        let stop_result = match bound.supervisor.state() {
            JavaScriptHostState::Running { generation, .. } => {
                bound.supervisor.stop_generation(generation).await.map(|_| ())
            }
            JavaScriptHostState::Stopped => Ok(()),
            JavaScriptHostState::Failed { generation, reason } => {
                // A Failed state is not a process-tree proof. The supervisor
                // only reports a commit fence after its managed child cleanup
                // completed successfully; retain the binding and require
                // explicit recovery instead of inferring emptiness from the
                // public state bit.
                Err(JavaScriptHostError::HostFailure {
                    generation,
                    reason,
                })
            }
        };
        if let Err(error) = stop_result {
            self.state.lock().await.replace(bound);
            return Err(error);
        }
        if bound.supervisor.process_count() != 0 {
            self.state.lock().await.replace(bound);
            return Err(JavaScriptHostError::HostFailure {
                generation: 0,
                reason: "shared Host process tree is not empty after stop"
                    .to_owned(),
            });
        }
        Ok(())
    }

    pub(crate) async fn commit_fence_for_mount(
        &self,
        mount_id: &PluginMountId,
    ) -> Result<PluginHostCommitFence, JavaScriptHostError> {
        if self.state.lock().await.is_none() {
            return Ok(PluginHostCommitFence::NotResident);
        }
        let lease = self
            .runtime
            .acquire_use(JavaScriptWorkKind::SharedExtensionHost)
            .await
            .map_err(|error| {
                JavaScriptHostError::RuntimeVerification(error.to_string())
            })?;
        let host = {
            let state = self.state.lock().await;
            let Some(bound) = state.as_ref() else {
                return Ok(PluginHostCommitFence::NotResident);
            };
            if bound.runtime != *lease.runtime() {
                return Err(JavaScriptHostError::RuntimeVerification(
                    "shared Host Runtime differs from the committed selection"
                        .to_owned(),
                ));
            }
            Arc::clone(&bound.supervisor)
        };
        match host.commit_fence_for_mount(mount_id).await {
            Ok(fence) => Ok(fence),
            Err(JavaScriptHostError::NotQuiescent { generation }) => {
                host.stop_generation(generation).await
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait]
impl ExtensionHostDemandPort for RuntimeBoundExtensionHost {
    async fn invoke_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        action_id: ActionId,
        input: StrictJsonValue,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        let (_lease, host) = self.demand_host().await?;
        host.invoke_demand(mount, contribution, action_id, input)
            .await
    }

    async fn contribute_context_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        schema_ref: CanonicalSchemaRef,
    ) -> Result<StrictJsonValue, JavaScriptHostError> {
        let (_lease, host) = self.demand_host().await?;
        host.contribute_context_demand(mount, contribution, schema_ref)
            .await
    }

    async fn acquire_resource_demand(
        &self,
        mount: MountLoadDemand,
        contribution: PluginHostContributionRef,
        binding_id: ResourceBindingId,
        resource_kind: ResourceKind,
        parameters: StrictJsonValue,
    ) -> Result<JavaScriptResourceHandle, JavaScriptHostError> {
        let (_lease, host) = self.demand_host().await?;
        host.acquire_resource_demand(
            mount,
            contribution,
            binding_id,
            resource_kind,
            parameters,
        )
        .await
    }

    async fn release_resource(
        &self,
        resource: &JavaScriptResourceHandle,
    ) -> Result<(), JavaScriptHostError> {
        let host = self
            .state
            .lock()
            .await
            .as_ref()
            .map(|bound| Arc::clone(&bound.supervisor));
        match host {
            Some(host) => host.release_resource(resource).await,
            None => Ok(()),
        }
    }
}
