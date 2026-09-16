//! Test-only transparent CDP relay. Commands and Chrome replies are never
//! fabricated: selected input acknowledgements are retained to expose the
//! real application's cancellation/cleanup ordering deterministically.
use futures_util::{SinkExt, StreamExt};
use nomi_browser_engine::attached_browser::{AttachError, UserTabInventory};
use nomifun_app::system_browser::{
    SystemBrowserConnection, SystemBrowserConnectionFactory, SystemBrowserTab,
};
use nomifun_browser_platform::system_browser::{SystemBrowserCommand, SystemBrowserRuntimeError};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::{net::TcpListener, sync::Semaphore, task::JoinHandle};
use tokio_tungstenite::{accept_async, connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

pub struct InputProxy {
    pub port_file: PathBuf,
    pub pressed: Semaphore,
    pub released: Semaphore,
    pub allow_pressed: Semaphore,
    pub allow_released: Semaphore,
    armed: AtomicBool,
    events: Mutex<Vec<String>>,
    active_cancel: Mutex<Option<CancellationToken>>,
    stop: CancellationToken,
}

impl InputProxy {
    pub async fn start(
        root: &Path,
        owned_port_file: &Path,
    ) -> (Arc<Self>, JoinHandle<Result<(), String>>) {
        // The caller supplies only its owned fixture Profile, never discovery
        // of a user's default browser. The relay is not a shipping endpoint.
        let metadata = std::fs::read_to_string(owned_port_file).unwrap();
        let mut lines = metadata.lines();
        let port: u16 = lines.next().unwrap().parse().unwrap();
        let path = lines.next().unwrap();
        assert!(path.starts_with("/devtools/browser/"));
        let upstream = format!("ws://127.0.0.1:{port}{path}");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port_file = root.join("fixture-proxy-port");
        std::fs::write(
            &port_file,
            format!("{}\n{path}\n", listener.local_addr().unwrap().port()),
        )
        .unwrap();
        let proxy = Arc::new(Self {
            port_file,
            pressed: Semaphore::new(0),
            released: Semaphore::new(0),
            allow_pressed: Semaphore::new(0),
            allow_released: Semaphore::new(0),
            armed: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
            active_cancel: Mutex::new(None),
            stop: CancellationToken::new(),
        });
        let owner = proxy.clone();
        let task = tokio::spawn(async move {
            let work = async {
                let (socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
                let mut client = accept_async(socket).await.map_err(|e| e.to_string())?;
                let (mut chrome, _) = connect_async(upstream).await.map_err(|e| e.to_string())?;
                let mut down_id = None;
                let mut up_id = None;
                let mut expect_up = false;
                let mut down_reply = None;
                let mut up_reply = None;
                loop {
                    tokio::select! {
                        message = client.next() => {
                            let Some(message) = message else { break; };
                            let message = message.map_err(|e|e.to_string())?;
                            if message.is_close() { break; }
                            if let Message::Text(text) = &message {
                                let value: Value = serde_json::from_str(text).map_err(|e|e.to_string())?;
                                if value["method"] == "Input.dispatchMouseEvent" {
                                    match value["params"]["type"].as_str() {
                                        Some("mousePressed") if owner.armed.swap(false, Ordering::SeqCst) => {
                                            down_id = value["id"].as_u64();
                                            expect_up = true;
                                            owner.events.lock().unwrap().push("down".into());
                                        }
                                        Some("mouseReleased") if expect_up => {
                                            up_id = value["id"].as_u64();
                                            expect_up = false;
                                            owner.events.lock().unwrap().push("up".into());
                                        }
                                        Some("mousePressed" | "mouseReleased") if !owner.events.lock().unwrap().is_empty() => {
                                            owner.events.lock().unwrap().push("extra_input".into());
                                        }
                                        _ => {}
                                    }
                                }
                                if value["method"] == "Target.detachFromTarget" && !owner.events.lock().unwrap().is_empty() {
                                    owner.events.lock().unwrap().push("detach".into());
                                }
                            }
                            chrome.send(message).await.map_err(|e|e.to_string())?;
                        }
                        message = chrome.next() => {
                            let Some(message) = message else { break; };
                            let message = message.map_err(|e|e.to_string())?;
                            if message.is_close() { break; }
                            if let Message::Text(text) = &message {
                                let value: Value = serde_json::from_str(text).map_err(|e|e.to_string())?;
                                let id = value["id"].as_u64();
                                if id.is_some() && (id == down_id || id == up_id) {
                                    if value.get("error").is_some() { return Err("Chrome rejected gated input".into()); }
                                    if id == down_id {
                                        down_id = None; down_reply = Some(message);
                                        owner.pressed.add_permits(1);
                                    } else {
                                        up_id = None; up_reply = Some(message);
                                        owner.released.add_permits(1);
                                    }
                                    continue;
                                }
                            }
                            client.send(message).await.map_err(|e|e.to_string())?;
                        }
                        permit = owner.allow_pressed.acquire(), if down_reply.is_some() => {
                            permit.map_err(|e|e.to_string())?.forget();
                            owner.events.lock().unwrap().push("down_ack".into());
                            client.send(down_reply.take().unwrap()).await.map_err(|e|e.to_string())?;
                        }
                        permit = owner.allow_released.acquire(), if up_reply.is_some() => {
                            permit.map_err(|e|e.to_string())?.forget();
                            owner.events.lock().unwrap().push("up_ack".into());
                            client.send(up_reply.take().unwrap()).await.map_err(|e|e.to_string())?;
                        }
                    }
                }
                Ok(())
            };
            tokio::select! { result = work => result, _ = owner.stop.cancelled() => Ok(()) }
        });
        (proxy, task)
    }

    pub fn arm(&self) {
        self.events.lock().unwrap().clear();
        self.active_cancel.lock().unwrap().take();
        assert!(!self.armed.swap(true, Ordering::SeqCst));
    }

    pub fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }

    pub fn active_cancel(&self) -> CancellationToken {
        self.active_cancel
            .lock()
            .unwrap()
            .clone()
            .expect("actual input invocation token")
    }

    pub fn observing_factory(
        self: &Arc<Self>,
        inner: Arc<dyn SystemBrowserConnectionFactory>,
    ) -> Arc<dyn SystemBrowserConnectionFactory> {
        Arc::new(ObservingFactory {
            inner,
            proxy: self.clone(),
        })
    }
}

// Transparent test observer: every operation still delegates to the production
// connection. Retaining its invocation token lets the test prove Stop reached
// the actual in-flight action before releasing any protocol acknowledgement.
struct ObservingFactory {
    inner: Arc<dyn SystemBrowserConnectionFactory>,
    proxy: Arc<InputProxy>,
}
struct ObservingConnection {
    inner: Arc<dyn SystemBrowserConnection>,
    proxy: Arc<InputProxy>,
}
#[async_trait::async_trait]
impl SystemBrowserConnectionFactory for ObservingFactory {
    async fn connect(&self) -> Result<Arc<dyn SystemBrowserConnection>, AttachError> {
        Ok(Arc::new(ObservingConnection {
            inner: self.inner.connect().await?,
            proxy: self.proxy.clone(),
        }))
    }
}
#[async_trait::async_trait]
impl SystemBrowserConnection for ObservingConnection {
    fn is_connected(&self) -> bool {
        self.inner.is_connected()
    }
    fn request_disconnect(&self) {
        self.inner.request_disconnect();
    }
    async fn choices(&self) -> Result<UserTabInventory, AttachError> {
        self.inner.choices().await
    }
    async fn grant(&self, choice: &str) -> Result<SystemBrowserTab, AttachError> {
        self.inner.grant(choice).await
    }
    async fn disconnect(&self) -> Result<(), AttachError> {
        self.inner.disconnect().await
    }
    fn target_key(&self, tab: &str) -> Result<String, SystemBrowserRuntimeError> {
        self.inner.target_key(tab)
    }
    async fn invoke(
        &self,
        tab: &str,
        command: SystemBrowserCommand,
        cancel: &CancellationToken,
    ) -> Result<Value, SystemBrowserRuntimeError> {
        if matches!(command, SystemBrowserCommand::Click { .. }) {
            *self.proxy.active_cancel.lock().unwrap() = Some(cancel.clone());
        }
        self.inner.invoke(tab, command, cancel).await
    }
    async fn settle(&self, tab: &str) -> Result<(), SystemBrowserRuntimeError> {
        self.inner.settle(tab).await
    }
}
