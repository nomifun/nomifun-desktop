use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use nomifun_agent_contracts::{
    AgentSessionId, DigestHex, ResolvedSnapshotRef, RuntimeBindingId,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::error::CodingEngineError;
use crate::context::CodingContextBudget;
use crate::events::{CodingEventSink, NoopCodingEventSink, SharedCodingEventSink};
use crate::model::CodingModelPort;
use crate::tool::CodingToolInvoker;
use crate::turn::{CodingTurnRequest, CodingTurnResult};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EngineBuildId(pub String);

impl From<&str> for EngineBuildId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for EngineBuildId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl AsRef<str> for EngineBuildId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingEngineBuild {
    pub build_id: EngineBuildId,
    pub build_digest: DigestHex,
}

impl CodingEngineBuild {
    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if !is_trimmed_non_empty(self.build_id.as_ref())
            || !is_trimmed_non_empty(self.build_digest.as_ref())
        {
            return Err(CodingEngineError::InvalidContract(
                "the unified engine requires a complete build identity".to_owned(),
            ));
        }
        if self.build_digest.as_ref().len() != 64
            || !self
                .build_digest
                .as_ref()
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(CodingEngineError::InvalidContract(
                "engine build digest must be 64 hexadecimal characters".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineBinding {
    agent_session_id: AgentSessionId,
    runtime_binding_id: RuntimeBindingId,
    build_id: EngineBuildId,
    build_digest: DigestHex,
    resolved_snapshot_ref: ResolvedSnapshotRef,
}

impl EngineBinding {
    pub fn new(
        agent_session_id: AgentSessionId,
        runtime_binding_id: RuntimeBindingId,
        build_id: EngineBuildId,
        build_digest: DigestHex,
        resolved_snapshot_ref: ResolvedSnapshotRef,
    ) -> Result<Self, CodingEngineError> {
        let binding = Self {
            agent_session_id,
            runtime_binding_id,
            build_id,
            build_digest,
            resolved_snapshot_ref,
        };
        binding.validate()?;
        Ok(binding)
    }

    pub fn agent_session_id(&self) -> &AgentSessionId {
        &self.agent_session_id
    }

    pub fn runtime_binding_id(&self) -> &RuntimeBindingId {
        &self.runtime_binding_id
    }

    pub fn build_id(&self) -> &EngineBuildId {
        &self.build_id
    }

    pub fn build_digest(&self) -> &DigestHex {
        &self.build_digest
    }

    pub fn resolved_snapshot_ref(&self) -> &ResolvedSnapshotRef {
        &self.resolved_snapshot_ref
    }

    pub fn validate(&self) -> Result<(), CodingEngineError> {
        if !is_trimmed_non_empty(self.agent_session_id.as_ref())
            || !is_trimmed_non_empty(self.runtime_binding_id.as_ref())
            || !is_trimmed_non_empty(self.build_id.as_ref())
            || !is_valid_digest(&self.build_digest)
            || !is_trimmed_non_empty(self.resolved_snapshot_ref.snapshot_id.as_ref())
            || !is_valid_digest(&self.resolved_snapshot_ref.snapshot_digest)
        {
            return Err(CodingEngineError::InvalidContract(
                "engine binding contains an empty or malformed identity".to_owned(),
            ));
        }
        Ok(())
    }
}

pub struct CodingEngine {
    build: CodingEngineBuild,
    context_budget: CodingContextBudget,
}

impl CodingEngine {
    pub fn new(build: CodingEngineBuild) -> Result<Self, CodingEngineError> {
        build.validate()?;
        Ok(Self { build, context_budget: CodingContextBudget::default() })
    }

    /// Algorithm policy belongs to the compiled engine, not the Session owner.
    pub fn with_context_budget(mut self, budget: CodingContextBudget) -> Result<Self, CodingEngineError> {
        self.context_budget = budget.validate()?;
        Ok(self)
    }

    pub fn bind(
        &self,
        agent_session_id: AgentSessionId,
        runtime_binding_id: RuntimeBindingId,
        resolved_snapshot_ref: ResolvedSnapshotRef,
    ) -> Result<EngineBinding, CodingEngineError> {
        EngineBinding::new(
            agent_session_id,
            runtime_binding_id,
            self.build.build_id.clone(),
            self.build.build_digest.clone(),
            resolved_snapshot_ref,
        )
    }

    pub fn open_session(
        &self,
        binding: EngineBinding,
        model: Arc<dyn CodingModelPort>,
        tools: Arc<dyn CodingToolInvoker>,
        event_sink: Option<Arc<dyn CodingEventSink>>,
    ) -> Result<CodingEngineSession, CodingEngineError> {
        binding.validate()?;
        if binding.build_id != self.build.build_id
            || binding.build_digest != self.build.build_digest
        {
            return Err(CodingEngineError::InvalidContract(
                "runtime binding does not match the selected engine build".to_owned(),
            ));
        }
        Ok(CodingEngineSession {
            binding,
            context_budget: self.context_budget,
            model,
            tools,
            event_sink: event_sink.unwrap_or_else(|| Arc::new(NoopCodingEventSink)),
            state: Mutex::new(CodingEngineSessionState::default()),
        })
    }

}

pub struct CodingEngineSession {
    binding: EngineBinding,
    context_budget: CodingContextBudget,
    model: Arc<dyn CodingModelPort>,
    tools: Arc<dyn CodingToolInvoker>,
    event_sink: SharedCodingEventSink,
    state: Mutex<CodingEngineSessionState>,
}

#[derive(Default)]
struct CodingEngineSessionState {
    disposed: bool,
    active_turn: Option<CancellationToken>,
}

/// Releasing an interrupted turn future must also release its admission and
/// cancel the broker request. Otherwise a host timeout permanently wedges the
/// Session and can leave a provider stream running after its caller is gone.
struct ActiveCodingTurn<'a> {
    state: &'a Mutex<CodingEngineSessionState>,
    cancellation: CancellationToken,
}

impl Drop for ActiveCodingTurn<'_> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        self.state.lock().unwrap_or_else(|error| error.into_inner()).active_turn = None;
    }
}

impl CodingEngineSession {
    pub fn binding(&self) -> &EngineBinding {
        &self.binding
    }

    pub async fn run_turn(
        &self,
        request: CodingTurnRequest,
    ) -> Result<CodingTurnResult, CodingEngineError> {
        self.run_turn_cancellable(request, CancellationToken::new()).await
    }

    /// The host owns the parent token. A completed/dropped engine turn only
    /// cancels its child, never the Session or a successor turn.
    pub async fn run_turn_cancellable(
        &self,
        request: CodingTurnRequest,
        parent_cancellation: CancellationToken,
    ) -> Result<CodingTurnResult, CodingEngineError> {
        let cancellation = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if state.disposed {
                return Err(CodingEngineError::SessionDisposed);
            }
            if state.active_turn.is_some() {
                return Err(CodingEngineError::TurnAlreadyRunning);
            }
            let cancellation = parent_cancellation.child_token();
            state.active_turn = Some(cancellation.clone());
            cancellation
        };
        let _admission = ActiveCodingTurn {
            state: &self.state,
            cancellation: cancellation.clone(),
        };
        let run_cancellation = cancellation.clone();
        let result = AssertUnwindSafe(crate::turn::run_turn(
            self.binding.clone(),
            Arc::clone(&self.model),
            Arc::clone(&self.tools),
            Arc::clone(&self.event_sink),
            request,
            self.context_budget,
            run_cancellation,
        ))
        .catch_unwind()
        .await
        .unwrap_or(Err(CodingEngineError::TurnPanicked));

        result
    }

    pub async fn cancel(&self) -> bool {
        let cancellation = self.state.lock().unwrap_or_else(|error| error.into_inner()).active_turn.clone();
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub async fn dispose(&self) {
        let cancellation = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.disposed = true;
            state.active_turn.clone()
        };
        if let Some(cancellation) = cancellation {
            cancellation.cancel();
        }
    }

    pub async fn is_turn_active(&self) -> bool {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).active_turn.is_some()
    }

    pub async fn is_disposed(&self) -> bool {
        self.state.lock().unwrap_or_else(|error| error.into_inner()).disposed
    }
}

