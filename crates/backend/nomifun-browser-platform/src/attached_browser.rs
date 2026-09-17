//! Attached Chrome Browser Provider runtime contract.
//!
//! This is an implementation/resource port for the shared `browser` Module.
//! It is not an Agent Tool or Capability and it cannot grant Browser Actions.
//! Web research and local search are separate products and never enter this
//! provider contract.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::product::{
    BrowserCapabilityAction, BrowserProviderKind, BrowserSessionAuthority,
};

pub const ATTACHED_CHROME_PROVIDER_ID: &str = "browser.attached-chrome";

/// Immutable installation-level provider identity. No connection token, tab
/// grant, Action allowlist, or model-visible schema is stored here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AttachedBrowserProviderBinding {
    pub schema_version: u32,
    pub runtime_digest: String,
}

/// Provider-internal operation shape used after canonical Browser Action
/// admission. This enum is never registered as a standalone Agent Tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum AttachedBrowserCommand {
    Tabs {},
    Observe {
        tab_id: String,
    },
    Navigate {
        tab_id: String,
        url: String,
    },
    Dialog {
        tab_id: String,
        dialog_id: String,
        accept: bool,
        prompt_text: Option<String>,
    },
    Click {
        tab_id: String,
        observation_id: String,
        ref_id: String,
    },
    Type {
        tab_id: String,
        observation_id: String,
        ref_id: String,
        text: String,
        #[serde(default)]
        replace: bool,
    },
    Press {
        tab_id: String,
        observation_id: String,
        keys: String,
    },
    Scroll {
        tab_id: String,
        observation_id: String,
        delta_x: f64,
        delta_y: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachedBrowserRuntimeError {
    #[error("Connect the installation-level attached Chrome Provider first")]
    Unavailable,
    #[error("This attached-browser run is no longer active")]
    StaleRun,
    #[error("The browser tab is in use or has an unfinished browser operation")]
    Busy,
    #[error("The attached Chrome connection was lost; reconnect explicitly")]
    Disconnected,
    #[error("The requested tab is unavailable to this installation connection")]
    TabDenied,
    #[error("The AgentSession Browser grant does not authorize this action")]
    ActionDenied,
    #[error("The browser request or observation reference is invalid or stale")]
    InvalidInput,
    #[error(
        "The browser operation failed; observe the current page before deciding whether to retry"
    )]
    ExecutionFailed,
    #[error("The browser run was cancelled")]
    Cancelled,
}

#[async_trait]
pub trait AttachedBrowserProviderHost: Send + Sync {
    fn binding(&self) -> AttachedBrowserProviderBinding;

    /// The host checks canonical AgentSession ownership and Browser authority.
    /// Binding a resource never connects Chrome or grants an Action.
    async fn resource(
        &self,
        principal_id: &str,
        agent_session_id: &str,
    ) -> Result<Arc<dyn AttachedBrowserResource>, AttachedBrowserRuntimeError>;
}

#[async_trait]
pub trait AttachedBrowserResource: Send + Sync {
    async fn begin_run(
        &self,
    ) -> Result<Arc<dyn AttachedBrowserTurn>, AttachedBrowserRuntimeError>;
}

#[async_trait]
pub trait AttachedBrowserTurn: Send + Sync {
    fn cancel(&self);
    async fn invoke(
        &self,
        command: AttachedBrowserCommand,
    ) -> Result<Value, AttachedBrowserRuntimeError>;
    async fn settle(&self) -> Result<(), AttachedBrowserRuntimeError>;
    async fn finish(&self) -> Result<(), AttachedBrowserRuntimeError>;
}

/// Final-owner wrapper that applies the same immutable Browser Module and
/// Resource authority used by the managed Provider. The attached Provider can
/// report implementation availability, but cannot create Action authority.
pub struct AuthorizedAttachedBrowserResource {
    authority: BrowserSessionAuthority,
    inner: Arc<dyn AttachedBrowserResource>,
}

impl AuthorizedAttachedBrowserResource {
    pub fn new(
        authority: BrowserSessionAuthority,
        inner: Arc<dyn AttachedBrowserResource>,
    ) -> Result<Self, AttachedBrowserRuntimeError> {
        if authority.resource().provider().kind() != BrowserProviderKind::AttachedChrome {
            return Err(AttachedBrowserRuntimeError::Unavailable);
        }
        Ok(Self { authority, inner })
    }

    pub fn authority(&self) -> &BrowserSessionAuthority {
        &self.authority
    }

    pub async fn begin_run(
        self: &Arc<Self>,
    ) -> Result<Arc<AuthorizedAttachedBrowserTurn>, AttachedBrowserRuntimeError> {
        Ok(Arc::new(AuthorizedAttachedBrowserTurn {
            authority: self.authority.clone(),
            inner: self.inner.begin_run().await?,
        }))
    }
}

pub struct AuthorizedAttachedBrowserTurn {
    authority: BrowserSessionAuthority,
    inner: Arc<dyn AttachedBrowserTurn>,
}

