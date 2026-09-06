use async_trait::async_trait;
use nomifun_agent_contracts::{
    MiniAppId, MiniAppMigration, MiniAppProductLifecycleState, ResolvedMiniAppServiceSpec,
};
use uuid::Uuid;

use crate::{
    MiniAppKind, MiniAppPlatformError, MiniAppPlatformResult, MiniAppRepositorySnapshot,
    StoredMiniAppRelease,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MiniAppReleaseCutoverKind {
    Publish,
    Rollback,
}

#[derive(Clone, Debug)]
pub struct MiniAppReleaseCutoverPlan {
    pub kind: MiniAppReleaseCutoverKind,
    pub miniapp_id: MiniAppId,
    pub lifecycle: MiniAppProductLifecycleState,
    pub current_release: Option<StoredMiniAppRelease>,
    pub target_release: StoredMiniAppRelease,
    pub current_active_epoch: u64,
    pub target_active_epoch: u64,
    pub current_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_service_spec: Option<ResolvedMiniAppServiceSpec>,
    pub target_migrations: Vec<MiniAppMigration>,
}

impl MiniAppReleaseCutoverPlan {
    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        if self.target_release.miniapp_id != self.miniapp_id
            || self.target_active_epoch
                != self
                    .current_active_epoch
                    .checked_add(1)
                    .ok_or_else(|| {
                        MiniAppPlatformError::InvalidState(
                            "active Release epoch overflow".into(),
                        )
                    })?
        {
            return Err(MiniAppPlatformError::InvalidState(
                "release cutover target or epoch is inconsistent".into(),
            ));
        }
        self.target_release.validate()?;
        if let Some(current) = &self.current_release {
            current.validate()?;
            if current.miniapp_id != self.miniapp_id {
                return Err(MiniAppPlatformError::InvalidState(
                    "current Release belongs to another MiniApp".into(),
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
            return Err(MiniAppPlatformError::InvalidState(
                "current Service spec exists without an Active Release".into(),
            ));
        }
        match self.kind {
            MiniAppReleaseCutoverKind::Publish => {
                if self.target_migrations != self.target_release.artifact.manifest.payload.migrations
                {
                    return Err(MiniAppPlatformError::InvalidState(
                        "Publish must carry the target Release migration set".into(),
                    ));
                }
            }
            MiniAppReleaseCutoverKind::Rollback => {
                if !self.target_migrations.is_empty() {
                    return Err(MiniAppPlatformError::InvalidState(
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
pub struct MiniAppLifecyclePlan {
    pub miniapp_id: MiniAppId,
    pub from: MiniAppProductLifecycleState,
    pub to: MiniAppProductLifecycleState,
    pub active_release_epoch: u64,
    pub active_release: Option<StoredMiniAppRelease>,
    pub active_service_spec: Option<ResolvedMiniAppServiceSpec>,
}

impl MiniAppLifecyclePlan {
    pub fn validate(&self) -> MiniAppPlatformResult<()> {
        let touches_execution = self.from == MiniAppProductLifecycleState::Enabled
            || self.to == MiniAppProductLifecycleState::Enabled;
        match &self.active_release {
            Some(release) => {
                if release.miniapp_id != self.miniapp_id {
                    return Err(MiniAppPlatformError::InvalidState(
                        "lifecycle Release belongs to another MiniApp".into(),
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
                    return Err(MiniAppPlatformError::InvalidState(
                        "enabled lifecycle requires an Active Release".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiniAppRuntimeTicket {
    pub ticket_id: String,
}

impl MiniAppRuntimeTicket {
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
pub trait MiniAppRuntimePort: Send + Sync {
    async fn prepare_release_cutover(
        &self,
        plan: &MiniAppReleaseCutoverPlan,
    ) -> MiniAppPlatformResult<MiniAppRuntimeTicket>;
    async fn complete_release_cutover(
        &self,
        ticket: MiniAppRuntimeTicket,
        committed: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;
    async fn abort_release_cutover(
        &self,
        ticket: MiniAppRuntimeTicket,
        previous: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;

    async fn prepare_lifecycle(
        &self,
        plan: &MiniAppLifecyclePlan,
    ) -> MiniAppPlatformResult<MiniAppRuntimeTicket>;
    async fn complete_lifecycle(
        &self,
        ticket: MiniAppRuntimeTicket,
        committed: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;
    async fn abort_lifecycle(
        &self,
        ticket: MiniAppRuntimeTicket,
        previous: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;

    async fn prepare_delete(
        &self,
        snapshot: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;
}

/// Deletes filesystem, Private SQLite, staging, and other non-row owner data.
/// The port must be idempotent; repository-owned rows and the deletion intent
/// are removed only by `MiniAppRepository::finalize_delete`.
#[async_trait]
pub trait MiniAppManagedDataPort: Send + Sync {
    async fn purge_for_delete(
        &self,
        snapshot: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()>;
}

/// A production-safe default for UI-only domain tests and early composition.
/// It never starts Node and rejects every flow that would require a Service.
#[derive(Debug, Default)]
pub struct UiOnlyMiniAppRuntime;

#[async_trait]
impl MiniAppRuntimePort for UiOnlyMiniAppRuntime {
    async fn prepare_release_cutover(
        &self,
        plan: &MiniAppReleaseCutoverPlan,
    ) -> MiniAppPlatformResult<MiniAppRuntimeTicket> {
        plan.validate()?;
        reject_service_release(&plan.target_release)?;
        if let Some(current) = &plan.current_release {
            reject_service_release(current)?;
        }
        Ok(MiniAppRuntimeTicket::new("miniapp-release"))
    }

    async fn complete_release_cutover(
        &self,
        _ticket: MiniAppRuntimeTicket,
        _committed: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn abort_release_cutover(
        &self,
        _ticket: MiniAppRuntimeTicket,
        _previous: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn prepare_lifecycle(
        &self,
        plan: &MiniAppLifecyclePlan,
    ) -> MiniAppPlatformResult<MiniAppRuntimeTicket> {
        plan.validate()?;
        if let Some(active) = &plan.active_release {
            reject_service_release(active)?;
        }
        Ok(MiniAppRuntimeTicket::new("miniapp-lifecycle"))
    }

    async fn complete_lifecycle(
        &self,
        _ticket: MiniAppRuntimeTicket,
        _committed: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn abort_lifecycle(
        &self,
        _ticket: MiniAppRuntimeTicket,
        _previous: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }

    async fn prepare_delete(
        &self,
        snapshot: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        if snapshot.root.product.kind == MiniAppKind::Service {
            return Err(MiniAppPlatformError::Runtime(
                "MiniApp Service Host is not configured".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct NoopMiniAppManagedData;

#[async_trait]
impl MiniAppManagedDataPort for NoopMiniAppManagedData {
    async fn purge_for_delete(
        &self,
        _snapshot: &MiniAppRepositorySnapshot,
    ) -> MiniAppPlatformResult<()> {
        Ok(())
    }
}

fn validate_service_spec(
    miniapp_id: &MiniAppId,
    release: &StoredMiniAppRelease,
    epoch: u64,
    spec: Option<&ResolvedMiniAppServiceSpec>,
    required: bool,
) -> MiniAppPlatformResult<()> {
    match (&release.artifact.manifest.payload.service, spec) {
        (None, None) => Ok(()),
        (None, Some(_)) => Err(MiniAppPlatformError::InvalidState(
            "UI-only Release cannot carry a Service spec".into(),
        )),
        (Some(_), None) if required => Err(MiniAppPlatformError::InvalidState(
            "Service Release requires an exact resolved Service spec".into(),
        )),
        (Some(_), None) => Ok(()),
        (Some(_), Some(spec)) => {
            spec.validate_for_release(&release.artifact.manifest.payload)?;
            if &spec.miniapp_id != miniapp_id
                || spec.release != *release.release_ref()
                || spec.active_release_epoch != epoch
            {
                return Err(MiniAppPlatformError::InvalidState(
                    "resolved Service spec is stale for the exact Release epoch".into(),
                ));
            }
            Ok(())
        }
    }
}

fn reject_service_release(release: &StoredMiniAppRelease) -> MiniAppPlatformResult<()> {
    if release.artifact.manifest.payload.service.is_some() {
        Err(MiniAppPlatformError::Runtime(
            "MiniApp Service Host is not configured".into(),
        ))
    } else {
        Ok(())
    }
}
