use crate::runtime_state::AgentRuntimeState;
use crate::protocol::acp::{PermissionDecision, PermissionRequest};
use crate::protocol::events::{AgentStreamEvent, permission_request_to_event_data};
use nomifun_common::Confirmation;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::time::Duration;
use tracing::{debug, warn};

struct PendingPermission {
    responder: oneshot::Sender<PermissionDecision>,
    confirmation: Confirmation,
    generation: u64,
}

/// Routes ACP permission requests from the protocol layer to the user
/// (via `event_tx`) and back (via `confirm`). Owns the receiver channel
/// for incoming permission requests, the pending responder map, and the
/// `closing` flag that prevents new requests from being routed after a
/// graceful shutdown has started.
pub struct PermissionRouter {
    /// Receiver for permission requests from the protocol layer.
    permission_rx: Mutex<mpsc::Receiver<PermissionRequest>>,
    /// Pending ACP permission responders and recovery data keyed by tool call ID.
    pending_permissions: StdMutex<HashMap<String, PendingPermission>>,
    /// Monotonic registration identity. ACP call IDs may be reused, so a
    /// timeout must only remove the registration it was created for.
    next_generation: AtomicU64,
    /// Maximum time a user-facing permission prompt may remain pending.
    permission_timeout: Duration,
    /// The receiver has one permanent owner; repeated starts must not park
    /// additional tasks behind its mutex for the lifetime of the router.
    started: AtomicBool,
    /// Whether a graceful shutdown is in progress.
    closing: AtomicBool,
}

impl PermissionRouter {
    /// Create a new permission router.
    pub fn new(permission_rx: mpsc::Receiver<PermissionRequest>) -> Self {
        Self::new_with_timeout(
            permission_rx,
            crate::protocol::acp::ACP_PERMISSION_TIMEOUT,
        )
    }

    fn new_with_timeout(
        permission_rx: mpsc::Receiver<PermissionRequest>,
        permission_timeout: Duration,
    ) -> Self {
        Self {
            permission_rx: Mutex::new(permission_rx),
            pending_permissions: StdMutex::new(HashMap::new()),
            next_generation: AtomicU64::new(0),
            closing: AtomicBool::new(false),
            permission_timeout,
            started: AtomicBool::new(false),
        }
    }

    /// Start the permission handler loop.
    ///
    /// This background task receives permission requests from the protocol
    /// layer, converts them to `Permission` events, and waits for user
    /// responses routed through the `confirm()` method.
    ///
    /// `runtime` is shared with the parent manager so permission
    /// arrivals count as activity (preventing idle timeouts) via
    /// `runtime.bump_activity()`.
    pub fn start(self: &Arc<Self>, runtime: AgentRuntimeState) {
        if self
            .started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            debug!(
                conversation_id = %runtime.conversation_id(),
                "ACP permission router already started",
            );
            return;
        }
        let this = Arc::clone(self);

        tokio::spawn(async move {
            let mut rx = this.permission_rx.lock().await;

            while let Some(perm_req) = rx.recv().await {
                let call_id = perm_req.request.tool_call.tool_call_id.to_string();
                if this.is_closing() {
                    let _ = perm_req.response_tx.send(PermissionDecision::Cancelled);
                    continue;
                }

                runtime.bump_activity();

                let permission_event = permission_request_to_event_data(&perm_req.request);
                let confirmation = permission_event
                    .as_confirmation()
                    .expect("ACP permission events must be recoverable as confirmations");

                let generation = this.next_generation();
                let mut pending = this.pending_permissions.lock().unwrap();
                // Serialize the closing check with insertion. `set_closing`
                // takes the same lock before flipping the flag and draining,
                // so it cannot return while a new request is being inserted.
                if this.is_closing() {
                    drop(pending);
                    let _ = perm_req.response_tx.send(PermissionDecision::Cancelled);
                    continue;
                }
                if let Some(previous) = pending.insert(
                    call_id.clone(),
                    PendingPermission {
                        responder: perm_req.response_tx,
                        confirmation,
                        generation,
                    },
                ) {
                    let _ = previous.responder.send(PermissionDecision::Cancelled);
                }
                drop(pending);
                debug!(
                    conversation_id = %runtime.conversation_id(),
                    call_id = %call_id,
                    generation,
                    "ACP permission pending confirmation registered",
                );

                if runtime
                    .event_sender()
                    .send(AgentStreamEvent::AcpPermission(permission_event))
                    .is_err()
                {
                    this.cancel_pending_if_generation(&call_id, generation);
                    continue;
                }

                let timeout = this.permission_timeout;
                let weak = Arc::downgrade(&this);
                let timeout_call_id = call_id.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(timeout).await;
                    if let Some(router) = weak.upgrade() {
                        router.expire_pending(&timeout_call_id, generation);
                    }
                });
            }

