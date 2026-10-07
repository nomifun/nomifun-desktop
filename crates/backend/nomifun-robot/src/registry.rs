//! Robot registry: `{data_dir}/robot/robots.json`, atomic temp+rename writes.
//!
//! Tokens are persisted as SHA-256 only. A fresh token is minted on **every**
//! OTA report because the firmware re-reads `websocket.token` from each response
//! and persists it to NVS — so rotation-per-boot is transparent, and an already
//! authenticated WebSocket keeps working until it drops.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;

/// Subdirectory of the backend data dir holding robot state.
pub const ROBOT_REL_DIR: &str = "robot";
/// Registry file name inside [`ROBOT_REL_DIR`].
pub const ROBOTS_FILE: &str = "robots.json";

/// Device authorization, independent of the Companion's Agent identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RobotPermissions {
    pub vision: bool,
    pub motion: bool,
    pub display: bool,
    pub device_tools: bool,
    pub proactive_speech: bool,
    pub continuous_vision: bool,
}

impl Default for RobotPermissions {
    fn default() -> Self {
        Self { vision: false, motion: false, display: true, device_tools: false,
            proactive_speech: false, continuous_vision: false }
    }
}

impl RobotPermissions {
    pub fn allows_tool(&self, device_name: &str) -> bool {
        self.allows_action(crate::tool_registry::tool_action(device_name))
            && (!crate::tool_registry::requires_continuous_vision(device_name) || self.continuous_vision)
    }

    pub fn allows_action(&self, action: crate::capability::RobotAction) -> bool {
        match action {
            crate::capability::RobotAction::Vision => self.vision,
            crate::capability::RobotAction::Motion => self.motion,
            crate::capability::RobotAction::Display => self.display,
            crate::capability::RobotAction::Device => self.device_tools,
        }
    }
}

/// One registered robot. `token_hash` is the SHA-256 of the last minted token;
/// the plaintext exists only in the OTA response that minted it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotRecord {
    pub robot_id: String,
    pub client_id: String,
    pub name: String,
    pub companion_id: Option<String>,
    pub token_hash: String,
    pub activation_code: Option<String>,
    pub board: String,
    pub firmware_version: String,
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub permissions: RobotPermissions,
    pub authorization_revision: u64,
}

/// The subset of a firmware device report the registry cares about.
#[derive(Debug, Clone)]
pub struct RobotReport {
    pub robot_id: String,
    pub client_id: String,
    pub board: String,
    pub firmware_version: String,
}

/// Why a claim / patch could not be applied.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    #[error("no robot matches that activation code")]
    NotFound,
    #[error("robot is already bound to companion {companion_id}")]
    AlreadyBound { companion_id: String },
    #[error("robot registry persistence failed: {0}")]
    Persistence(String),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    robots: Vec<RobotRecord>,
}

/// Owns the on-disk registry and the in-memory token index.
pub struct RobotRegistry {
    path: PathBuf,
    inner: RwLock<BTreeMap<String, RobotRecord>>,
    connections: RwLock<BTreeMap<String, String>>,
    playback: RwLock<BTreeMap<String, (String, tokio::sync::mpsc::Sender<RobotPlaybackCommand>)>>,
    operation_gates: Mutex<BTreeMap<String, Arc<RwLock<()>>>>,
}

/// Read lease proving that pairing, permission and connection mutations for
/// one robot cannot race a physical Agent action. The guard has no public
/// contents; retaining the value is the authority boundary.
pub struct RobotActionLease {
    _guard: tokio::sync::OwnedRwLockReadGuard<()>,
}

pub struct RobotPlaybackCommand {
    pub text: String,
    pub accepted: tokio::sync::oneshot::Sender<Result<(), String>>,
}