impl AuthorizedAttachedBrowserTurn {
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    pub async fn invoke(
        &self,
        command: AttachedBrowserCommand,
    ) -> Result<Value, AttachedBrowserRuntimeError> {
        self.authority
            .authorize(attached_command_action(&command))
            .map_err(|error| match error {
                crate::runtime::WorkspaceError::ActionDenied => {
                    AttachedBrowserRuntimeError::ActionDenied
                }
                crate::runtime::WorkspaceError::UnsupportedAction => {
                    AttachedBrowserRuntimeError::Unavailable
                }
                _ => AttachedBrowserRuntimeError::ExecutionFailed,
            })?;
        self.inner.invoke(command).await
    }

    pub async fn settle(&self) -> Result<(), AttachedBrowserRuntimeError> {
        self.inner.settle().await
    }

    pub async fn finish(&self) -> Result<(), AttachedBrowserRuntimeError> {
        self.inner.finish().await
    }
}

fn attached_command_action(command: &AttachedBrowserCommand) -> BrowserCapabilityAction {
    match command {
        AttachedBrowserCommand::Tabs {} | AttachedBrowserCommand::Observe { .. } => {
            BrowserCapabilityAction::Observe
        }
        AttachedBrowserCommand::Navigate { .. } => BrowserCapabilityAction::Navigate,
        AttachedBrowserCommand::Dialog { .. }
        | AttachedBrowserCommand::Click { .. }
        | AttachedBrowserCommand::Type { .. }
        | AttachedBrowserCommand::Press { .. }
        | AttachedBrowserCommand::Scroll { .. } => BrowserCapabilityAction::Act,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use serde_json::json;

    use super::*;

    #[test]
    fn provider_command_is_strict_bounded_by_the_adapter_and_has_no_authority_fields() {
        for input in [
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":false,"prompt_text":null}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"prompt_text":"中文回复"}),
        ] {
            assert!(serde_json::from_value::<AttachedBrowserCommand>(input).is_ok());
        }
        for input in [
            json!({"operation":"dialog","tab_id":"tab","accept":true}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog"}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":"true"}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"prompt_text":1}),
            json!({"operation":"dialog","tab_id":"tab","dialog_id":"dialog","accept":true,"action_allowlist":["browser/act"]}),
            json!({"operation":"tabs","provider_id":"other"}),
        ] {
            assert!(serde_json::from_value::<AttachedBrowserCommand>(input).is_err());
        }
    }

    #[test]
    fn attached_provider_identity_is_not_a_browser_action() {
        assert_eq!(
            crate::product::BrowserCapabilityAction::parse(ATTACHED_CHROME_PROVIDER_ID),
            None
        );
        assert_eq!(
            crate::product::BrowserCapabilityAction::parse("web.research/search"),
            None
        );
    }

    struct Resource(Arc<AtomicUsize>);
    struct Turn(Arc<AtomicUsize>);

    #[async_trait]
    impl AttachedBrowserResource for Resource {
        async fn begin_run(
            &self,
        ) -> Result<Arc<dyn AttachedBrowserTurn>, AttachedBrowserRuntimeError> {
            Ok(Arc::new(Turn(self.0.clone())))
        }
    }

    #[async_trait]
    impl AttachedBrowserTurn for Turn {
        fn cancel(&self) {}
        async fn invoke(
            &self,
            _: AttachedBrowserCommand,
        ) -> Result<Value, AttachedBrowserRuntimeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"ok": true}))
        }
        async fn settle(&self) -> Result<(), AttachedBrowserRuntimeError> {
            Ok(())
        }
        async fn finish(&self) -> Result<(), AttachedBrowserRuntimeError> {
            Ok(())
        }
    }

    fn authority(
        actions: impl IntoIterator<Item = BrowserCapabilityAction>,
    ) -> BrowserSessionAuthority {
        let provider = crate::product::BrowserProviderDescriptor::new(
            ATTACHED_CHROME_PROVIDER_ID,
            BrowserProviderKind::AttachedChrome,
            "attached-lock",
            [
                BrowserCapabilityAction::Observe,
                BrowserCapabilityAction::Navigate,
                BrowserCapabilityAction::Act,
                BrowserCapabilityAction::RenderContent,
            ],
        )
        .unwrap();
        let binding = crate::product::BrowserResourceBinding::new(
            "binding",
            "installation-connection",
            "alice",
            provider,
            BrowserCapabilityAction::all().map(BrowserCapabilityAction::resource_operation),
        )
        .unwrap();
        BrowserSessionAuthority::new("alice", "delegated-session", actions, binding).unwrap()
    }

    #[tokio::test]
    async fn attached_provider_cannot_invoke_an_ungranted_browser_action() {
        let calls = Arc::new(AtomicUsize::new(0));
        let resource = Arc::new(
            AuthorizedAttachedBrowserResource::new(
                authority([BrowserCapabilityAction::Observe]),
                Arc::new(Resource(calls.clone())),
            )
            .unwrap(),
        );
        let turn = resource.begin_run().await.unwrap();
        assert_eq!(
            turn.invoke(AttachedBrowserCommand::Navigate {
                tab_id: "tab".into(),
                url: "https://example.test".into(),
            })
            .await,
            Err(AttachedBrowserRuntimeError::ActionDenied)
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(turn
            .invoke(AttachedBrowserCommand::Observe {
                tab_id: "tab".into(),
            })
            .await
            .is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
