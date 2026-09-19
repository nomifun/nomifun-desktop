//! Connection-only owner for a user's already-running browser.
//!
//! Deliberately separate from process launch: no process handle, Profile
//! lifetime, browser creation, global auto-attach, target cleanup or reconnect.
//! No raw protocol handle is exposed to application/model callers. The
//! application owns Browser Module/Resource authority; this engine supplies
//! only installation-connected tab handles and Agent operations.

use std::{path::Path, sync::Mutex};

use chromiumoxide::cdp::browser_protocol::browser::GetVersionParams;
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use tokio::io::AsyncReadExt;

use crate::transport::{Connection, ROOT_SESSION};

mod automation;
mod tabs;
pub use tabs::{AttachedProviderTab, GrantedTab, GrantedTabInfo};

const MAX_PORT_FILE_BYTES: u64 = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachError {
    #[error("Connecting an existing browser is not yet supported on this platform")]
    UnsupportedPlatform,
    #[error(
        "Chrome is not ready for an authorized connection; enable remote debugging in the running browser"
    )]
    NotReady,
    #[error("The running browser connection metadata is invalid")]
    InvalidEndpoint,
    #[error("The browser connection was denied, lost, or timed out")]
    ConnectionFailed,
    #[error("The connected browser does not meet the Chrome 144+ connection requirement")]
    UnsupportedBrowser,
    #[error("The attached browser target is no longer current")]
    StaleTarget,
    #[error("The browser tab list exceeded its bounded capacity")]
    InventoryLimit,
}

/// This object never owns the browser. Explicit disconnect and Drop only
/// invalidate and dispose our socket. Disconnect is terminal for this object.
pub struct AttachedBrowser {
    state: std::sync::Arc<Mutex<AttachedState>>,
    operations: std::sync::Arc<tokio::sync::Mutex<()>>,
    incarnation: String,
    browser_identity: [u8; 32],
    chromium_major: u32,
}

type Retirement = Shared<BoxFuture<'static, Result<(), AttachError>>>;

struct AttachedState {
    connection: Option<Connection>,
    retirement: Option<Retirement>,
    automation: std::collections::BTreeMap<String, automation::TabAutomation>,
    pending: std::collections::BTreeMap<String, automation::Pending>,
    #[cfg(test)]
    retirement_pause: Option<(
        tokio::sync::oneshot::Sender<()>,
        tokio::sync::oneshot::Receiver<()>,
    )>,
}

impl AttachedBrowser {
    /// Test harness seam for a separately owned, disposable browser. It uses
    /// the production attach path but is absent from normal desktop builds.
    #[cfg(feature = "conformance")]
    pub async fn connect_for_conformance(port_file: &Path) -> Result<Self, AttachError> {
        Self::connect_port_file(port_file).await
    }
    /// Invoke only following an explicit user connection request. This reads
    /// Chrome's small connection-discovery file, not Cookies, Login Data, Local
    /// State, Preferences or history. It does not enable debugging for the user.
    /// Non-default Profile/Edge discovery requires separate platform evidence.
    pub async fn connect_running_chrome() -> Result<Self, AttachError> {
        #[cfg(target_os = "windows")]
        {
            let local_data = std::env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .ok_or(AttachError::NotReady)?;
            let local_data = std::path::PathBuf::from(local_data);
            if !local_data.is_absolute() {
                return Err(AttachError::NotReady);
            }
            Self::connect_port_file(&local_data.join("Google/Chrome/User Data/DevToolsActivePort"))
                .await
        }
        #[cfg(target_os = "macos")]
        {
            let home = dirs::home_dir()
                .filter(|path| path.is_absolute())
                .ok_or(AttachError::NotReady)?;
            Self::connect_port_file(&macos_chrome_port_file(&home)).await
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Err(AttachError::UnsupportedPlatform)
        }
    }

    async fn connect_port_file(path: &Path) -> Result<Self, AttachError> {
        let file = tokio::fs::File::open(path)
            .await
            .map_err(|_| AttachError::NotReady)?;
        let metadata = file.metadata().await.map_err(|_| AttachError::NotReady)?;
        if !metadata.is_file() || metadata.len() > MAX_PORT_FILE_BYTES {
            return Err(AttachError::InvalidEndpoint);
        }
        let mut bytes = Vec::new();
        file.take(MAX_PORT_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| AttachError::NotReady)?;
        let endpoint = parse_endpoint(&bytes)?;
        // Connection::connect neither launches nor attaches targets. Never
        // enable the managed engine's global auto-attach/cleanup machinery.
        let connection = Connection::connect(&endpoint)
            .await
            .map_err(|_| AttachError::ConnectionFailed)?;
        let probe = connection
            .send(ROOT_SESSION, &GetVersionParams::default())
            .await;
        let chromium_major = probe
            .as_ref()
            .ok()
            .and_then(|value| value.get("product"))
            .and_then(|value| value.as_str())
            .and_then(|product| product.strip_prefix("Chrome/"))
            .and_then(|version| version.split('.').next())
            .and_then(|major| major.parse::<u32>().ok())
            .filter(|major| *major >= 144);
        match chromium_major {
            Some(chromium_major) if !connection.registry().is_connection_closed() => Ok(Self {
                state: std::sync::Arc::new(Mutex::new(AttachedState {
                    connection: Some(connection),
                    retirement: None,
                    automation: Default::default(),
                    pending: Default::default(),
                    #[cfg(test)]
                    retirement_pause: None,
                })),
                operations: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                incarnation: nomifun_common::generate_id(),
                browser_identity: {
                    use sha2::Digest;
                    sha2::Sha256::digest(endpoint.as_bytes()).into()
                },
                chromium_major,
            }),
            _ => {
                let failed = probe.is_err() || connection.registry().is_connection_closed();
                connection.shutdown().await;
                Err(if failed {
                    AttachError::ConnectionFailed
                } else {
                    AttachError::UnsupportedBrowser
                })
            }
        }
    }

