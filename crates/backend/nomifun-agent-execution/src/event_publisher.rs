//! Realtime projection of committed Agent Execution events.
//!
//! Durable mutations first append an outbox row in the repository transaction.
//! This publisher only projects committed rows to the WebSocket bus. Clients use
//! `(execution_id, sequence)` for ordering and fetch the canonical detail again;
//! the UI no longer mirrors separate lifecycle event families.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use nomifun_api_types::{AgentExecutionChangedEvent, WebSocketMessage};
use nomifun_common::now_ms;
use nomifun_db::IAgentExecutionRepository;
use nomifun_realtime::UserEventSink;
use serde_json::{json, to_value};

use crate::domain_mapper;
use crate::lifecycle::AgentExecutionLifecycle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeadThinkingPhase {
    Planning,
    Adjust,
}

impl LeadThinkingPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Adjust => "adjust",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeadThinkingKind {
    Reasoning,
    Text,
}

impl LeadThinkingKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
            Self::Text => "text",
        }
    }
}

#[derive(Clone)]
pub(crate) struct AgentExecutionEventPublisher {
    sink: Arc<dyn UserEventSink>,
    drain_gate: Arc<tokio::sync::Mutex<()>>,
    retry_worker_started: Arc<AtomicBool>,
    retry_notify: Arc<tokio::sync::Notify>,
    changes: Arc<Mutex<HashMap<(String, String), Weak<tokio::sync::Notify>>>>,
    lifecycle: AgentExecutionLifecycle,
}

impl AgentExecutionEventPublisher {
    #[cfg(test)]
    pub(crate) async fn hold_drain_for_test(&self)->tokio::sync::OwnedMutexGuard<()> {
        self.drain_gate.clone().lock_owned().await
    }
    pub fn new(sink: Arc<dyn UserEventSink>) -> Self {
        Self {
            sink,
            drain_gate: Arc::new(tokio::sync::Mutex::new(())),
            retry_worker_started: Arc::new(AtomicBool::new(false)),
            retry_notify: Arc::new(tokio::sync::Notify::new()),
            changes: Arc::default(),
            lifecycle: AgentExecutionLifecycle::new(),
        }
    }

    pub(crate) fn with_lifecycle(mut self, lifecycle: AgentExecutionLifecycle) -> Self {
        self.lifecycle = lifecycle;
        self
    }

    fn publish_change(&self, owner_id: &str, event: AgentExecutionChangedEvent) {
        self.notify_observation_changed(owner_id,&event.execution_id);
        let payload = to_value(event)
            .expect("AgentExecutionChangedEvent must remain JSON serializable");
        self.sink.send_to_user(owner_id, WebSocketMessage::new(
            "agentExecution.changed",
            payload,
        ));
    }

    /// Native receipt/cleanup can advance after the domain outcome was
    /// published. Wake only existing scoped observers to reread canonical
    /// facts; this is neither another event nor execution/cleanup authority.
    pub(crate) fn notify_observation_changed(&self,owner_id:&str,execution_id:&str) {
        let changed = {
            let mut changes = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let key = (owner_id.to_owned(), execution_id.to_owned());
            let changed = changes.get(&key).and_then(Weak::upgrade);
            if changed.is_none() { changes.remove(&key); }
            changed
        };
        if let Some(changed) = changed { changed.notify_waiters(); }
    }

    /// A disposable wake hint for one owner and Execution, never a result or
    /// admission receipt. Callers subscribe before reading canonical state.
    pub(crate) fn changed_for(&self, owner_id: &str, execution_id: &str) -> Arc<tokio::sync::Notify> {
        let mut changes = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        changes.retain(|_, changed| changed.strong_count() > 0);
        let entry = changes.entry((owner_id.to_owned(), execution_id.to_owned())).or_default();
        if let Some(changed) = entry.upgrade() { return changed; }
        let changed = Arc::new(tokio::sync::Notify::new());
        *entry = Arc::downgrade(&changed);
        changed
    }

    /// Drain committed outbox rows in sequence order. Safe to call after every
    /// mutation and once during boot; publication markers make it idempotent.
    pub async fn drain(&self, repository: Arc<dyn IAgentExecutionRepository>) {
        if self.lifecycle.is_cancelled() {
            return;
        }
        self.ensure_retry_worker(repository.clone());
        if !self.drain_once(repository.as_ref()).await {
            self.retry_notify.notify_one();
        }
    }