            // A closed protocol channel must wake every SDK request already
            // routed through this router instead of leaving its responder
            // parked forever.
            this.cancel_all();
        });
    }

    fn next_generation(&self) -> u64 {
        let generation = self
            .next_generation
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1);
        if generation == 0 { 1 } else { generation }
    }

    fn cancel_pending_if_generation(&self, call_id: &str, generation: u64) {
        let pending = {
            let mut pending = self.pending_permissions.lock().unwrap();
            let is_current = pending
                .get(call_id)
                .is_some_and(|entry| entry.generation == generation);
            if is_current {
                pending.remove(call_id)
            } else {
                None
            }
        };
        if let Some(pending) = pending {
            let _ = pending.responder.send(PermissionDecision::Cancelled);
        }
    }

    fn expire_pending(&self, call_id: &str, generation: u64) {
        let pending = {
            let mut pending = self.pending_permissions.lock().unwrap();
            let is_current = pending
                .get(call_id)
                .is_some_and(|entry| entry.generation == generation);
            if is_current {
                pending.remove(call_id)
            } else {
                None
            }
        };
        if let Some(pending) = pending {
            let _ = pending.responder.send(PermissionDecision::Cancelled);
            warn!(call_id, generation, "ACP permission timed out");
        }
    }

    /// Pending permission items recoverable by conversation confirmation APIs.
    pub fn get_confirmations(&self) -> Vec<Confirmation> {
        self.pending_permissions
            .lock()
            .unwrap()
            .values()
            .map(|pending| pending.confirmation.clone())
            .collect()
    }

    /// Resolve a pending permission request with the user's selected option.
    pub fn confirm(
        &self,
        call_id: &str,
        option_id: String,
        conversation_id: &str,
    ) -> Result<(), nomifun_common::AppError> {
        let pending = self
            .pending_permissions
            .lock()
            .unwrap()
            .remove(call_id)
            .ok_or_else(|| {
                nomifun_common::AppError::BadRequest(format!(
                    "Pending ACP permission not found: {call_id}"
                ))
            })?;

        pending
            .responder
            .send(PermissionDecision::Selected { option_id })
            .map_err(|_| {
                nomifun_common::AppError::BadRequest(format!(
                    "Pending ACP permission expired: {call_id}"
                ))
            })?;

        debug!(conversation_id = %conversation_id, call_id, "ACP permission response forwarded");
        Ok(())
    }

    /// Cancel all pending permission requests. Called during `stop()` and `kill()`.
    pub fn cancel_all(&self) {
        for (_, pending) in self.pending_permissions.lock().unwrap().drain() {
            let _ = pending.responder.send(PermissionDecision::Cancelled);
        }
    }

    /// Whether a graceful shutdown is in progress.
    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Acquire)
    }

    /// Mark the router as closing (graceful shutdown in progress).
    pub fn set_closing(&self) {
        let mut pending = self.pending_permissions.lock().unwrap();
        self.closing.store(true, Ordering::Release);
        for (_, pending) in pending.drain() {
            let _ = pending.responder.send(PermissionDecision::Cancelled);
        }
    }

    #[cfg(test)]
    fn insert_pending_for_test(
        &self,
        call_id: String,
        responder: oneshot::Sender<PermissionDecision>,
        confirmation: Confirmation,
    ) {
        let generation = self.next_generation();
        self.insert_pending_for_test_with_generation(call_id, responder, confirmation, generation);
    }

    #[cfg(test)]
    fn insert_pending_for_test_with_generation(
        &self,
        call_id: String,
        responder: oneshot::Sender<PermissionDecision>,
        confirmation: Confirmation,
        generation: u64,
    ) {
        self.pending_permissions.lock().unwrap().insert(
            call_id,
            PendingPermission {
                responder,
                confirmation,
                generation,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::events::AgentStreamEvent;
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionKind as SdkPermissionOptionKind,
        RequestPermissionRequest, ToolCallUpdate as SdkToolCallUpdate, ToolCallUpdateFields,
        ToolKind as SdkToolKind,
    };
    use nomifun_common::Confirmation;
    use serde_json::json;
    use std::time::Duration;

    fn sample_confirmation(call_id: &str) -> Confirmation {
        Confirmation {
            id: call_id.to_owned(),
            call_id: call_id.to_owned(),
            title: Some("Write file".to_owned()),
            action: None,
            description: "Write /tmp/current_time.txt".to_owned(),
            command_type: Some("edit".to_owned()),
            options: vec![nomifun_common::ConfirmationOption {
                label: "Allow".to_owned(),
                value: json!("allow_once"),
                params: None,
            }],
            screenshot: None,
        }
    }

    #[test]
    fn get_confirmations_returns_pending_acp_permission() {
        let (_tx, rx) = mpsc::channel(1);
        let router = PermissionRouter::new(rx);
        let (response_tx, _response_rx) = oneshot::channel();

        router.insert_pending_for_test(
            "tool-1".to_owned(),
            response_tx,
            sample_confirmation("tool-1"),
        );

        let confirmations = router.get_confirmations();
        assert_eq!(confirmations.len(), 1);
        assert_eq!(confirmations[0].id, "tool-1");
        assert_eq!(confirmations[0].call_id, "tool-1");
        assert_eq!(confirmations[0].description, "Write /tmp/current_time.txt");
    }

    #[test]
    fn confirm_removes_pending_confirmation_and_forwards_option() {
        let (_tx, rx) = mpsc::channel(1);
        let router = PermissionRouter::new(rx);
        let (response_tx, mut response_rx) = oneshot::channel();
        router.insert_pending_for_test(
            "tool-1".to_owned(),
            response_tx,
            sample_confirmation("tool-1"),
        );

        router
            .confirm("tool-1", "allow_once".to_owned(), "conv-1")
            .expect("confirm should succeed");

        assert!(router.get_confirmations().is_empty());
        assert!(matches!(
            response_rx.try_recv(),
            Ok(PermissionDecision::Selected { option_id }) if option_id == "allow_once"
        ));
    }

    #[test]
    fn confirm_missing_permission_returns_specific_error() {
        let (_tx, rx) = mpsc::channel(1);
        let router = PermissionRouter::new(rx);

        let error = router
            .confirm("missing-tool", "allow_once".to_owned(), "conv-1")
            .expect_err("missing permission should fail");

        assert!(
            error
                .to_string()
                .contains("Pending ACP permission not found: missing-tool")
        );
    }

    #[test]
    fn cancel_all_removes_pending_confirmations() {
        let (_tx, rx) = mpsc::channel(1);
        let router = PermissionRouter::new(rx);
        let (response_tx, _response_rx) = oneshot::channel();
        router.insert_pending_for_test(
            "tool-1".to_owned(),
            response_tx,
            sample_confirmation("tool-1"),
        );

        router.cancel_all();

        assert!(router.get_confirmations().is_empty());
    }

    #[tokio::test]
    async fn start_routes_permission_request_and_exposes_recoverable_confirmation() {
        let (permission_tx, permission_rx) = mpsc::channel(1);
        let router = Arc::new(PermissionRouter::new(permission_rx));
        let runtime = AgentRuntimeState::new("conv-1", "/tmp/workspace", 8);
        let mut event_rx = runtime.subscribe();
        router.start(runtime);

        let request = RequestPermissionRequest::new(
            "session-1",
            SdkToolCallUpdate::new(
                "tool-1",
                ToolCallUpdateFields::new()
                    .title("Write file")
                    .kind(SdkToolKind::Edit)
                    .raw_input(json!({ "description": "Write /tmp/current_time.txt" })),
            ),
            vec![PermissionOption::new(
                "allow_once",
                "Allow",
                SdkPermissionOptionKind::AllowOnce,
            )],
        );
        let (response_tx, mut response_rx) = oneshot::channel();

        permission_tx
            .send(PermissionRequest {
                request,
                response_tx,
            })
            .await
            .expect("permission request should be accepted");

        let event = tokio::time::timeout(Duration::from_secs(1), event_rx.recv())
            .await
            .expect("permission event should be emitted")
            .expect("permission event channel should stay open");
        assert!(matches!(event, AgentStreamEvent::AcpPermission(_)));

        let confirmations = router.get_confirmations();
        assert_eq!(confirmations.len(), 1);
        assert_eq!(confirmations[0].id, "tool-1");
        assert_eq!(confirmations[0].call_id, "tool-1");
        assert_eq!(confirmations[0].command_type.as_deref(), Some("edit"));

        router
            .confirm("tool-1", "allow_once".to_owned(), "conv-1")
            .expect("confirm should resolve routed request");

        assert!(router.get_confirmations().is_empty());
        assert!(matches!(
            response_rx.try_recv(),
            Ok(PermissionDecision::Selected { option_id }) if option_id == "allow_once"
        ));
    }

    #[tokio::test]
    async fn start_is_idempotent_and_does_not_spawn_a_second_receiver_owner() {
        let (permission_tx, permission_rx) = mpsc::channel(1);
        let router = Arc::new(PermissionRouter::new(permission_rx));
        let runtime = AgentRuntimeState::new("conv-idempotent-start", "/tmp/workspace", 8);

        router.start(runtime.clone());
        router.start(runtime);
        tokio::task::yield_now().await;

        assert_eq!(
            Arc::strong_count(&router),
            2,
            "the caller and exactly one receiver task should own the router"
        );

        drop(permission_tx);
        tokio::time::timeout(Duration::from_secs(1), async {
            while Arc::strong_count(&router) != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the receiver task should release its router owner after channel EOF");
    }

    #[tokio::test]
    async fn watchdog_timeout_cancels_pending_permission_and_removes_confirmation() {
        let (permission_tx, permission_rx) = mpsc::channel(1);
        let router = Arc::new(PermissionRouter::new_with_timeout(
            permission_rx,
            Duration::from_millis(10),
        ));
        let runtime = AgentRuntimeState::new("conv-timeout", "/tmp/workspace", 8);
        let mut event_rx = runtime.subscribe();
        router.start(runtime);

        let request = RequestPermissionRequest::new(
            "session-timeout",
            SdkToolCallUpdate::new(
                "timeout-tool",
                ToolCallUpdateFields::new()
                    .title("Write file")
                    .kind(SdkToolKind::Edit)
                    .raw_input(json!({ "description": "Write /tmp/timeout.txt" })),
            ),
            vec![PermissionOption::new(
                "allow_once",
                "Allow",
                SdkPermissionOptionKind::AllowOnce,
            )],
        );
        let (response_tx, mut response_rx) = oneshot::channel();
        permission_tx
            .send(PermissionRequest {
                request,
                response_tx,
            })
            .await
            .expect("permission request should be accepted");
        let _ = event_rx
            .recv()
            .await
            .expect("permission event should be emitted");
        assert_eq!(router.get_confirmations().len(), 1);

        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), &mut response_rx).await,
            Ok(Ok(PermissionDecision::Cancelled))
        ));
        assert!(router.get_confirmations().is_empty());
    }

    #[test]
    fn stale_watchdog_generation_cannot_cancel_replacement_permission() {
        let (_tx, rx) = mpsc::channel(1);
        let router = PermissionRouter::new(rx);
        let (old_tx, mut old_rx) = oneshot::channel();
        let old_generation = router.next_generation();
        router.insert_pending_for_test_with_generation(
            "reused-tool".to_owned(),
            old_tx,
            sample_confirmation("reused-tool"),
            old_generation,
        );

        let (new_tx, mut new_rx) = oneshot::channel();
        let new_generation = router.next_generation();
        router.insert_pending_for_test_with_generation(
            "reused-tool".to_owned(),
            new_tx,
            sample_confirmation("reused-tool"),
            new_generation,
        );

        router.expire_pending("reused-tool", old_generation);
        assert_eq!(router.get_confirmations().len(), 1);
        assert!(matches!(old_rx.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        assert!(matches!(
            new_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));

        router
            .confirm("reused-tool", "allow_once".to_owned(), "conv-generation")
            .expect("replacement permission should still be confirmable");
        assert!(matches!(
            new_rx.try_recv(),
            Ok(PermissionDecision::Selected { option_id }) if option_id == "allow_once"
        ));
    }

    #[tokio::test]
    async fn set_closing_cancels_existing_and_rejects_new_permission_requests() {
        let (permission_tx, permission_rx) = mpsc::channel(2);
        let router = Arc::new(PermissionRouter::new(permission_rx));
        let runtime = AgentRuntimeState::new("conv-closing", "/tmp/workspace", 8);
        let mut event_rx = runtime.subscribe();
        let (existing_tx, mut existing_rx) = oneshot::channel();
        router.insert_pending_for_test(
            "existing-tool".to_owned(),
            existing_tx,
            sample_confirmation("existing-tool"),
        );

        router.set_closing();
        assert!(router.is_closing());
        assert!(matches!(
            existing_rx.try_recv(),
            Ok(PermissionDecision::Cancelled)
        ));
        assert!(router.get_confirmations().is_empty());

        router.start(runtime);
        let request = RequestPermissionRequest::new(
            "session-closing",
            SdkToolCallUpdate::new(
                "new-tool",
                ToolCallUpdateFields::new()
                    .title("Write file")
                    .kind(SdkToolKind::Edit)
                    .raw_input(json!({ "description": "Write /tmp/new.txt" })),
            ),
            vec![PermissionOption::new(
                "allow_once",
                "Allow",
                SdkPermissionOptionKind::AllowOnce,
            )],
        );
        let (new_tx, new_rx) = oneshot::channel();
        permission_tx
            .send(PermissionRequest {
                request,
                response_tx: new_tx,
            })
            .await
            .expect("closing router still receives and cancels requests");

        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), new_rx).await,
            Ok(Ok(PermissionDecision::Cancelled))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), event_rx.recv())
                .await
                .is_err(),
            "a closing router must not emit a new permission event"
        );
    }

    #[tokio::test]
    async fn permission_channel_eof_cancels_all_pending_requests() {
        let (permission_tx, permission_rx) = mpsc::channel(1);
        let router = Arc::new(PermissionRouter::new(permission_rx));
        let runtime = AgentRuntimeState::new("conv-eof", "/tmp/workspace", 8);
        let mut event_rx = runtime.subscribe();
        router.start(runtime);

        let request = RequestPermissionRequest::new(
            "session-eof",
            SdkToolCallUpdate::new(
                "eof-tool",
                ToolCallUpdateFields::new()
                    .title("Write file")
                    .kind(SdkToolKind::Edit)
                    .raw_input(json!({ "description": "Write /tmp/eof.txt" })),
            ),
            vec![PermissionOption::new(
                "allow_once",
                "Allow",
                SdkPermissionOptionKind::AllowOnce,
            )],
        );
        let (response_tx, response_rx) = oneshot::channel();
        permission_tx
            .send(PermissionRequest {
                request,
                response_tx,
            })
            .await
            .expect("permission request should be accepted");
        let _ = event_rx
            .recv()
            .await
            .expect("permission event should be emitted");
        assert_eq!(router.get_confirmations().len(), 1);

        drop(permission_tx);
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), response_rx).await,
            Ok(Ok(PermissionDecision::Cancelled))
        ));
        assert!(router.get_confirmations().is_empty());
    }

}