    pub fn chromium_major(&self) -> u32 {
        self.chromium_major
    }

    pub fn is_connected(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connection
            .as_ref()
            .is_some_and(|conn| !conn.registry().is_connection_closed())
    }

    /// Synchronous admission/I/O fence. The owner still must await disconnect
    /// to prove physical disposal; this can interrupt a pending inventory read.
    pub fn request_disconnect(&self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(connection) = &state.connection {
            connection.registry().fail_connection();
        }
    }

    /// Stop admission synchronously even if the caller later cancels the close
    /// future. This never emits Browser.close or Target.closeTarget.
    pub async fn disconnect(&self) -> Result<(), AttachError> {
        let retirement = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(retirement) = &state.retirement {
                retirement.clone()
            } else {
                let connection = state
                    .connection
                    .take()
                    .expect("connection retires exactly once");
                connection.registry().fail_connection();
                let mut cleanup: Vec<_> = state
                    .pending
                    .values()
                    .map(automation::Pending::retirement)
                    .collect();
                cleanup.extend(
                    state
                        .automation
                        .values()
                        .filter_map(automation::TabAutomation::retire_dialogs),
                );
                state.automation.clear();
                state.pending.clear();
                let operations = self.operations.clone();
                #[cfg(test)]
                let pause = state.retirement_pause.take();
                // One task owns physical disposal. Cancellation only stops a
                // caller's wait; a later/concurrent caller joins the same task.
                let task = tokio::spawn(async move {
                    #[cfg(test)]
                    if let Some((started, resume)) = pause {
                        let _ = started.send(());
                        let _ = resume.await;
                    }
                    // Input/preparation owns the gate; reply-drain and dialog
                    // observers also retain explicit receipts beyond that gate.
                    let _operations = operations.lock().await;
                    connection.shutdown().await;
                    // Dispose the sink even without a WebSocket Close ACK.
                    drop(connection);
                    for receipt in cleanup {
                        receipt.await;
                    }
                });
                let retirement =
                    async move { task.await.map_err(|_| AttachError::ConnectionFailed) }
                        .boxed()
                        .shared();
                state.retirement = Some(retirement.clone());
                retirement
            }
        };
        retirement.await
    }
}

#[cfg(target_os = "macos")]
fn macos_chrome_port_file(home: &Path) -> std::path::PathBuf {
    home.join("Library/Application Support/Google/Chrome/DevToolsActivePort")
}

impl Drop for AttachedBrowser {
    fn drop(&mut self) {
        // Retained modal/input tasks may outlive their caller. Dropping the
        // unique connection owner must fence them rather than form an Arc cycle.
        self.request_disconnect();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pending.clear();
        state.automation.clear();
    }
}

/// Do not accept a URL from the file, model, or network. Construct a loopback
/// URL from strictly validated port + opaque browser path. No redirects, user
/// info, query, fragment, traversal, percent escapes or alternate hosts.
fn parse_endpoint(bytes: &[u8]) -> Result<String, AttachError> {
    if bytes.len() as u64 > MAX_PORT_FILE_BYTES || !bytes.is_ascii() {
        return Err(AttachError::InvalidEndpoint);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| AttachError::InvalidEndpoint)?;
    let mut lines = text.lines();
    let port = lines.next().ok_or(AttachError::InvalidEndpoint)?;
    let path = lines.next().ok_or(AttachError::InvalidEndpoint)?;
    if lines.next().is_some() || port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AttachError::InvalidEndpoint);
    }
    let port = port
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or(AttachError::InvalidEndpoint)?;
    let token = path
        .strip_prefix("/devtools/browser/")
        .ok_or(AttachError::InvalidEndpoint)?;
    if token.is_empty()
        || token.len() > 128
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(AttachError::InvalidEndpoint);
    }
    Ok(format!("ws://127.0.0.1:{port}{path}"))
}

#[cfg(test)]
#[path = "attached_browser_tests.rs"]
mod tests;