    /// Hand committed publication to the existing outbox owner. A business
    /// acknowledgement must never wait for unrelated WebSocket/outbox work.
    pub(crate) fn notify_pending(&self, repository: Arc<dyn IAgentExecutionRepository>) {
        if self.lifecycle.is_cancelled() { return; }
        self.ensure_retry_worker(repository);
        self.retry_notify.notify_one();
    }

    async fn drain_once(&self, repository: &dyn IAgentExecutionRepository) -> bool {
        // Parallel Step settlement can call drain concurrently. Serialize the
        // local outbox consumer so one committed fact is not broadcast twice
        // before either caller records its publication marker.
        let _guard = self.drain_gate.lock().await;
        loop {
            if self.lifecycle.is_cancelled() {
                return true;
            }
            let events = match repository.list_unpublished_events(100).await {
                Ok(events) => events,
                Err(error) => {
                    tracing::warn!(%error, "failed to read Agent Execution outbox");
                    return false;
                }
            };
            if events.is_empty() {
                return true;
            }
            let count = events.len();
            for event in events {
                let change_kind = match domain_mapper::event_kind(&event.event_type) {
                    Ok(kind) => kind,
                    Err(error) => {
                        tracing::error!(
                            execution_id = %event.execution_id,
                            sequence = event.sequence,
                            event_type = %event.event_type,
                            %error,
                            "refusing to publish invalid Agent Execution outbox event"
                        );
                        return false;
                    }
                };
                self.publish_change(
                    &event.on_behalf_of_user_id,
                    AgentExecutionChangedEvent {
                        execution_id: event.execution_id.clone(),
                        sequence: event.sequence,
                        change_kind,
                    },
                );
                if let Err(error) = repository
                    .mark_event_published(&event.execution_id, event.sequence, now_ms())
                    .await
                {
                    tracing::warn!(
                        execution_id = %event.execution_id,
                        sequence = event.sequence,
                        %error,
                        "failed to mark execution event published"
                    );
                    return false;
                }
            }
            if count < 100 {
                return true;
            }
        }
    }

    fn ensure_retry_worker(&self, repository: Arc<dyn IAgentExecutionRepository>) {
        if self
            .retry_worker_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }
        let publisher = self.clone();
        self.lifecycle.spawn(async move {
            const MIN_DELAY: Duration = Duration::from_secs(1);
            const MAX_DELAY: Duration = Duration::from_secs(60);
            loop {
                tokio::select! {
                    _ = publisher.lifecycle.cancelled() => return,
                    _ = publisher.retry_notify.notified() => {}
                }
                let mut delay = MIN_DELAY;
                while !publisher.drain_once(repository.as_ref()).await {
                    tokio::select! {
                        _ = publisher.lifecycle.cancelled() => return,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    delay = delay.saturating_mul(2).min(MAX_DELAY);
                }
            }
        });
    }

