//! Git remains a platform resource owner. This witness is not permission to
//! push or proof that an old user request may be replayed.
use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use nomifun_common::AppError;

use super::nomi_core_wave2::NomiCoreWave2Host;

pub(crate) struct WorkspaceGitWitness {
    host: Arc<NomiCoreWave2Host>,
    root: PathBuf,
    receipts: super::hosted_effect_receipts::HostedEffectReceipts,
    user: String,
    session: String,
}

impl WorkspaceGitWitness {
    pub(crate) fn new(
        host: Arc<NomiCoreWave2Host>,
        root: &str,
        receipts: super::hosted_effect_receipts::HostedEffectReceipts,
        user: String,
        session: String,
    ) -> Result<Arc<Self>, AppError> {
        let root = super::nomi_core_wave2::canonical_workspace_root(std::path::Path::new(root))?;
        host.ensure_workspace_git_ready(&root)?;
        Ok(Arc::new(Self {
            host,
            root,
            receipts,
            user,
            session,
        }))
    }
}

#[async_trait]
impl nomifun_ai_agent::engine_effect_scope::EngineEffectSettlement for WorkspaceGitWitness {
    async fn ensure_settled(&self) -> Result<(), AppError> {
        self.host.settle_workspace_git(&self.root).await?;
        self.host.ensure_workspace_git_evidence(&self.root).await
    }

    async fn ensure_source_replay_safe(&self, source: &str) -> Result<(), AppError> {
        self.host.ensure_workspace_git_evidence(&self.root).await?;
        self.receipts
            .replay_safe(&self.user, &self.session, source)
            .await
    }
}

#[async_trait]
impl nomifun_ai_agent::ContextContributor for WorkspaceGitWitness {
    async fn pre_turn_context(&self) -> Option<String> {
        None
    }

    async fn pre_turn_context_for_turn_result(
        &self,
        _: &nomifun_ai_agent::TurnContext,
    ) -> Result<Option<String>, String> {
        self.host
            .ensure_workspace_git_evidence(&self.root)
            .await
            .map_err(|error| error.to_string())?;
        Ok(None)
    }

    fn label(&self) -> &str {
        "platform_git_effect_fence"
    }
}
