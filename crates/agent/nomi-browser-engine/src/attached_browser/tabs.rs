//! User-only inventory and opaque tab grants. Never append this inventory to
//! Agent output. No page/session is attached merely to populate the chooser.

use super::{AttachError, AttachedBrowser, Connection, ROOT_SESSION};
use chromiumoxide::cdp::browser_protocol::target::{GetTargetInfoParams, GetTargetsParams};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;

const MAX_TABS: usize = 4096;
const MAX_INVENTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Serialize)]
pub struct UserTabChoice {
    pub choice_id: String,
    pub title: String,
    pub url: String,
}

#[derive(Serialize)]
pub struct UserTabInventory {
    pub tabs: Vec<UserTabChoice>,
}

#[derive(Serialize)]
pub struct GrantedTabInfo {
    pub tab_id: String,
    pub title: String,
    pub url: String,
}

pub(super) struct OfferedTab {
    target_id: String,
    title: String,
    url: String,
}

/// Not deserializable: only a successful user selection can mint this object.
/// The application host must also bind it to its user/conversation/run. This
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

    /// Only the local authenticated user chooser may receive this output. Each
    /// refresh invalidates all prior selection tokens, including on failure.
    pub async fn tabs_for_user(&self) -> Result<UserTabInventory, AttachError> {
        let _operation = self.operations.lock().await;
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .offered_tabs
            .clear();
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
        let mut offered = HashMap::new();
        let mut tabs = Vec::new();
        let mut bytes = 0usize;
        let mut targets_seen = std::collections::HashSet::new();
        for target in targets {
            let Some(tab) = offered_tab(target) else {
                continue;
            };
            if !targets_seen.insert(tab.target_id.clone()) {
                return Err(AttachError::ConnectionFailed);
            }
            bytes += tab.target_id.len() + tab.url.len() + tab.title.len();
            if bytes > MAX_INVENTORY_BYTES {
                return Err(AttachError::InventoryLimit);
            }
            let choice_id = nomifun_common::generate_id();
            tabs.push(UserTabChoice {
                choice_id: choice_id.clone(),
                title: tab.title.clone(),
                url: tab.url.clone(),
            });
            offered.insert(choice_id, tab);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.connection.is_none() || connection.registry().is_connection_closed() {
            return Err(AttachError::ConnectionFailed);
        }
        state.offered_tabs = offered;
        Ok(UserTabInventory { tabs })
    }

    /// Revalidate the exact chosen target before granting it. Never interpret
    /// a raw CDP target ID, default selected page, URL or ordinal as a choice.
    pub async fn grant_tab(&self, choice_id: &str) -> Result<GrantedTab, AttachError> {
        let _operation = self.operations.lock().await;
        let connection = self.current_connection()?;
        let (target_id, shown_url) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let offered = state
                .offered_tabs
                .get(choice_id)
                .ok_or(AttachError::StaleSelection)?;
            (offered.target_id.clone(), offered.url.clone())
        };
        let current = target_info(&connection, &target_id).await?;
        if current.url != shown_url || !self.is_connected() {
            return Err(AttachError::StaleSelection);
        }
        Ok(GrantedTab {
            incarnation: self.incarnation.clone(),
            target_id,
            id: nomifun_common::generate_id(),
            browser_identity: self.browser_identity,
        })
    }

    /// Scoped metadata seam for the owning host. Later observation/actions use
    /// the same opaque grant; the full user chooser is never their fallback.
    pub async fn granted_tab_metadata(
        &self,
        grant: &GrantedTab,
    ) -> Result<GrantedTabInfo, AttachError> {
        let _operation = self.operations.lock().await;
        if grant.incarnation != self.incarnation {
            return Err(AttachError::TabNotAuthorized);
        }
        let connection = self.current_connection()?;
        let tab = target_info(&connection, &grant.target_id).await?;
        if !self.is_connected() {
            return Err(AttachError::ConnectionFailed);
        }
        Ok(GrantedTabInfo {
            tab_id: grant.id.clone(),
            title: tab.title,
            url: tab.url,
        })
    }
}

pub(super) async fn target_info(
    connection: &Connection,
    target_id: &str,
) -> Result<OfferedTab, AttachError> {
    let params = GetTargetInfoParams::builder()
        .target_id(target_id.to_owned())
        .build();
    let result = connection
        .send(ROOT_SESSION, &params)
        .await
        .map_err(|_| AttachError::StaleSelection)?;
    let tab = result
        .get("targetInfo")
        .and_then(offered_tab)
        .ok_or(AttachError::StaleSelection)?;
    if tab.target_id != target_id {
        return Err(AttachError::StaleSelection);
    }
    Ok(tab)
}

fn offered_tab(value: &Value) -> Option<OfferedTab> {
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
    Some(OfferedTab {
        target_id: target_id.into(),
        title: title.into(),
        url: url.into(),
    })
}

#[cfg(test)]
mod tests;