    /// High-frequency lead deltas are intentionally transient and do not occupy
    /// the durable outbox sequence.
    pub fn publish_lead_thinking(
        &self,
        owner_id: &str,
        execution_id: &str,
        phase: LeadThinkingPhase,
        kind: LeadThinkingKind,
        delta: Option<&str>,
        content: Option<&str>,
        done: bool,
    ) {
        let mut payload = json!({
            "execution_id": execution_id,
            "phase": phase.as_str(),
            "kind": kind.as_str(),
            "done": done,
        });
        if let Some(delta) = delta {
            payload["delta"] = json!(delta);
        }
        if let Some(content) = content {
            payload["content"] = json!(content);
        }
        self.sink.send_to_user(owner_id, WebSocketMessage::new(
            "agentExecution.leadThinking",
            payload,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::FutureExt;
    use nomifun_common::{
        AdaptationPolicy, AgentExecutionActor, AgentExecutionEventKind,
        AgentExecutionStatus, DecisionPolicy, DelegationPolicy,
    };
    use nomifun_db::{
        CreateAgentExecutionParams, IAgentExecutionRepository, NewAgentExecutionEvent,
        NewAgentExecutionParticipant, SqliteAgentExecutionRepository,
        init_database_memory,
    };

    const EXECUTION_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";
    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000003";
    const NOMI_AGENT_ID: &str = "0190f5fe-7c00-7a00-8000-000000000114";

    #[tokio::test]
    async fn committed_changes_wake_only_the_exact_owner_and_execution() {
        let publisher=AgentExecutionEventPublisher::new(Arc::new(RecordingSink(std::sync::Mutex::new(vec![]))));
        let exact=publisher.changed_for(USER_ID,EXECUTION_ID);
        let sibling=publisher.changed_for(USER_ID,"another-execution");
        let foreign=publisher.changed_for("another-owner",EXECUTION_ID);
        let exact=exact.notified();tokio::pin!(exact);exact.as_mut().enable();
        let sibling=sibling.notified();tokio::pin!(sibling);sibling.as_mut().enable();
        let foreign=foreign.notified();tokio::pin!(foreign);foreign.as_mut().enable();
        publisher.publish_change(USER_ID,AgentExecutionChangedEvent {execution_id:EXECUTION_ID.into(),sequence:2,
            change_kind:AgentExecutionEventKind::StepChanged});
        assert!(exact.now_or_never().is_some(),"an enabled subscriber retains a commit that wins before await");
        assert!(sibling.now_or_never().is_none());
        assert!(foreign.now_or_never().is_none());
    }

    #[tokio::test]
    async fn native_observation_hint_wakes_its_existing_reader_without_a_domain_event_or_new_owner() {
        let sink=Arc::new(RecordingSink(std::sync::Mutex::new(vec![])));
        let publisher=AgentExecutionEventPublisher::new(sink.clone());
        let exact=publisher.changed_for(USER_ID,EXECUTION_ID);
        let sibling=publisher.changed_for(USER_ID,"unrelated-work");
        let foreign=publisher.changed_for("another-owner",EXECUTION_ID);
        let exact=exact.notified();tokio::pin!(exact);exact.as_mut().enable();
        let sibling=sibling.notified();tokio::pin!(sibling);sibling.as_mut().enable();
        let foreign=foreign.notified();tokio::pin!(foreign);foreign.as_mut().enable();
        publisher.notify_observation_changed(USER_ID,EXECUTION_ID);
        assert!(exact.now_or_never().is_some());
        assert!(sibling.now_or_never().is_none()&&foreign.now_or_never().is_none());
        assert!(sink.0.lock().unwrap().is_empty(),"a native observation is not another published domain outcome");
        publisher.notify_observation_changed(USER_ID,"finished-work-without-readers");
        assert!(!publisher.changes.lock().unwrap().contains_key(&(USER_ID.into(),"finished-work-without-readers".into())),
            "a hint cannot install a persistent observer or task owner");
    }

    #[test]
    fn discarded_observers_do_not_pin_completed_executions() {
        let publisher=AgentExecutionEventPublisher::new(Arc::new(RecordingSink(std::sync::Mutex::new(vec![]))));
        for index in 0..100 { drop(publisher.changed_for(USER_ID,&format!("completed-{index}"))); }
        let observer=publisher.changed_for(USER_ID,EXECUTION_ID);
        assert_eq!(publisher.changes.lock().unwrap().len(),1);
        assert_eq!(Arc::strong_count(&observer),1,"the publisher retains no observer or execution owner");
    }

    struct RecordingSink(std::sync::Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>);

    impl UserEventSink for RecordingSink {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.0.lock().unwrap().push((user_id.to_owned(), event));
        }
    }

    #[test]
    fn change_event_is_the_only_durable_wire_shape() {
        let sink = Arc::new(RecordingSink(std::sync::Mutex::new(vec![])));
        AgentExecutionEventPublisher::new(sink.clone()).publish_change(
            USER_ID,
            AgentExecutionChangedEvent {
                execution_id: EXECUTION_ID.to_owned(),
                sequence: 9,
                change_kind: AgentExecutionEventKind::StepChanged,
            },
        );
        let events = sink.0.lock().unwrap();
        assert_eq!(events[0].0, USER_ID);
        assert_eq!(events[0].1.name, "agentExecution.changed");
        assert_eq!(events[0].1.data["execution_id"], EXECUTION_ID);
        assert_eq!(events[0].1.data["sequence"], 9);
        assert_eq!(events[0].1.data["change_kind"], "step_changed");
    }

    #[test]
    fn full_lead_content_is_sent_only_to_the_named_owner() {
        let sink = Arc::new(RecordingSink(std::sync::Mutex::new(vec![])));
        AgentExecutionEventPublisher::new(sink.clone()).publish_lead_thinking(
            USER_ID,
            EXECUTION_ID,
            LeadThinkingPhase::Planning,
            LeadThinkingKind::Reasoning,
            Some("private delta"),
            Some("private content"),
            false,
        );

        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, USER_ID);
        assert_eq!(events[0].1.name, "agentExecution.leadThinking");
        assert_eq!(events[0].1.data["phase"], "planning");
        assert_eq!(events[0].1.data["kind"], "reasoning");
        assert_eq!(events[0].1.data["delta"], "private delta");
        assert_eq!(events[0].1.data["content"], "private content");
    }

    fn participant(participant_id: &str) -> NewAgentExecutionParticipant {
        NewAgentExecutionParticipant {
            participant_id: participant_id.to_owned(),
            source_agent_id: NOMI_AGENT_ID.to_owned(),
            preset_id: None,
            preset_revision: None,
            agent_snapshot: None,
            provider_id: Some(PROVIDER_ID.to_owned()),
            model: Some("model".to_owned()),
            role: None,
            capability: None,
            constraints: None,
            description: None,
            system_prompt: None,
            enabled_skills: "[]".to_owned(),
            disabled_builtin_skills: "[]".to_owned(),
            sort_order: 0,
        }
    }

    fn create_params() -> CreateAgentExecutionParams {
        CreateAgentExecutionParams {
            goal: "test".to_owned(),
            status: AgentExecutionStatus::Planning,
            adaptation_policy: AdaptationPolicy::Fixed,
            decision_policy: DecisionPolicy::Automatic,
            delegation_policy: DelegationPolicy::Automatic,
            max_parallel: 1,
            work_dir: None,
            lead_conversation_id: None,
            initial_plan_input: r#"{"mode":"automatic"}"#.to_owned(),
        }
    }

    fn created_event() -> NewAgentExecutionEvent {
        NewAgentExecutionEvent {
            event_type: AgentExecutionEventKind::Created,
            step_id: None,
            attempt_id: None,
            actor: AgentExecutionActor::system(),
            payload: "{}".to_owned(),
        }
    }

    #[tokio::test]
    async fn outbox_drain_targets_installation_owner_and_marks_every_event_published() {
        let database = init_database_memory().await.unwrap();
        let installation_owner = nomifun_db::installation_owner_id(database.pool()).await.unwrap();
        let credentials_encrypted = nomifun_common::encrypt_string(
            r#"{"api_keys":["test-only"]}"#,
            &[0x42; 32],
        )
        .unwrap();
        sqlx::query(
            "INSERT INTO providers (\
                provider_id, platform, name, base_url, auth_scheme, credentials_encrypted, enabled, \
                created_at, updated_at\
             ) VALUES (?1, 'openai', 'provider', 'https://example.invalid', \
                       'bearer', ?2, 1, 1, 1)",
        )
        .bind(PROVIDER_ID)
        .bind(&credentials_encrypted)
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_models (\
                provider_id, model, enabled, sort_order, description, created_at, updated_at\
             ) VALUES (?1, 'model', 1, 0, NULL, 1, 1)",
        )
        .bind(PROVIDER_ID)
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_model_capabilities (\
                provider_id, model, task, traits, protocol, connection_role, \
                allow_cross_origin_credentials, provider_params, created_at, updated_at\
             ) VALUES (?1, 'model', 'chat', '[]', 'openai.chat_text', 'default', \
                       0, '{}', 1, 1)",
        )
        .bind(PROVIDER_ID)
        .execute(database.pool())
        .await
        .unwrap();
        let repository = Arc::new(SqliteAgentExecutionRepository::new(database.pool().clone()));
        let execution = repository
            .create_execution_with_participants(
                &installation_owner,
                &create_params(),
                &[participant(
                    "0190f5fe-7c00-7a00-8000-000000000041",
                )],
                &created_event(),
            )
            .await
            .unwrap();
        let sink = Arc::new(RecordingSink(std::sync::Mutex::new(vec![])));
        let publisher = AgentExecutionEventPublisher::new(sink.clone());

        publisher.drain(repository.clone()).await;

        let events = sink.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, installation_owner);
        assert_eq!(events[0].1.data["execution_id"], execution.execution_id);
        drop(events);
        assert!(repository.list_unpublished_events(100).await.unwrap().is_empty());
    }
}