fn is_trimmed_non_empty(value: &str) -> bool {
    !value.is_empty() && value.trim() == value
}

fn is_valid_digest(value: &DigestHex) -> bool {
    value.as_ref().len() == 64 && value.as_ref().bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(
        build_id: &str,
        digest_byte: char,
    ) -> CodingEngineBuild {
        CodingEngineBuild {
            build_id: EngineBuildId::from(build_id),
            build_digest: DigestHex::from(digest_byte.to_string().repeat(64)),
        }
    }

    #[test]
    fn one_official_build_binds_every_session_without_a_profile_selector() {
        let engine = CodingEngine::new(build("release-1", 'a')).unwrap();
        let snapshot = ResolvedSnapshotRef {
            snapshot_id: nomifun_agent_contracts::ResolvedSnapshotId::from("snapshot"),
            snapshot_digest: DigestHex::from("c".repeat(64)),
        };
        let first = engine
            .bind(
                AgentSessionId::from("first-session"),
                RuntimeBindingId::from("first-binding"),
                snapshot.clone(),
            )
            .unwrap();
        let second = engine
            .bind(
                AgentSessionId::from("second-session"),
                RuntimeBindingId::from("second-binding"),
                snapshot,
            )
            .unwrap();

        assert_eq!(first.build_id(), second.build_id());
        assert_eq!(first.build_digest(), second.build_digest());
    }

}
