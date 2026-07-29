//! The protocol-adapter seam: one [`ProtocolAdapter`] per remote wire protocol
//! (e.g. `"openai.images"`, `"ark.video_jobs"`), looked up in the
//! [`AdapterRegistry`] strictly by `(protocol, task)` — model-name/platform
//! `if` routing is banned here (that lives only in the catalog seeding layer).

use std::collections::HashMap;
use std::sync::Arc;

use nomifun_api_types::ModelTask;

use crate::call::ResolvedCall;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::types::{JobHandle, TaskOutcome};

/// One remote protocol implementation.
#[async_trait::async_trait]
pub trait ProtocolAdapter: Send + Sync {
    /// Stable protocol id (registry key), e.g. `"openai.images"`.
    fn id(&self) -> &'static str;
    /// Whether this adapter can serve `task`.
    fn supports(&self, task: ModelTask) -> bool;
    /// Execute (or start) the call.
    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError>;
    /// Poll an async job. Default: this protocol has no async jobs.
    async fn poll(
        &self,
        _http: &reqwest::Client,
        _call: &ResolvedCall,
        _job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        Err(InvokeError::not_pollable())
    }
}

/// Immutable protocol → adapter table, keyed by [`ProtocolAdapter::id`].
pub struct AdapterRegistry {
    map: HashMap<&'static str, Arc<dyn ProtocolAdapter>>,
}

impl AdapterRegistry {
    /// Build the registry from the assembled adapter set (key = `adapter.id()`).
    pub fn new(adapters: Vec<Arc<dyn ProtocolAdapter>>) -> Self {
        Self { map: adapters.into_iter().map(|a| (a.id(), a)).collect() }
    }

    /// Look up the adapter for `(protocol, task)`.
    /// Unregistered protocol → [`InvokeErrorKind::NoAdapter`] (message names
    /// both protocol and task); registered but `!supports(task)` → same kind.
    pub fn get(&self, protocol: &str, task: ModelTask) -> Result<Arc<dyn ProtocolAdapter>, InvokeError> {
        let Some(adapter) = self.map.get(protocol) else {
            return Err(InvokeError::new(
                InvokeErrorKind::NoAdapter,
                format!("no adapter registered for protocol {protocol:?} (task {task:?})"),
            ));
        };
        if !adapter.supports(task) {
            return Err(InvokeError::new(
                InvokeErrorKind::NoAdapter,
                format!("adapter {protocol:?} does not support task {task:?}"),
            ));
        }
        Ok(Arc::clone(adapter))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::ResolvedConnection;
    use crate::types::{ChatTextRequest, TaskRequest, TaskResult};

    struct FakeAdapter;

    #[async_trait::async_trait]
    impl ProtocolAdapter for FakeAdapter {
        fn id(&self) -> &'static str {
            "fake.chat"
        }
        fn supports(&self, task: ModelTask) -> bool {
            task == ModelTask::Chat
        }
        async fn submit(&self, _http: &reqwest::Client, _call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
            Ok(TaskOutcome::Done(TaskResult::Text("ok".into())))
        }
    }

    fn call() -> ResolvedCall {
        ResolvedCall {
            provider_id: "p".into(),
            platform: "openai".into(),
            model: "m".into(),
            task: ModelTask::Chat,
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: "https://x.test".into(),
                is_full_url: false,
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({"api_keys": ["sk"]}) },
                extra: json!({}),
            },
            model_params: json!({}),
            request: TaskRequest::ChatText(ChatTextRequest { prompt: "hi".into(), system: None, extra: json!({}) }),
        }
    }

    fn registry() -> AdapterRegistry {
        AdapterRegistry::new(vec![Arc::new(FakeAdapter)])
    }

    #[test]
    fn get_returns_registered_supporting_adapter() {
        let adapter = registry().get("fake.chat", ModelTask::Chat).expect("registered + supported");
        assert_eq!(adapter.id(), "fake.chat");
    }

    #[test]
    fn get_unregistered_protocol_is_no_adapter_naming_protocol_and_task() {
        let err = registry().get("volc.tts_v3", ModelTask::SpeechSynthesis).map(|a| a.id()).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
        assert!(err.message.contains("volc.tts_v3"), "message: {}", err.message);
        assert!(err.message.contains("SpeechSynthesis"), "message: {}", err.message);
    }

    #[test]
    fn get_registered_but_unsupported_task_is_no_adapter() {
        let err = registry().get("fake.chat", ModelTask::Embedding).map(|a| a.id()).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
    }

    #[tokio::test]
    async fn default_poll_is_not_pollable() {
        let adapter = registry().get("fake.chat", ModelTask::Chat).unwrap();
        let job = JobHandle { adapter_id: "fake.chat".into(), remote_id: "r".into(), poll_state: json!({}) };
        let err = adapter.poll(&reqwest::Client::new(), &call(), &job).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NotPollable);
    }
}
