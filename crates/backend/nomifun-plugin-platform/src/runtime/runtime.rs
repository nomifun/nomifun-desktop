use async_trait::async_trait;
use nomifun_agent_contracts::{
    MiniAppId, MiniAppMigration, MiniAppProductLifecycleState, ResolvedMiniAppServiceSpec,
};
use uuid::Uuid;

use crate::runtime::{
    PluginRuntimeKind, PluginRuntimePlatformError, PluginRuntimePlatformResult, PluginRuntimeRepositorySnapshot,
    StoredPluginRuntimeRelease,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginRuntimeReleaseCutoverKind {
    Publish,
    Rollback,
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeReleaseCutoverPlan {
    pub kind: PluginRuntimeReleaseCutoverKind,
    pub miniapp_id: MiniAppId,
    pub lifecycle: MiniAppProductLifecycleState,
    pub current_release: Option<StoredPluginRuntimeRelease>,
    pub target_release: StoredPluginRuntimeRelease,
    pub current_active_epoch: u64,
    pub target_active_epoch: u64,
    pub current_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_migrations: Vec<MiniAppMigration>,
}

impl PluginRuntimeReleaseCutoverPlan {
    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.target_release.miniapp_id != self.miniapp_id
            || self.target_active_epoch
                != self
                    .current_active_epoch
                    .checked_add(1)
                    .ok_or_else(|| {
                        PluginRuntimePlatformError::InvalidState(
                            "active Release epoch overflow".into(),
                        )
                    })?
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "release cutover target or epoch is inconsistent".into(),
            ));
        }
        self.target_release.validate()?;
        if let Some(current) = &self.current_release {
            current.validate()?;
            if current.miniapp_id != self.miniapp_id {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "current Release belongs to another Plugin".into(),
                ));
            }
        }
        validate_service_spec(
            &self.miniapp_id,
            &self.target_release,
            self.target_active_epoch,
            self.target_service_spec.as_ref(),
            true,
        )?;
        if let Some(current) = &self.current_release {
            let require_current = self.lifecycle == MiniAppProductLifecycleState::Enabled
                && current.artifact.manifest.payload.service.is_some();
            validate_service_spec(
                &self.miniapp_id,
                current,
                self.current_active_epoch,
                self.current_service_spec.as_ref(),
                require_current,
            )?;
        } else if self.current_service_spec.is_some() {
            return Err(PluginRuntimePlatformError::InvalidState(
                "current Service spec exists without an Active Release".into(),
            ));
        }
        match self.kind {
            PluginRuntimeReleaseCutoverKind::Publish => {
                if self.target_migrations != self.target_release.artifact.manifest.payload.migrations
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Publish must carry the target Release migration set".into(),
                    ));
                }
            }
            PluginRuntimeReleaseCutoverKind::Rollback => {
                if !self.target_migrations.is_empty() {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "Rollback never performs reverse or target migrations".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn requires_service_restart(&self) -> bool {
        match (
            self.current_service_spec.as_ref(),
            self.target_service_spec.as_ref(),
        ) {
            (_, None) => false,
            (Some(current), Some(target)) => {
                current.service_run_key != target.service_run_key
                    || !self.target_migrations.is_empty()
            }
            (None, Some(_)) => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PluginRuntimeLifecyclePlan {
    pub miniapp_id: MiniAppId,
    pub from: MiniAppProductLifecycleState,
    pub to: MiniAppProductLifecycleState,
    pub active_release_epoch: u64,
    pub active_release: Option<StoredPluginRuntimeRelease>,
    pub active_service_spec: Option<ResolvedMiniAppServiceSpec>,
}

impl PluginRuntimeLifecyclePlan {
    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        let touches_execution = self.from == MiniAppProductLifecycleState::Enabled
            || self.to == MiniAppProductLifecycleState::Enabled;
        match &self.active_release {
            Some(release) => {
                if release.miniapp_id != self.miniapp_id {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "lifecycle Release belongs to another Plugin".into(),
                    ));
                }
                let require_spec =
                    touches_execution && release.artifact.manifest.payload.service.is_some();
                validate_service_spec(
                    &self.miniapp_id,
                    release,
                    self.active_release_epoch,
                    self.active_service_spec.as_ref(),
                    require_spec,
                )?;
            }
            None => {
                if self.to == MiniAppProductLifecycleState::Enabled
                    || self.active_service_spec.is_some()
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "enabled lifecycle requires an Active Release".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginRuntimeRuntimeTicket {
    pub ticket_id: String,
}

impl PluginRuntimeRuntimeTicket {
    fn new(prefix: &str) -> Self {
        Self {
            ticket_id: format!("{prefix}-{}", Uuid::now_v7()),
        }
    }
}

/// Transient Host coordinator. Prepared tickets are deliberately not durable:
/// the committed Release pointer remains authoritative and startup reconciliation
/// repairs any post-commit Host or Surface failure.
#[async_trait]
pub trait PluginRuntimeRuntimePort: Send + Sync {
    async fn prepare_release_cutover(
        &self,
        plan: &PluginRuntimeReleaseCutoverPlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket>;
    async fn complete_release_cutover(
        &self,
        ticket: PluginRuntimeRuntimeTicket,
        committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;
    async fn abort_release_cutover(
        &self,
        ticket: PluginRuntimeRuntimeTicket,
        previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;

    async fn prepare_lifecycle(
        &self,
        plan: &PluginRuntimeLifecyclePlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket>;
    async fn complete_lifecycle(
        &self,
        ticket: PluginRuntimeRuntimeTicket,
        committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;
    async fn abort_lifecycle(
        &self,
        ticket: PluginRuntimeRuntimeTicket,
        previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;

    async fn prepare_delete(
        &self,
        snapshot: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;
}

/// Deletes filesystem, Private SQLite, staging, and other non-row owner data.
/// The port must be idempotent; repository-owned rows and the deletion intent
/// are removed only by `PluginRuntimeRepository::finalize_delete`.
#[async_trait]
pub trait PluginRuntimeManagedDataPort: Send + Sync {
    async fn purge_for_delete(
        &self,
        snapshot: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()>;
}

/// A production-safe default for UI-only domain tests and early composition.
/// It never starts Node and rejects every flow that would require a Service.
#[derive(Debug, Default)]
pub struct UiOnlyPluginRuntimeRuntime;

#[async_trait]
impl PluginRuntimeRuntimePort for UiOnlyPluginRuntimeRuntime {
    async fn prepare_release_cutover(
        &self,
        plan: &PluginRuntimeReleaseCutoverPlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket> {
        plan.validate()?;
        reject_service_release(&plan.target_release)?;
        if let Some(current) = &plan.current_release {
            reject_service_release(current)?;
        }
        Ok(PluginRuntimeRuntimeTicket::new("miniapp-release"))
    }

    async fn complete_release_cutover(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn abort_release_cutover(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn prepare_lifecycle(
        &self,
        plan: &PluginRuntimeLifecyclePlan,
    ) -> PluginRuntimePlatformResult<PluginRuntimeRuntimeTicket> {
        plan.validate()?;
        if let Some(active) = &plan.active_release {
            reject_service_release(active)?;
        }
        Ok(PluginRuntimeRuntimeTicket::new("miniapp-lifecycle"))
    }

    async fn complete_lifecycle(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _committed: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn abort_lifecycle(
        &self,
        _ticket: PluginRuntimeRuntimeTicket,
        _previous: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }

    async fn prepare_delete(
        &self,
        snapshot: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        if snapshot.root.product.kind == PluginRuntimeKind::Service {
            return Err(PluginRuntimePlatformError::Runtime(
                "Plugin Service Host is not configured".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopPluginRuntimeManagedData;

#[async_trait]
impl PluginRuntimeManagedDataPort for NoopPluginRuntimeManagedData {
    async fn purge_for_delete(
        &self,
        _snapshot: &PluginRuntimeRepositorySnapshot,
    ) -> PluginRuntimePlatformResult<()> {
        Ok(())
    }
}

fn validate_service_spec(
    miniapp_id: &MiniAppId,
    release: &StoredPluginRuntimeRelease,
    epoch: u64,
    spec: Option<&ResolvedMiniAppServiceSpec>,
    required: bool,
) -> PluginRuntimePlatformResult<()> {
    match (&release.artifact.manifest.payload.service, spec) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(PluginRuntimePlatformError::InvalidState(
            "UI-only Release cannot carry a Service spec".into(),
        )),
        (Some(_), None) if required => Err(PluginRuntimePlatformError::InvalidState(
            "Service Release requires an exact resolved Service spec".into(),
        )),
        (Some(_), None) => Ok(()),
        (Some(_), Some(spec)) => {
            spec.validate_for_release(&release.artifact.manifest.payload)?;
            if &spec.miniapp_id != miniapp_id
                || spec.release != *release.release_ref()
                || spec.active_release_epoch != epoch
            {
                return Err(PluginRuntimePlatformError::InvalidState(
                    "resolved Service spec is stale for the exact Release epoch".into(),
                ));
            }
            Ok(())
        }
    }
}

fn reject_service_release(release: &StoredPluginRuntimeRelease) -> PluginRuntimePlatformResult<()> {
    if release.artifact.manifest.payload.service.is_some() {
        Err(PluginRuntimePlatformError::Runtime(
            "Plugin Service Host is not configured".into(),
        ))
    } else {
        Ok(())
    }
}
