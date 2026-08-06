//! Robot registry: `{data_dir}/robot/robots.json`, atomic temp+rename writes.
//!
//! Tokens are persisted as SHA-256 only. A fresh token is minted on **every**
//! OTA report because the firmware re-reads `websocket.token` from each response
//! and persists it to NVS — so rotation-per-boot is transparent, and an already
//! authenticated WebSocket keeps working until it drops.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;

/// Subdirectory of the backend data dir holding robot state.
pub const ROBOT_REL_DIR: &str = "robot";
/// Registry file name inside [`ROBOT_REL_DIR`].
pub const ROBOTS_FILE: &str = "robots.json";

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
            Ok(bytes) => serde_json::from_slice::<RegistryFile>(&bytes)
                .unwrap_or_default()
                .robots,
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
        })
    }

    async fn persist(&self, map: &BTreeMap<String, RobotRecord>) -> anyhow::Result<()> {
        let file = RegistryFile {
            robots: map.values().cloned().collect(),
        };
        let bytes = serde_json::to_vec_pretty(&file)?;
        let tmp = self.path.with_extension("json.tmp");
        tokio::fs::write(&tmp, &bytes).await?;
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
        let token = mint_token();
        let token_hash = token_sha256_hex(&token);
        let mut map = self.inner.write().await;
        let record = match map.get_mut(&report.robot_id) {
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
                map.insert(record.robot_id.clone(), record.clone());
                record
            }
        };
        self.persist(&map).await?;
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
        let mut map = self.inner.write().await;
        let record = map
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
        let _ = self.persist(&map).await;
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
        let mut map = self.inner.write().await;
        let record = map.get_mut(robot_id).ok_or(ClaimError::NotFound)?;
        if let Some(name) = name {
            record.name = name;
        }
        if let Some(binding) = companion_id {
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
        let _ = self.persist(&map).await;
        Ok(out)
    }

    /// Remove a robot (revokes its token). Returns whether it existed.
    pub async fn remove(&self, robot_id: &str) -> anyhow::Result<bool> {
        let mut map = self.inner.write().await;
        let existed = map.remove(robot_id).is_some();
        if existed {
            self.persist(&map).await?;
        }
        Ok(existed)
    }

    /// All records, ordered by `robot_id`.
    pub async fn list(&self) -> Vec<RobotRecord> {
        self.inner.read().await.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
