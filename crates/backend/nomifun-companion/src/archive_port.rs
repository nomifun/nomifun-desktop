//! Typed archive-Session boundary used by the companion archiver.
//!
//! The archiver consumes normalized [`WindowMessage`] values and never knows
//! how the host stores sessions. The canonical owner returns the original
//! transient context boundary together with the normalized archive window.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::AppError;

use crate::archiver::{ArchiveConversationPort, ArchiveWindow};

/// Upper bound on messages pulled for one digest — a chatty window only needs
/// its most-recent turns summarized (the digest prompt caps again). Keeping this
/// bounded also bounds the initial boundary=0 archive pass.
const FETCH_LIMIT: u32 = 400;

/// Minimal host contract needed by the session-window archiver.
///
/// `window_messages` returns oldest-first, already normalized dialogue
/// messages. In particular, hidden/tool/system rows and their raw metadata
/// representation do not cross this boundary. `reset_context` must preserve
/// the visible transcript and memory while advancing the canonical context floor.
#[async_trait]
pub trait CompanionArchiveSessionPort: Send + Sync {
    async fn window_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        since_ts: i64,
        limit: u32,
        idempotency_key: &str,
    ) -> Result<ArchiveWindow, AppError>;

    async fn reset_context(
        &self,
        owner_id: &str,
        session_id: &str,
        idempotency_key: &str,
        expected: &nomifun_agent_session::ContextClearBoundary,
    ) -> Result<(), AppError>;
}

/// Adapts the typed host contract to the archiver's domain seam.
pub(crate) struct SessionArchivePort {
    authoritative_user_id: Arc<str>,
    sessions: Arc<dyn CompanionArchiveSessionPort>,
}

impl SessionArchivePort {
    pub(crate) fn new(
        authoritative_user_id: Arc<str>,
        sessions: Arc<dyn CompanionArchiveSessionPort>,
    ) -> Self {
        Self {
            authoritative_user_id,
            sessions,
        }
    }
}

#[async_trait]
impl ArchiveConversationPort for SessionArchivePort {
    async fn window_messages(&self, conversation_id: &str, since_ts: i64, idempotency_key: &str) -> Result<ArchiveWindow, AppError> {
        self.sessions
            .window_messages(
                self.authoritative_user_id.as_ref(),
                conversation_id,
                since_ts,
                FETCH_LIMIT,
                idempotency_key,
            )
            .await
    }

    async fn reset_context(&self, conversation_id: &str, idempotency_key: &str,
        expected: &nomifun_agent_session::ContextClearBoundary) -> Result<(), AppError> {
        self.sessions
            .reset_context(
                self.authoritative_user_id.as_ref(),
                conversation_id,
                idempotency_key,
                expected,
            )
            .await
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::archiver::WindowMessage;
    use std::sync::Mutex;

    pub(crate) async fn context_boundary() -> nomifun_agent_session::ContextClearBoundary {
        use nomifun_agent_contracts::*;
        use nomifun_agent_session::{AgentSessionStore, ClearSessionContext, ContextClearPreparation, CreateSessionRequest};
        let db = nomifun_db::init_database_memory().await.unwrap();
        let store = AgentSessionStore::from_pool(db.pool().clone()).await.unwrap();
        let id: AgentSessionId = nomifun_common::generate_id().into();
        let owner = PrincipalRef { principal_kind:"user".into(),principal_id:nomifun_common::generate_id() };
        let session: AgentSessionLiveRecord = serde_json::from_value(serde_json::json!({
            "agent_session_id":id,"owner_ref":owner,"metadata":{"title":"archive fixture","archived":false,"pinned":false},
            "agent_binding":{"preset_revision_ref":{"preset_id":"archive-fixture","revision":1,"revision_digest":"a".repeat(64)},
                "resolved_snapshot_ref":{"snapshot_id":"archive-fixture-snapshot","snapshot_digest":"b".repeat(64)},"typed_resource_bindings":[],"binding_version":1},
            "remote_binding_provenance":null,"parent_session_id":null,"fork_base_payload_id":null,"next_seq":1
        })).unwrap();
        let created = store.create_session(CreateSessionRequest::new(session,1,"archive-open", "session_api".into(),"archive-open".into(),"archive-open".into())).await.unwrap();
        store.append_event(&SessionEventAppend { agent_session_id:id.clone(),event_id:"archive-ready".into(),producer_id:"runtime_supervisor".into(),idempotency_key:"archive-ready".into(),
            semantic_event:SemanticSessionEventDraft {kind:SessionEventKind("session/ready".into()),kind_version:1,correlation_id:"archive-open".into(),causation_event_id:Some(created.opening_ack.event_id),payload:SessionEventPayloadRef::InlineJson(StrictJsonValue(serde_json::json!({})))}}).await.unwrap();
        match store.preflight_context_clear(&owner,&id,&ClearSessionContext {idempotency_key:"archive-clear".into(),reason:"companion_archive".into()}).await.unwrap() {
            ContextClearPreparation::Ready(boundary) => boundary,
            ContextClearPreparation::AlreadyCleared(_) => unreachable!(),
        }
    }

    struct RecordingHost {
        windows: Mutex<Vec<(String, String, i64, u32)>>,
        resets: Mutex<Vec<(String, String, String)>>,
        context_boundary:nomifun_agent_session::ContextClearBoundary,
    }

    #[async_trait]
    impl CompanionArchiveSessionPort for RecordingHost {
        async fn window_messages(
            &self,
            owner_id: &str,
            session_id: &str,
            since_ts: i64,
            limit: u32,
            _idempotency_key: &str,
        ) -> Result<ArchiveWindow, AppError> {
            self.windows.lock().unwrap().push((
                owner_id.to_owned(),
                session_id.to_owned(),
                since_ts,
                limit,
            ));
            Ok(ArchiveWindow { messages:vec![WindowMessage {
                is_user: true,
                content: "hello".to_owned(),
                created_at: since_ts + 1,
            }], context_boundary:self.context_boundary.clone() })
        }

        async fn reset_context(
            &self,
            owner_id: &str,
            session_id: &str,
            idempotency_key: &str,
            _expected: &nomifun_agent_session::ContextClearBoundary,
        ) -> Result<(), AppError> {
            self.resets
                .lock()
                .unwrap()
                .push((owner_id.to_owned(), session_id.to_owned(), idempotency_key.to_owned()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn typed_archive_adapter_forwards_scope_and_limit() {
        let host = Arc::new(RecordingHost {
            windows: Mutex::new(Vec::new()),
            resets: Mutex::new(Vec::new()),
            context_boundary:context_boundary().await,
        });
        let port = SessionArchivePort::new(Arc::from("owner-a"), host.clone());

        let window = port.window_messages("session-a", 123, "archive-window-a").await.unwrap();
        assert_eq!(window.messages.len(), 1);
        assert_eq!(
            host.windows.lock().unwrap().as_slice(),
            &[("owner-a".into(), "session-a".into(), 123, FETCH_LIMIT)]
        );

        port.reset_context("session-a", "archive-window-a", &window.context_boundary).await.unwrap();
        assert_eq!(
            host.resets.lock().unwrap().as_slice(),
            &[("owner-a".into(), "session-a".into(), "archive-window-a".into())]
        );
    }
}
