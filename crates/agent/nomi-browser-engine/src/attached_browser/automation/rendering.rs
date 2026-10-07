//! A granted page stays paintable from its first Agent action until run settle.
//! BringToFront alone does not undo Windows occlusion. Releasing this override
//! after each wheel ACK is too early: Chromium may not have applied its scroll.
//! No OS focus, launch flags, user Profile or browser preferences are changed.
use super::{Connection, Error};
use chromiumoxide::cdp::browser_protocol::emulation::SetFocusEmulationEnabledParams;

pub(super) struct Rendering {
    connection: Connection,
    session: String,
    armed: bool,
}

impl Rendering {
    pub(super) async fn begin(
        connection: &Connection,
        session: &str,
        cleanup: Connection,
    ) -> Result<Self, Error> {
        let owner = Self {
            // Cleanup must retain an ordinary response deadline, even after
            // the dialog observer has stopped with its last pause bit set.
            connection: cleanup,
            session: session.into(),
            armed: true,
        };
        connection
            .send(session, &SetFocusEmulationEnabledParams::new(true))
            .await
            .map_err(|_| Error::ExecutionFailed)?;
        Ok(owner)
    }

    pub(super) async fn finish(mut self) -> Result<(), Error> {
        self.connection
            .send(&self.session, &SetFocusEmulationEnabledParams::new(false))
            .await
            .map_err(|_| Error::ExecutionFailed)?;
        self.armed = false;
        Ok(())
    }
}

impl Drop for Rendering {
    fn drop(&mut self) {
        if self.armed {
            // A lost enable/reset ACK or abandoned owner cannot leave our
            // debugger override live. Retire our connection, never the page.
            self.connection.registry().fail_connection();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tokio_tungstenite::{accept_async, tungstenite::Message};

    async fn fixture(
        fail_reset: bool,
        hold_enable: bool,
    ) -> (
        Connection,
        Arc<Mutex<Vec<bool>>>,
        tokio::task::JoinHandle<()>,
    ) {
        let events = Arc::new(Mutex::new(vec![]));
        let recorded = events.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(socket).await.unwrap();
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                assert_eq!(request["method"], "Emulation.setFocusEmulationEnabled");
                assert_eq!(request["sessionId"], "render-fixture");
                let enabled = request["params"]["enabled"].as_bool().unwrap();
                recorded.lock().unwrap().push(enabled);
                if enabled && hold_enable {
                    continue;
                }
                let mut reply = json!({"id":request["id"],"sessionId":request["sessionId"]});
                if !enabled && fail_reset {
                    reply["error"] = json!({"code":-32000,"message":"fixture reset failure"});
                } else {
                    reply["result"] = json!({});
                }
                if socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let connection = Connection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        connection
            .registry()
            .register_session("render-fixture", "page");
        (connection, events, task)
    }

    #[tokio::test]
    async fn reset_releases_our_override_and_keeps_the_connection() {
        let (connection, events, task) = fixture(false, false).await;
        let (pause, receiver) = tokio::sync::watch::channel(false);
        let paused = connection.with_response_pause(receiver);
        let owner = Rendering::begin(&paused, "render-fixture", connection.clone())
            .await
            .unwrap();
        pause.send_replace(true);
        tokio::time::timeout(std::time::Duration::from_secs(2), owner.finish())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(*events.lock().unwrap(), [true, false]);
        assert!(!connection.registry().is_connection_closed());
        connection.shutdown().await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn failed_reset_retires_only_our_protocol_connection() {
        let (connection, events, task) = fixture(true, false).await;
        let owner = Rendering::begin(&connection, "render-fixture", connection.clone())
            .await
            .unwrap();
        assert_eq!(owner.finish().await, Err(Error::ExecutionFailed));
        assert!(connection.registry().is_connection_closed());
        assert_eq!(*events.lock().unwrap(), [true, false]);
        connection.shutdown().await;
        task.await.unwrap();
    }

    #[tokio::test]
    async fn abandoned_enable_ack_does_not_leave_rendering_owned() {
        let (connection, events, task) = fixture(false, true).await;
        let active = connection.clone();
        let enabling = tokio::spawn(async move {
            Rendering::begin(&active, "render-fixture", active.clone()).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while events.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        enabling.abort();
        assert!(matches!(enabling.await, Err(error) if error.is_cancelled()));
        assert!(connection.registry().is_connection_closed());
        assert_eq!(*events.lock().unwrap(), [true]);
        connection.shutdown().await;
        task.await.unwrap();
    }
}
