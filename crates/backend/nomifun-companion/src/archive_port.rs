//! Typed archive-Session boundary used by the companion archiver.
//!
//! The archiver consumes normalized [`WindowMessage`] values and never knows
//! how the host stores sessions. The host-facing contract below is deliberately
//! narrow so the current Conversation implementation can remain a transitional
//! adapter in `session_port.rs` while a future canonical Session owner can
//! implement the same operations directly.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_common::AppError;

use crate::archiver::{ArchiveConversationPort, WindowMessage};

/// Upper bound on messages pulled for one digest — a chatty window only needs
/// its most-recent turns summarized (the digest prompt caps again). Keeping this
/// bounded also bounds the initial boundary=0 archive pass.
const FETCH_LIMIT: u32 = 400;

/// Minimal host contract needed by the session-window archiver.
///
/// `window_messages` returns oldest-first, already normalized dialogue
/// messages. In particular, hidden/tool/system rows and their raw metadata
/// representation do not cross this boundary. `reset_context` must preserve
/// the visible transcript while clearing the runtime's persisted context.
#[async_trait]
pub trait CompanionArchiveSessionPort: Send + Sync {
    async fn window_messages(
        &self,
        owner_id: &str,
        session_id: &str,
        since_ts: i64,
        limit: u32,
    ) -> Result<Vec<WindowMessage>, AppError>;

    async fn reset_context(
        &self,
        owner_id: &str,
        session_id: &str,
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
    async fn window_messages(&self, conversation_id: &str, since_ts: i64) -> Result<Vec<WindowMessage>, AppError> {
        self.sessions
            .window_messages(
                self.authoritative_user_id.as_ref(),
                conversation_id,
                since_ts,
                FETCH_LIMIT,
            )
            .await
    }

    async fn reset_context(&self, conversation_id: &str) -> Result<(), AppError> {
        self.sessions
            .reset_context(
                self.authoritative_user_id.as_ref(),
                conversation_id,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingHost {
        windows: Mutex<Vec<(String, String, i64, u32)>>,
        resets: Mutex<Vec<(String, String)>>,
    }

    #[async_trait]
    impl CompanionArchiveSessionPort for RecordingHost {
        async fn window_messages(
            &self,
            owner_id: &str,
            session_id: &str,
            since_ts: i64,
            limit: u32,
        ) -> Result<Vec<WindowMessage>, AppError> {
            self.windows.lock().unwrap().push((
                owner_id.to_owned(),
                session_id.to_owned(),
                since_ts,
                limit,
            ));
            Ok(vec![WindowMessage {
                is_user: true,
                content: "hello".to_owned(),
                created_at: since_ts + 1,
            }])
        }

        async fn reset_context(
            &self,
            owner_id: &str,
            session_id: &str,
        ) -> Result<(), AppError> {
            self.resets
                .lock()
                .unwrap()
                .push((owner_id.to_owned(), session_id.to_owned()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn typed_archive_adapter_forwards_scope_and_limit() {
        let host = Arc::new(RecordingHost {
            windows: Mutex::new(Vec::new()),
            resets: Mutex::new(Vec::new()),
        });
        let port = SessionArchivePort::new(Arc::from("owner-a"), host.clone());

        let messages = port.window_messages("session-a", 123).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            host.windows.lock().unwrap().as_slice(),
            &[("owner-a".into(), "session-a".into(), 123, FETCH_LIMIT)]
        );

        port.reset_context("session-a").await.unwrap();
        assert_eq!(
            host.resets.lock().unwrap().as_slice(),
            &[("owner-a".into(), "session-a".into())]
        );
    }
}
