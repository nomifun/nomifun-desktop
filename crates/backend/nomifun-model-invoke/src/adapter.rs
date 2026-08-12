//! The protocol-adapter seam: one [`ProtocolAdapter`] per remote wire protocol
//! (e.g. `"openai.images"`, `"ark.video_jobs"`), looked up in the
//! [`AdapterRegistry`] strictly by `(protocol, task)` — model-name/platform
//! `if` routing is banned here (that lives only in the catalog seeding layer).

use std::collections::BTreeMap;
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
    map: BTreeMap<&'static str, Arc<dyn ProtocolAdapter>>,
}

impl AdapterRegistry {
    /// Build the registry from the assembled adapter set (key = `adapter.id()`).
    pub fn new(adapters: Vec<Arc<dyn ProtocolAdapter>>) -> Self {
        Self::try_new(adapters).unwrap_or_else(|error| panic!("invalid protocol adapter registry: {error}"))
    }

    /// Fallible registry construction used by assembly/tests. Duplicate ids
    /// are rejected instead of silently replacing the first implementation.
    pub fn try_new(adapters: Vec<Arc<dyn ProtocolAdapter>>) -> Result<Self, InvokeError> {
        let mut map = BTreeMap::new();
        for adapter in adapters {
            let id = adapter.id();
            if map.insert(id, adapter).is_some() {
                return Err(InvokeError::config(format!(
                    "duplicate protocol adapter id {id:?}"
                )));
            }
        }
        Ok(Self { map })
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

    /// Whether any adapter is registered under `protocol` (regardless of task
    /// support). Lets the resolver distinguish "unknown protocol string" (a
    /// user-fixable model-row override → Config) from "registered adapter
    /// lacking the task" (a genuine NoAdapter) without sniffing messages.
    pub fn contains(&self, protocol: &str) -> bool {
        self.map.contains_key(protocol)
    }

    /// Stable, lexicographically ordered registry enumeration.
    pub fn protocol_ids(&self) -> impl ExactSizeIterator<Item = &'static str> + '_ {
        self.map.keys().copied()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::auth::{AuthMaterial, AuthScheme};
    use crate::call::ResolvedConnection;
    use crate::types::{EmbedRequest, TaskRequest, TaskResult};

    struct FakeAdapter;

    #[async_trait::async_trait]
    impl ProtocolAdapter for FakeAdapter {
        fn id(&self) -> &'static str {
            "fake.embeddings"
        }
        fn supports(&self, task: ModelTask) -> bool {
            task == ModelTask::Embedding
        }
        async fn submit(&self, _http: &reqwest::Client, _call: &ResolvedCall) -> Result<TaskOutcome, InvokeError> {
            Ok(TaskOutcome::Done(TaskResult::Embeddings(vec![vec![1.0]])))
        }
    }

    fn call() -> ResolvedCall {
        ResolvedCall {
            provider_id: "p".into(),
            config_revision: 1,
            platform: "openai".into(),
            model: "m".into(),
            task: ModelTask::Embedding,
            protocol: "fake.embeddings".into(),
            connection: ResolvedConnection {
                role: "default".into(),
                base_url: "https://x.test".into(),
                auth: AuthMaterial { scheme: AuthScheme::Bearer, credentials: json!({"api_keys": ["sk"]}) },
                extra: json!({}),
            },
            model_params: json!({}),
            request: TaskRequest::Embedding(EmbedRequest {
                inputs: vec!["hi".into()],
                extra: json!({}),
            }),
        }
    }

    fn registry() -> AdapterRegistry {
        AdapterRegistry::new(vec![Arc::new(FakeAdapter)])
    }

    #[test]
    fn get_returns_registered_supporting_adapter() {
        let adapter = registry().get("fake.embeddings", ModelTask::Embedding).expect("registered + supported");
        assert_eq!(adapter.id(), "fake.embeddings");
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
        let err = registry().get("fake.embeddings", ModelTask::SpeechSynthesis).map(|a| a.id()).unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
    }

    #[test]
    fn duplicate_protocol_ids_are_rejected() {
        let error = AdapterRegistry::try_new(vec![Arc::new(FakeAdapter), Arc::new(FakeAdapter)])
            .err()
            .expect("duplicate must fail registry construction");
        assert_eq!(error.kind, InvokeErrorKind::Config);
        assert!(error.message.contains("fake.embeddings"));
    }

    #[test]
    fn registry_enumeration_is_stable() {
        let registry = registry();
        assert_eq!(registry.protocol_ids().collect::<Vec<_>>(), vec!["fake.embeddings"]);
        assert_eq!(registry.len(), 1);
        assert!(!registry.is_empty());
    }

    #[tokio::test]
    async fn default_poll_is_not_pollable() {
        let adapter = registry().get("fake.embeddings", ModelTask::Embedding).unwrap();
        let job = JobHandle { adapter_id: "fake.embeddings".into(), config_revision: 1, remote_id: "r".into(), poll_state: json!({}) };
        let err = adapter.poll(&reqwest::Client::new(), &call(), &job).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NotPollable);
    }
}
