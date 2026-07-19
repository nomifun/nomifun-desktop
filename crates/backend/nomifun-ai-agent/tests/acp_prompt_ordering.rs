//! Locks the ACP SDK ordering contract that the manager relies on when it
//! emits a terminal event after `session/prompt` resolves.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use agent_client_protocol::schema::{
    ContentBlock, ContentChunk, PromptRequest, PromptResponse, SessionId, SessionNotification,
    SessionUpdate, StopReason,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Responder};
use tokio::sync::{oneshot, Notify};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

#[tokio::test]
async fn prompt_response_cannot_overtake_a_prior_session_update() {
    // Exercise the same newline-framed byte transport used for real CLI
    // stdin/stdout, while remaining deterministic and platform-independent.
    let (client_stream, agent_stream) = tokio::io::duplex(8 * 1024);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let (agent_read, agent_write) = tokio::io::split(agent_stream);
    let client_transport = ByteStreams::new(client_write.compat_write(), client_read.compat());
    let agent_transport = ByteStreams::new(agent_write.compat_write(), agent_read.compat());
    let notification_entered = Arc::new(Notify::new());
    let release_notification = Arc::new(Notify::new());
    let notification_completed = Arc::new(AtomicBool::new(false));
    let (shutdown_agent_tx, shutdown_agent_rx) = oneshot::channel();

    let agent = Agent.builder().on_receive_request(
        async move |_request: PromptRequest,
                    responder: Responder<PromptResponse>,
                    connection: ConnectionTo<Client>| {
            // Both messages use the SDK connection's single FIFO outgoing
            // queue, matching a conforming agent's final update + EndTurn.
            connection.send_notification(SessionNotification::new(
                SessionId::new("ordering-session"),
                SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::from(
                    "final chunk",
                ))),
            ))?;
            responder.respond(PromptResponse::new(StopReason::EndTurn))
        },
        agent_client_protocol::on_receive_request!(),
    );

    let client = Client.builder().on_receive_notification(
        {
            let notification_entered = Arc::clone(&notification_entered);
            let release_notification = Arc::clone(&release_notification);
            let notification_completed = Arc::clone(&notification_completed);
            async move |_notification: SessionNotification, _connection: ConnectionTo<Agent>| {
                notification_entered.notify_one();
                release_notification.notified().await;
                notification_completed.store(true, Ordering::Release);
                Ok(())
            }
        },
        agent_client_protocol::on_receive_notification!(),
    );

    let agent_task = agent.connect_with(agent_transport, async move |_connection| {
        let _ = shutdown_agent_rx.await;
        Ok(())
    });

    let client_task = client.connect_with(client_transport, async move |connection| {
        let prompt = connection
            .send_request(PromptRequest::new(
                SessionId::new("ordering-session"),
                vec![ContentBlock::from("hello")],
            ))
            .block_task();
        tokio::pin!(prompt);

        // The agent has already queued the final update and its response by
        // the time this handler is entered. While the handler is blocked, the
        // SDK dispatch loop must not deliver the following PromptResponse.
        notification_entered.notified().await;
        tokio::select! {
            biased;
            response = &mut prompt => {
                panic!("PromptResponse overtook its prior session/update: {response:?}");
            }
            _ = tokio::task::yield_now() => {}
        }

        release_notification.notify_one();
        let response = prompt.await?;
        assert_eq!(response.stop_reason, StopReason::EndTurn);
        assert!(
            notification_completed.load(Ordering::Acquire),
            "terminal response resolved before the final update handler completed"
        );

        let _ = shutdown_agent_tx.send(());
        Ok(())
    });

    let (agent_result, client_result) = tokio::join!(agent_task, client_task);
    agent_result.expect("agent endpoint failed");
    client_result.expect("client endpoint failed");
}
