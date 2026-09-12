//! Durable idempotency receipts for Agent-initiated physical robot effects.
//!
//! A reservation is persisted before a device call is dispatched. If the
//! process, Turn, or link disappears before a terminal receipt is stored, the
//! reservation remains unresolved and the same idempotency identity is never
//! sent to the device automatically again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

const LEDGER_FILE: &str = "agent-effect-receipts.json";
const MAX_EFFECT_RECEIPTS: usize = 16_384;
// A terminal receipt remains a replay fence for the complete retry horizon.
// Once it is older than this window it may be discarded to keep the durable
// ledger usable for future Sessions. Unresolved physical outcomes are never
// discarded: losing one of those fences could repeat an effect after a crash.
const TERMINAL_RECEIPT_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const RESTARTED_RESERVATION_REASON: &str =
    "the physical effect was reserved before restart and has no durable terminal receipt";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RobotEffectKey {
    pub principal_id: String,
    pub agent_session_id: String,
    pub capability_id: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotEffectRequest {
    pub key: RobotEffectKey,
    pub input_digest: String,
    pub robot_id: String,
    pub tool_name: String,
    pub reserved_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RobotEffectReservation {
    pub key: RobotEffectKey,
    pub input_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RobotEffectAdmission {
    Dispatch(RobotEffectReservation),
    Completed { output: String },
    Failed { code: String, message: String },
    OutcomeUnknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum RobotEffectState {
    Reserved,
    Completed { output: String },
    Failed { code: String, message: String },
    OutcomeUnknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RobotEffectReceipt {
    key: RobotEffectKey,
    input_digest: String,
    robot_id: String,
    tool_name: String,
    reserved_at_ms: i64,
    updated_at_ms: i64,
    outcome: RobotEffectState,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LedgerFile {
    #[serde(default)]
    receipts: Vec<RobotEffectReceipt>,
}

pub struct RobotEffectLedger {
    path: PathBuf,
    inner: Mutex<BTreeMap<RobotEffectKey, RobotEffectReceipt>>,
}

impl RobotEffectLedger {
    pub async fn load(data_dir: &Path) -> anyhow::Result<Self> {
        let dir = data_dir.join(crate::registry::ROBOT_REL_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        let path = dir.join(LEDGER_FILE);
        let file = match tokio::fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<LedgerFile>(&bytes).map_err(|error| {
                anyhow::anyhow!(
                    "robot effect ledger {} is invalid: {error}",
                    path.display()
                )
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => LedgerFile::default(),
            Err(error) => return Err(error.into()),
        };
        let mut receipts = BTreeMap::new();
        for receipt in file.receipts {
            if receipts.insert(receipt.key.clone(), receipt).is_some() {
                anyhow::bail!("robot effect ledger contains duplicate idempotency identities");
            }
        }
        // A persisted reservation means dispatch may already have reached the
        // device. Reconcile it to an explicit sticky unknown outcome before
        // accepting any new effect; it must never become eligible for terminal
        // receipt compaction.
        let mut reconciled = false;
        let restarted_at_ms = now_ms();
        for receipt in receipts.values_mut() {
            if matches!(receipt.outcome, RobotEffectState::Reserved) {
                receipt.outcome = RobotEffectState::OutcomeUnknown {
                    reason: RESTARTED_RESERVATION_REASON.to_owned(),
                };
                receipt.updated_at_ms = restarted_at_ms.max(receipt.updated_at_ms);
                reconciled = true;
            }
        }
        let compacted = compact_expired_terminal_receipts(
            &mut receipts,
            restarted_at_ms,
            MAX_EFFECT_RECEIPTS,
        );
        if receipts.len() > MAX_EFFECT_RECEIPTS {
            anyhow::bail!(
                "robot effect ledger exceeds the {MAX_EFFECT_RECEIPTS}-receipt safety bound after retaining every unresolved physical-effect fence"
            );
        }
        let ledger = Self {
            path,
            inner: Mutex::new(receipts),
        };
        if reconciled || compacted > 0 {
            let receipts = ledger.inner.lock().await;
            ledger.persist(&receipts).await?;
        }
        Ok(ledger)
    }

    pub async fn admit(
        &self,
        request: RobotEffectRequest,
    ) -> anyhow::Result<RobotEffectAdmission> {
        validate_request(&request)?;
        let mut receipts = self.inner.lock().await;
        if let Some(existing) = receipts.get(&request.key) {
            if existing.input_digest != request.input_digest
                || existing.robot_id != request.robot_id
                || existing.tool_name != request.tool_name
            {
                anyhow::bail!(
                    "robot effect idempotency key was reused with a different request"
                );
            }
            return Ok(match &existing.outcome {
                RobotEffectState::Completed { output } => RobotEffectAdmission::Completed {
                    output: output.clone(),
                },
                RobotEffectState::Failed { code, message } => RobotEffectAdmission::Failed {
                    code: code.clone(),
                    message: message.clone(),
                },
                RobotEffectState::Reserved => RobotEffectAdmission::OutcomeUnknown {
                    reason: "the physical effect was reserved but has no durable terminal receipt"
                        .to_owned(),
                },
                RobotEffectState::OutcomeUnknown { reason } => {
                    RobotEffectAdmission::OutcomeUnknown {
                        reason: reason.clone(),
                    }
                }
            });
        }
        // Bound growth without turning one busy installation into a permanent
        // outage. Only settled receipts beyond the retry horizon are eligible;
        // Reserved/OutcomeUnknown entries remain sticky fail-closed fences.
        let before_compaction = (receipts.len() >= MAX_EFFECT_RECEIPTS)
            .then(|| receipts.clone());
        compact_expired_terminal_receipts(
            &mut receipts,
            now_ms(),
            MAX_EFFECT_RECEIPTS.saturating_sub(1),
        );
        if receipts.len() >= MAX_EFFECT_RECEIPTS {
            anyhow::bail!(
                "robot effect ledger is full of live retry receipts or unresolved physical-effect fences; refusing an effect whose idempotency cannot be retained"
            );
        }
        let receipt = RobotEffectReceipt {
            key: request.key.clone(),
            input_digest: request.input_digest.clone(),
            robot_id: request.robot_id,
            tool_name: request.tool_name,
            reserved_at_ms: request.reserved_at_ms,
            updated_at_ms: request.reserved_at_ms,
            outcome: RobotEffectState::Reserved,
        };
        receipts.insert(request.key.clone(), receipt);
        if let Err(error) = self.persist(&receipts).await {
            if let Some(before_compaction) = before_compaction {
                *receipts = before_compaction;
            } else {
                receipts.remove(&request.key);
            }
            return Err(error);
        }
        Ok(RobotEffectAdmission::Dispatch(RobotEffectReservation {
            key: request.key,
            input_digest: request.input_digest,
        }))
    }

    pub async fn complete(
        &self,
        reservation: &RobotEffectReservation,
        output: String,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.settle(
            reservation,
            RobotEffectState::Completed { output },
            now_ms,
        )
        .await
    }

    pub async fn fail(
        &self,
        reservation: &RobotEffectReservation,
        code: String,
        message: String,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.settle(
            reservation,
            RobotEffectState::Failed { code, message },
            now_ms,
        )
        .await
    }

    pub async fn mark_outcome_unknown(
        &self,
        reservation: &RobotEffectReservation,
        reason: String,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        self.settle(
            reservation,
            RobotEffectState::OutcomeUnknown { reason },
            now_ms,
        )
        .await
    }

    async fn settle(
        &self,
        reservation: &RobotEffectReservation,
        outcome: RobotEffectState,
        now_ms: i64,
    ) -> anyhow::Result<()> {
        let mut receipts = self.inner.lock().await;
        let previous = receipts
            .get(&reservation.key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("robot effect reservation disappeared"))?;
        if previous.input_digest != reservation.input_digest {
            anyhow::bail!("robot effect reservation digest changed");
        }
        if !matches!(previous.outcome, RobotEffectState::Reserved) {
            anyhow::bail!("robot effect reservation is already terminal");
        }
        let receipt = receipts
            .get_mut(&reservation.key)
            .expect("checked reservation exists");
        receipt.outcome = outcome;
        receipt.updated_at_ms = now_ms;
        if let Err(error) = self.persist(&receipts).await {
            receipts.insert(reservation.key.clone(), previous);
            return Err(error);
        }
        Ok(())
    }

    async fn persist(
        &self,
        receipts: &BTreeMap<RobotEffectKey, RobotEffectReceipt>,
    ) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(&LedgerFile {
            receipts: receipts.values().cloned().collect(),
        })?;
        let temporary = self.path.with_extension("json.tmp");
        tokio::fs::write(&temporary, bytes).await?;
        tokio::fs::rename(&temporary, &self.path).await?;
        Ok(())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

/// Remove the oldest expired terminal receipts until `target_len` is met.
///
/// Terminal entries are replay caches, not ambiguous-effect fences. They stay
/// durable for the full retry horizon; after that, the Session-scoped
/// operation identity is outside the supported replay window. Reserved and
/// OutcomeUnknown entries are deliberately ineligible regardless of age.
fn compact_expired_terminal_receipts(
    receipts: &mut BTreeMap<RobotEffectKey, RobotEffectReceipt>,
    now_ms: i64,
    target_len: usize,
) -> usize {
    if receipts.len() <= target_len {
        return 0;
    }
    let expires_before = now_ms.saturating_sub(TERMINAL_RECEIPT_RETENTION_MS);
    let mut eligible = receipts
        .iter()
        .filter_map(|(key, receipt)| {
            (matches!(
                receipt.outcome,
                RobotEffectState::Completed { .. } | RobotEffectState::Failed { .. }
            ) && receipt.updated_at_ms <= expires_before)
                .then_some((receipt.updated_at_ms, key.clone()))
        })
        .collect::<Vec<_>>();
    eligible.sort_by(|left, right| left.cmp(right));
    let remove_count = receipts.len().saturating_sub(target_len).min(eligible.len());
    for (_, key) in eligible.into_iter().take(remove_count) {
        receipts.remove(&key);
    }
    remove_count
}

fn validate_request(request: &RobotEffectRequest) -> anyhow::Result<()> {
    for (field, value) in [
        ("principal_id", request.key.principal_id.as_str()),
        ("agent_session_id", request.key.agent_session_id.as_str()),
        ("capability_id", request.key.capability_id.as_str()),
        ("idempotency_key", request.key.idempotency_key.as_str()),
        ("input_digest", request.input_digest.as_str()),
        ("robot_id", request.robot_id.as_str()),
        ("tool_name", request.tool_name.as_str()),
    ] {
        if value.trim().is_empty() || value.len() > 1_024 {
            anyhow::bail!("{field} must be non-empty and at most 1024 bytes");
        }
    }
    if request.reserved_at_ms <= 0 {
        anyhow::bail!("reserved_at_ms must be positive");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(key: &str, digest: &str) -> RobotEffectRequest {
        RobotEffectRequest {
            key: RobotEffectKey {
                principal_id: "owner".to_owned(),
                agent_session_id: "session".to_owned(),
                capability_id: "robot.motion".to_owned(),
                idempotency_key: key.to_owned(),
            },
            input_digest: digest.to_owned(),
            robot_id: "robot-1".to_owned(),
            tool_name: "robot_head_look".to_owned(),
            reserved_at_ms: 1,
        }
    }

    #[tokio::test]
    async fn reservation_survives_restart_and_disables_unknown_retry() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = RobotEffectLedger::load(dir.path()).await.unwrap();
        assert!(matches!(
            ledger.admit(request("key-1", "digest-1")).await.unwrap(),
            RobotEffectAdmission::Dispatch(_)
        ));
        drop(ledger);

        let restarted = RobotEffectLedger::load(dir.path()).await.unwrap();
        assert!(matches!(
            restarted.admit(request("key-1", "digest-1")).await.unwrap(),
            RobotEffectAdmission::OutcomeUnknown { .. }
        ));
        let persisted = tokio::fs::read(
            dir.path()
                .join(crate::registry::ROBOT_REL_DIR)
                .join(LEDGER_FILE),
        )
        .await
        .unwrap();
        let persisted: LedgerFile = serde_json::from_slice(&persisted).unwrap();
        assert!(matches!(
            persisted.receipts[0].outcome,
            RobotEffectState::OutcomeUnknown { .. }
        ));
        assert!(restarted
            .admit(request("key-1", "different"))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn terminal_receipts_replay_without_redispatch() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = RobotEffectLedger::load(dir.path()).await.unwrap();
        let reservation = match ledger.admit(request("key-2", "digest-2")).await.unwrap() {
            RobotEffectAdmission::Dispatch(reservation) => reservation,
            other => panic!("unexpected admission {other:?}"),
        };
        ledger
            .complete(&reservation, "turned".to_owned(), 2)
            .await
            .unwrap();
        assert_eq!(
            ledger.admit(request("key-2", "digest-2")).await.unwrap(),
            RobotEffectAdmission::Completed {
                output: "turned".to_owned()
            }
        );
        drop(ledger);
        let restarted = RobotEffectLedger::load(dir.path()).await.unwrap();
        assert_eq!(
            restarted.admit(request("key-2", "digest-2")).await.unwrap(),
            RobotEffectAdmission::Completed {
                output: "turned".to_owned()
            }
        );
    }

    #[tokio::test]
    async fn failed_receipt_replays_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = RobotEffectLedger::load(dir.path()).await.unwrap();
        let reservation = match ledger.admit(request("key-fail", "digest-fail")).await.unwrap() {
            RobotEffectAdmission::Dispatch(reservation) => reservation,
            other => panic!("unexpected admission {other:?}"),
        };
        ledger
            .fail(
                &reservation,
                "DEVICE_REJECTED".to_owned(),
                "blocked".to_owned(),
                now_ms(),
            )
            .await
            .unwrap();
        drop(ledger);

        let restarted = RobotEffectLedger::load(dir.path()).await.unwrap();
        assert_eq!(
            restarted
                .admit(request("key-fail", "digest-fail"))
                .await
                .unwrap(),
            RobotEffectAdmission::Failed {
                code: "DEVICE_REJECTED".to_owned(),
                message: "blocked".to_owned(),
            }
        );
    }

    #[test]
    fn compaction_removes_only_expired_terminal_receipts_and_retains_fences() {
        let current = TERMINAL_RECEIPT_RETENTION_MS * 2;
        let mut receipts = BTreeMap::new();
        for (key, outcome, updated_at_ms) in [
            (
                "completed-old",
                RobotEffectState::Completed { output: "ok".to_owned() },
                1,
            ),
            (
                "failed-old",
                RobotEffectState::Failed {
                    code: "FAILED".to_owned(),
                    message: "no".to_owned(),
                },
                2,
            ),
            ("reserved-old", RobotEffectState::Reserved, 1),
            (
                "unknown-old",
                RobotEffectState::OutcomeUnknown {
                    reason: "ambiguous".to_owned(),
                },
                1,
            ),
            (
                "completed-current",
                RobotEffectState::Completed { output: "ok".to_owned() },
                current,
            ),
        ] {
            let request = request(key, key);
            receipts.insert(
                request.key.clone(),
                RobotEffectReceipt {
                    key: request.key,
                    input_digest: request.input_digest,
                    robot_id: request.robot_id,
                    tool_name: request.tool_name,
                    reserved_at_ms: 1,
                    updated_at_ms,
                    outcome,
                },
            );
        }

        assert_eq!(compact_expired_terminal_receipts(&mut receipts, current, 2), 2);
        assert!(!receipts.keys().any(|key| key.idempotency_key == "completed-old"));
        assert!(!receipts.keys().any(|key| key.idempotency_key == "failed-old"));
        assert!(receipts.keys().any(|key| key.idempotency_key == "reserved-old"));
        assert!(receipts.keys().any(|key| key.idempotency_key == "unknown-old"));
        assert!(receipts
            .keys()
            .any(|key| key.idempotency_key == "completed-current"));
    }
}
