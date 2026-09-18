//! Provider-neutral result of binding an authorized AgentSession Browser Resource.

use std::sync::Arc;

use crate::{
    attached_browser::{
        AttachedBrowserCommand, AttachedBrowserRuntimeError,
        AuthorizedAttachedBrowserResource,
    },
    product::{BrowserProviderKind, BrowserSessionAuthority},
    run_guard::{BrowserInputState, BrowserRunSnapshot, RunAdmissionError},
    runtime::{BrowserTabCommand, WorkspaceError},
    workspace::{BrowserResource, BrowserResourceSnapshot},
};

#[derive(Clone)]
pub enum BoundBrowserProviderResource {
    Managed(Arc<BrowserResource>),
    AttachedChrome(Arc<AuthorizedAttachedBrowserResource>),
}

impl BoundBrowserProviderResource {
    pub fn provider_kind(&self) -> BrowserProviderKind {
        self.authority().resource().provider().kind()
    }

    pub fn authority(&self) -> &BrowserSessionAuthority {
        match self {
            Self::Managed(resource) => resource.authority(),
            Self::AttachedChrome(resource) => resource.authority(),
        }
    }

    /// Provider-neutral REST/product snapshot. Attached Chrome has no managed
    /// native-runtime generation; its installation connection status is
    /// exposed by the provider endpoint, while this snapshot proves the exact
    /// AgentSession Resource binding selected by the canonical route.
    pub async fn snapshot(&self) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        match self {
            Self::Managed(resource) => resource.snapshot().await,
            Self::AttachedChrome(_) => Ok(self.inactive_snapshot()),
        }
    }

    pub fn inactive_snapshot(&self) -> BrowserResourceSnapshot {
        let authority = self.authority();
        BrowserResourceSnapshot {
            agent_session_id: authority.agent_session_id().to_owned(),
            resource_binding_id: authority.resource().binding_id().to_owned(),
            provider_id: authority.resource().provider().provider_id().to_owned(),
            provider_kind: authority.resource().provider().kind(),
            allowed_actions: crate::product::BrowserCapabilityAction::all()
                .into_iter()
                .filter(|action| authority.authorize(*action).is_ok())
                .map(|action| action.action_id().to_owned())
                .collect(),
            run: BrowserRunSnapshot {
                revision: 0,
                input_state: BrowserInputState::UserReady,
                input_gate_failed: false,
            },
            runtime: None,
        }
    }

    /// Human Browser panel commands still enter through the selected provider's
    /// Resource and run authority. The attached Provider deliberately exposes
    /// only operations it can prove against an existing tab; managed-only tab,
    /// profile, download and site-data controls fail closed.
    pub async fn user_command(
        &self,
        command: BrowserTabCommand,
    ) -> Result<BrowserResourceSnapshot, WorkspaceError> {
        match self {
            Self::Managed(resource) => {
                resource.user_command(command).await?;
                resource.snapshot().await
            }
            Self::AttachedChrome(resource) => {
                let command = attached_user_command(command)?;
                let turn = resource
                    .begin_run()
                    .await
                    .map_err(map_attached_error)?;
                let outcome = turn.invoke(command).await.map_err(map_attached_error);
                let cleanup = match turn.settle().await {
                    Ok(()) => turn.finish().await.map_err(map_attached_error),
                    Err(error) => Err(map_attached_error(error)),
                };
                cleanup?;
                outcome?;
                Ok(self.inactive_snapshot())
            }
        }
    }
}

fn attached_user_command(command: BrowserTabCommand) -> Result<AttachedBrowserCommand, WorkspaceError> {
    match command {
        BrowserTabCommand::Navigate { target, url } => Ok(AttachedBrowserCommand::Navigate {
            tab_id: target.tab_id,
            url,
        }),
        BrowserTabCommand::Dialog {
            target,
            request_id,
            accept,
            text,
        } => Ok(AttachedBrowserCommand::Dialog {
            tab_id: target.tab_id,
            dialog_id: request_id,
            accept,
            prompt_text: text,
        }),
        _ => Err(WorkspaceError::UnsupportedAction),
    }
}

fn map_attached_error(error: AttachedBrowserRuntimeError) -> WorkspaceError {
    match error {
        AttachedBrowserRuntimeError::Unavailable | AttachedBrowserRuntimeError::Disconnected => {
            WorkspaceError::NativeUnavailable
        }
        AttachedBrowserRuntimeError::StaleRun => {
            WorkspaceError::Admission(RunAdmissionError::StaleRun)
        }
        AttachedBrowserRuntimeError::Busy => WorkspaceError::Admission(RunAdmissionError::Busy),
        AttachedBrowserRuntimeError::TabDenied => WorkspaceError::TabNotFound,
        AttachedBrowserRuntimeError::ActionDenied => WorkspaceError::ActionDenied,
        AttachedBrowserRuntimeError::InvalidInput => WorkspaceError::StaleObservation,
        AttachedBrowserRuntimeError::ExecutionFailed => WorkspaceError::ActionInterrupted,
        AttachedBrowserRuntimeError::Cancelled => {
            WorkspaceError::Admission(RunAdmissionError::Cancelled)
        }
    }
}
