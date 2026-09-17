//! Opaque installation-level attached-Chrome tab handles. Inventory is
//! available only after the application's Browser Module/Resource admission.
//! No page is attached merely to enumerate.

use super::{AttachError, AttachedBrowser, Connection, ROOT_SESSION};
use chromiumoxide::cdp::browser_protocol::target::{GetTargetInfoParams, GetTargetsParams};
use serde_json::Value;

const MAX_TABS: usize = 4096;
const MAX_INVENTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct GrantedTabInfo {
    pub tab_id: String,
    pub title: String,
    pub url: String,
}

/// Host-only tab handle for the installation-level attached Chrome Provider.
/// It is never serialized and does not represent an Agent Action grant.
pub struct AttachedProviderTab {
    pub grant: GrantedTab,
    pub info: GrantedTabInfo,
}

pub(super) struct DiscoveredTab {
    target_id: String,
    title: String,
    url: String,
}

/// Not deserializable: only the connected engine can mint this object.
/// The application host must also bind it to its principal/AgentSession/run. This
/// low-level grant is not a replacement for the Agent invocation boundary.
#[derive(Clone)]
pub struct GrantedTab {
    pub(super) incarnation: String,
    pub(super) target_id: String,
    pub(super) id: String,
    browser_identity: [u8; 32],
}

impl GrantedTab {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn same_target(&self, other: &Self) -> bool {
        self.incarnation == other.incarnation && self.target_id == other.target_id
    }
    /// Host-only contention key, stable across connections to the same browser
    /// incarnation. It discloses neither endpoint nor raw target identity.
    pub fn target_key(&self) -> String {
        use sha2::Digest;
        let mut digest = sha2::Sha256::new();
        digest.update(self.browser_identity);
        digest.update(self.target_id.as_bytes());
        format!("{:x}", digest.finalize())
    }
}

impl AttachedBrowser {
    pub(super) fn current_connection(&self) -> Result<Connection, AttachError> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .connection
            .as_ref()
            .filter(|conn| !conn.registry().is_connection_closed())
            .cloned()
            .ok_or(AttachError::ConnectionFailed)
    }

    /// Enumerate current page tabs for an already connected installation-level
    /// Provider. Connecting the Provider is the user's one explicit consent
    /// step; AgentSessions do not mint a second per-tab authorization grant.
    /// Canonical Browser Module and Resource authority is still checked by the
    /// application host before this method is reachable.
    pub async fn tabs_for_provider(&self) -> Result<Vec<AttachedProviderTab>, AttachError> {
        let _operation = self.operations.lock().await;
        let connection = self.current_connection()?;
        let value = connection
            .send(ROOT_SESSION, &GetTargetsParams::default())
            .await
            .map_err(|_| AttachError::ConnectionFailed)?;
        let targets = value
            .get("targetInfos")
            .and_then(Value::as_array)
            .ok_or(AttachError::ConnectionFailed)?;
        if targets.len() > MAX_TABS {
            return Err(AttachError::InventoryLimit);
        }
        let mut tabs = Vec::new();
        let mut bytes = 0usize;
        let mut targets_seen = std::collections::HashSet::new();
        for target in targets {
            let Some(tab) = discovered_tab(target) else {
                continue;
            };
            if !targets_seen.insert(tab.target_id.clone()) {
                return Err(AttachError::ConnectionFailed);
            }
            bytes += tab.target_id.len() + tab.url.len() + tab.title.len();
            if bytes > MAX_INVENTORY_BYTES {
                return Err(AttachError::InventoryLimit);
            }
            let grant = GrantedTab {
                incarnation: self.incarnation.clone(),
                target_id: tab.target_id,
                id: nomifun_common::generate_id(),
                browser_identity: self.browser_identity,
            };
            tabs.push(AttachedProviderTab {
                info: GrantedTabInfo {
                    tab_id: grant.id.clone(),
                    title: tab.title,
                    url: tab.url,
                },
                grant,
            });
        }
        if !self.is_connected() {
            return Err(AttachError::ConnectionFailed);
        }
        Ok(tabs)
    }

}

pub(super) async fn target_info(
    connection: &Connection,
    target_id: &str,
) -> Result<DiscoveredTab, AttachError> {
    let params = GetTargetInfoParams::builder()
        .target_id(target_id.to_owned())
        .build();
    let result = connection
        .send(ROOT_SESSION, &params)
        .await
        .map_err(|_| AttachError::StaleTarget)?;
    let tab = result
        .get("targetInfo")
        .and_then(discovered_tab)
        .ok_or(AttachError::StaleTarget)?;
    if tab.target_id != target_id {
        return Err(AttachError::StaleTarget);
    }
    Ok(tab)
}

fn discovered_tab(value: &Value) -> Option<DiscoveredTab> {
    if value.get("type")?.as_str()? != "page" {
        return None;
    }
    let target_id = value.get("targetId")?.as_str()?;
    let title = value.get("title")?.as_str()?;
    let url = value.get("url")?.as_str()?;
    if target_id.is_empty()
        || target_id.len() > 256
        || target_id.chars().any(char::is_control)
        || title.len() > 4096
        || url.len() > 8192
    {
        return None;
    }
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") && url != "about:blank" {
        return None;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    Some(DiscoveredTab {
        target_id: target_id.into(),
        title: title.into(),
        url: url.into(),
    })
}

#[cfg(test)]
mod tests;