/// SHA-256 of `token`, lowercase hex (64 chars). Mirrors
/// `nomifun_auth::token_sha256_hex` — duplicated to keep this crate's
/// dependency surface minimal.
fn token_sha256_hex(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn ct_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn mint_activation_code() -> String {
    let mut bytes = [0u8; 4];
    rand::rng().fill_bytes(&mut bytes);
    let n = u32::from_be_bytes(bytes) % 1_000_000;
    format!("{n:06}")
}

fn default_name(board: &str) -> String {
    match board {
        "esp32-s3n16r8-emoji" => "表情机器人".to_owned(),
        other => other.to_owned(),
    }
}

impl RobotRegistry {
    /// Load (or create) the registry under `data_dir/robot/robots.json`.
    pub async fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join(ROBOT_REL_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(ROBOTS_FILE);
        let robots = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<RegistryFile>(&bytes)?.robots,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        let map = robots
            .into_iter()
            .map(|r| (r.robot_id.clone(), r))
            .collect::<BTreeMap<_, _>>();
        Ok(Self {
            path,
            inner: RwLock::new(map),
            connections: RwLock::new(BTreeMap::new()),
            playback: RwLock::new(BTreeMap::new()),
            operation_gates: Mutex::new(BTreeMap::new()),
        })
    }

    fn operation_gate(&self, robot_id: &str) -> Arc<RwLock<()>> {
        let mut gates = self
            .operation_gates
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(
            gates
                .entry(robot_id.to_owned())
                .or_insert_with(|| Arc::new(RwLock::new(()))),
        )
    }

    async fn persist(&self, map: &BTreeMap<String, RobotRecord>) -> anyhow::Result<()> {
        let file = RegistryFile {
            robots: map.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        let tmp = self.path.with_extension("json.tmp");
        let mut output = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .await?;
        output.write_all(&bytes).await?;
        output.sync_all().await?;
        drop(output);
        tokio::fs::rename(&tmp, &self.path).await?;
        Ok(())
    }

    /// Upsert on device report. Always mints a fresh token and returns its
    /// plaintext for the OTA response.
    pub async fn upsert_on_report(
        &self,
        report: RobotReport,
        now_ms: i64,
    ) -> anyhow::Result<(RobotRecord, String)> {
        let _authority = self.operation_gate(&report.robot_id).write_owned().await;
        let token = mint_token();
        let token_hash = token_sha256_hex(&token);
        let mut map = self.inner.write().await;
        let mut next = map.clone();
        let record = match next.get_mut(&report.robot_id) {
            Some(existing) => {
                existing.client_id = report.client_id;
                existing.board = report.board;
                existing.firmware_version = report.firmware_version;
                existing.token_hash = token_hash;
                existing.last_seen = Some(now_ms);
                if existing.companion_id.is_none() && existing.activation_code.is_none() {
                    existing.activation_code = Some(mint_activation_code());
                }
                existing.clone()
            }
            None => {
                let record = RobotRecord {
                    permissions: RobotPermissions::default(),
                    authorization_revision: 0,
                    name: default_name(&report.board),
                    robot_id: report.robot_id.clone(),
                    client_id: report.client_id,
                    companion_id: None,
                    token_hash,
                    activation_code: Some(mint_activation_code()),
                    board: report.board,
                    firmware_version: report.firmware_version,
                    last_seen: Some(now_ms),
                    created_at: now_ms,
                };
                next.insert(record.robot_id.clone(), record.clone());
                record
            }
        };
        self.persist(&next).await?;
        *map = next;
        Ok((record, token))
    }

    /// Resolve a presented bearer token to its robot. Constant-time per entry.
    pub async fn resolve_token(&self, token: &str) -> Option<RobotRecord> {
        if token.is_empty() {
            return None;
        }
        let presented = token_sha256_hex(token);
        let map = self.inner.read().await;
        map.values()
            .find(|r| ct_eq(&presented, &r.token_hash))
            .cloned()
    }

    /// Bind the robot holding `code` to `companion_id`, clearing the code.
    pub async fn claim(&self, code: &str, companion_id: &str) -> Result<RobotRecord, ClaimError> {
        let robot_id = self
            .inner
            .read()
            .await
            .values()
            .find(|record| record.activation_code.as_deref() == Some(code))
            .map(|record| record.robot_id.clone())
            .ok_or(ClaimError::NotFound)?;
        let _authority = self.operation_gate(&robot_id).write_owned().await;
        let mut map = self.inner.write().await;
        let mut next = map.clone();
        let record = next
            .values_mut()
            .find(|r| r.activation_code.as_deref() == Some(code))
            .ok_or(ClaimError::NotFound)?;
        if let Some(bound) = &record.companion_id {
            return Err(ClaimError::AlreadyBound {
                companion_id: bound.clone(),
            });
        }
        record.companion_id = Some(companion_id.to_owned());
        record.activation_code = None;
        let out = record.clone();
        self.persist(&next)
            .await
            .map_err(|error| ClaimError::Persistence(error.to_string()))?;
        *map = next;
        Ok(out)
    }

    /// Rename and/or rebind. `companion_id = Some(None)` unbinds (and re-issues
    /// an activation code so the robot can be claimed again).
    pub async fn patch(
        &self,
        robot_id: &str,
        name: Option<String>,
        companion_id: Option<Option<String>>,
    ) -> Result<RobotRecord, ClaimError> {
        let _authority = self.operation_gate(robot_id).write_owned().await;
        let mut map = self.inner.write().await;
        let mut next = map.clone();
        let record = next.get_mut(robot_id).ok_or(ClaimError::NotFound)?;
        let binding_changed = companion_id.is_some();
        if let Some(name) = name {
            record.name = name;
        }
        if let Some(binding) = companion_id {
            record.authorization_revision += 1;
            match binding {
                Some(id) => {
                    record.companion_id = Some(id);
                    record.activation_code = None;
                }
                None => {
                    record.companion_id = None;
                    record.activation_code = Some(mint_activation_code());
                }
            }
        }
        let out = record.clone();
        self.persist(&next)
            .await
            .map_err(|error| ClaimError::Persistence(error.to_string()))?;
        *map = next;
        if binding_changed {
            self.connections.write().await.remove(robot_id);
            self.playback.write().await.remove(robot_id);
        }
        Ok(out)
    }

    /// Remove a robot (revokes its token). Returns whether it existed.
    pub async fn remove(&self, robot_id: &str) -> anyhow::Result<bool> {
        let _authority = self.operation_gate(robot_id).write_owned().await;
        let mut map = self.inner.write().await;
        let mut next = map.clone();
        let existed = next.remove(robot_id).is_some();
        if existed {
            self.persist(&next).await?;
            *map = next;
            self.connections.write().await.remove(robot_id);
            self.playback.write().await.remove(robot_id);
        }
        Ok(existed)
    }

    /// All records, ordered by `robot_id`.
    pub async fn list(&self) -> Vec<RobotRecord> {
        self.inner.read().await.values().cloned().collect()
    }

    /// Resolve one installation-scoped robot resource by its canonical ID.
    pub async fn get(&self, robot_id: &str) -> Option<RobotRecord> {
        self.inner.read().await.get(robot_id).cloned()
    }

    pub async fn set_permissions(&self, robot_id: &str, permissions: RobotPermissions) -> anyhow::Result<RobotRecord> {
        let _authority = self.operation_gate(robot_id).write_owned().await;
        let mut map = self.inner.write().await;
        let mut next = map.clone();
        let record = next.get_mut(robot_id).ok_or_else(|| anyhow::anyhow!("robot not found"))?;
        record.permissions = permissions;
        record.authorization_revision += 1;
        let result = record.clone();
        self.persist(&next).await?;
        *map = next;
        Ok(result)
    }

    pub async fn connect(&self, robot_id: &str, companion_id: &str, connection_id: &str) -> anyhow::Result<()> {
        let _authority = self.operation_gate(robot_id).write_owned().await;
        let map = self.inner.read().await;
        let record = map.get(robot_id).ok_or_else(|| anyhow::anyhow!("robot not found"))?;
        if record.companion_id.as_deref() != Some(companion_id) {
            anyhow::bail!("robot binding changed");
        }
        self.connections.write().await.insert(robot_id.to_owned(), connection_id.to_owned());
        Ok(())
    }

    pub async fn connection_matches(&self, robot_id: &str, connection_id: &str) -> bool {
        self.connections.read().await.get(robot_id).is_some_and(|current| current == connection_id)
    }

    pub async fn current_connection(&self, robot_id: &str) -> Option<String> {
        self.connections.read().await.get(robot_id).cloned()
    }

    /// Hold the exact live connection stable across permission/pairing recheck,
    /// device dispatch and durable receipt settlement.
    pub async fn hold_action_connection(
        &self,
        robot_id: &str,
        expected_connection_id: &str,
    ) -> Option<RobotActionLease> {
        let guard = self.operation_gate(robot_id).read_owned().await;
        if self
            .connections
            .read()
            .await
            .get(robot_id)
            .is_some_and(|current| current == expected_connection_id)
        {
            Some(RobotActionLease { _guard: guard })
        } else {
            None
        }
    }

    pub(crate) async fn hold_connection(&self, robot_id: &str, expected: Option<&str>)
        -> Option<tokio::sync::RwLockReadGuard<'_, BTreeMap<String, String>>>
    {
        let guard = self.connections.read().await;
        if guard.get(robot_id).map(String::as_str) == expected { Some(guard) } else { None }
    }

    pub async fn open_playback(&self, robot_id: &str, connection_id: &str)
        -> anyhow::Result<tokio::sync::mpsc::Receiver<RobotPlaybackCommand>>
    {
        let connections = self.connections.read().await;
        if connections.get(robot_id).map(String::as_str) != Some(connection_id) { anyhow::bail!("device connection changed"); }
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        self.playback.write().await.insert(robot_id.to_owned(), (connection_id.to_owned(), tx));
        Ok(rx)
    }

    pub async fn speak(&self, robot_id: &str, connection_id: &str, text: String) -> Result<(), String> {
        if text.trim().is_empty() || text.chars().count() > 12000 { return Err("reply is empty or too long".to_owned()); }
        let sender = self.playback.read().await.get(robot_id)
            .filter(|(current, _)| current == connection_id).map(|(_, sender)| sender.clone())
            .ok_or("device is offline")?;
        let (accepted, rx) = tokio::sync::oneshot::channel();
        sender.try_send(RobotPlaybackCommand { text, accepted }).map_err(|_| "device is busy".to_owned())?;
        tokio::time::timeout(std::time::Duration::from_secs(5), rx).await
            .map_err(|_| "device did not accept playback".to_owned())?
            .map_err(|_| "device connection closed".to_owned())?
    }

    /// A late disconnect from an old socket cannot revoke its replacement.
    pub async fn disconnect(&self, robot_id: &str, connection_id: &str) -> bool {
        let _authority = self.operation_gate(robot_id).write_owned().await;
        let mut map = self.connections.write().await;
        if map.get(robot_id).is_some_and(|current| current == connection_id) {
            map.remove(robot_id);
            self.playback.write().await.remove(robot_id);
            true
        } else { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_vision_requires_both_camera_and_continuous_permission() {
        let mut permissions = RobotPermissions::default();
        assert!(!permissions.allows_tool("self.camera.take_photo"));
        permissions.vision = true;
        assert!(permissions.allows_tool("self.camera.take_photo"));
        assert!(!permissions.allows_tool("self.camera.start_stream"));
        permissions.continuous_vision = true;
        assert!(permissions.allows_tool("self.camera.start_stream"));
        permissions.vision = false;
        assert!(!permissions.allows_tool("self.camera.start_stream"));
    }

    fn report(id: &str) -> RobotReport {
        RobotReport {
            robot_id: id.to_owned(),
            client_id: "3f2b9c1e-0000-4000-8000-000000000001".to_owned(),
            board: "esp32-s3n16r8-emoji".to_owned(),
            firmware_version: "1.9.0".to_owned(),
        }
    }

    #[tokio::test]
    async fn first_report_mints_token_and_activation_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();

        let (record, token) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:ff"), 1_700_000_000_000)
            .await
            .unwrap();

        assert_eq!(record.robot_id, "aa:bb:cc:dd:ee:ff");
        assert!(record.companion_id.is_none());
        assert_eq!(record.activation_code.as_deref().map(str::len), Some(6));
        assert!(
            record
                .activation_code
                .as_deref()
                .unwrap()
                .chars()
                .all(|c| c.is_ascii_digit())
        );
        assert_eq!(token.len(), 64, "token is 256-bit hex");
        assert_ne!(record.token_hash, token, "only the hash is persisted");
        assert_eq!(
            reg.resolve_token(&token).await.unwrap().robot_id,
            record.robot_id
        );
    }

    #[tokio::test]
    async fn failed_persistence_never_publishes_permission_or_pairing_authority() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        let (before, _) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:ff"), 1)
            .await
            .unwrap();
        let code = before.activation_code.clone().unwrap();
        let path = dir.path().join(ROBOT_REL_DIR).join(ROBOTS_FILE);
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        let mut permissions = before.permissions.clone();
        permissions.motion = true;
        assert!(reg
            .set_permissions("aa:bb:cc:dd:ee:ff", permissions)
            .await
            .is_err());
        assert_eq!(reg.get("aa:bb:cc:dd:ee:ff").await.unwrap(), before);

        assert!(matches!(
            reg.claim(&code, "companion-1").await,
            Err(ClaimError::Persistence(_))
        ));
        assert_eq!(reg.get("aa:bb:cc:dd:ee:ff").await.unwrap(), before);
    }

    #[tokio::test]
    async fn re_report_rotates_token_and_keeps_activation_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        let (first, token_a) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:01"), 1)
            .await
            .unwrap();
        let (second, token_b) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:01"), 2)
            .await
            .unwrap();

        assert_ne!(token_a, token_b, "each report mints a fresh token");
        assert_eq!(
            first.activation_code, second.activation_code,
            "code is stable while unbound"
        );
        assert_eq!(first.created_at, second.created_at);
        assert_eq!(second.last_seen, Some(2));
        assert!(
            reg.resolve_token(&token_a).await.is_none(),
            "old token is invalidated"
        );
        assert!(reg.resolve_token(&token_b).await.is_some());
    }

    #[tokio::test]
    async fn claim_binds_companion_and_clears_code() {
        let dir = tempfile::tempdir().unwrap();
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        let (record, _) = reg
            .upsert_on_report(report("aa:bb:cc:dd:ee:02"), 1)
            .await
            .unwrap();
        let code = record.activation_code.clone().unwrap();

        let bound = reg
            .claim(&code, "0190f5fe-7c00-7a00-8000-0000000000aa")
            .await
            .unwrap();
        assert_eq!(
            bound.companion_id.as_deref(),
            Some("0190f5fe-7c00-7a00-8000-0000000000aa")
        );
        assert!(bound.activation_code.is_none());

        assert!(matches!(
            reg.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000bb").await,
            Err(ClaimError::NotFound)
        ));
    }

    #[tokio::test]
    async fn state_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let (code, token) = {
            let reg = RobotRegistry::load(dir.path()).await.unwrap();
            let (record, token) = reg
                .upsert_on_report(report("aa:bb:cc:dd:ee:03"), 1)
                .await
                .unwrap();
            (record.activation_code.unwrap(), token)
        };
        let reg = RobotRegistry::load(dir.path()).await.unwrap();
        assert_eq!(reg.list().await.len(), 1);
        assert!(reg.resolve_token(&token).await.is_some());
        assert!(
            reg.claim(&code, "0190f5fe-7c00-7a00-8000-0000000000cc")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn pairing_changes_revoke_connections_and_permissions_advance_authority() {
        let dir = tempfile::tempdir().unwrap();
        let registry = RobotRegistry::load(dir.path()).await.unwrap();
        let (record, _) = registry.upsert_on_report(report("robot-1"), 1).await.unwrap();
        let companion = "0190f5fe-7c00-7a00-8000-0000000000aa";
        registry.claim(record.activation_code.as_deref().unwrap(), companion).await.unwrap();
        registry.connect("robot-1", companion, "socket-1").await.unwrap();
        registry.connect("robot-1", companion, "socket-2").await.unwrap();
        assert!(!registry.disconnect("robot-1", "socket-1").await);
        assert!(registry.connection_matches("robot-1", "socket-2").await);
        let before = registry.get("robot-1").await.unwrap();
        let mut permissions = before.permissions;
        permissions.motion = true;
        let after = registry.set_permissions("robot-1", permissions).await.unwrap();
        assert!(after.authorization_revision > before.authorization_revision);
        registry.patch("robot-1", None, Some(None)).await.unwrap();
        assert!(!registry.connection_matches("robot-1", "socket-2").await);
        assert!(registry.connect("robot-1", companion, "socket-3").await.is_err());
    }

    #[tokio::test]
    async fn physical_action_lease_serializes_permission_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
        let (record, _) = registry.upsert_on_report(report("robot-1"), 1).await.unwrap();
        registry
            .claim(record.activation_code.as_deref().unwrap(), "companion-1")
            .await
            .unwrap();
        registry
            .connect("robot-1", "companion-1", "socket-1")
            .await
            .unwrap();
        let lease = registry
            .hold_action_connection("robot-1", "socket-1")
            .await
            .unwrap();
        let updating = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                let mut permissions = RobotPermissions::default();
                permissions.motion = true;
                registry
                    .set_permissions("robot-1", permissions)
                    .await
                    .unwrap();
            })
        };
        tokio::pin!(updating);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), &mut updating)
                .await
                .is_err(),
            "permission mutation must wait for the physical action receipt boundary"
        );
        drop(lease);
        tokio::time::timeout(std::time::Duration::from_secs(1), &mut updating)
            .await
            .expect("permission mutation should continue after action settlement")
            .unwrap();
    }
}
