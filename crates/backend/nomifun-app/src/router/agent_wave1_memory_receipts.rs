//! Durable, Session-scoped exactly-once fence for Wave 1 Companion memory.
//!
//! Companion memory is stored by `CompanionService`, outside the Nomi Session
//! receipt transaction. Consequently a process can stop after the memory
//! mutation commits but before its receipt is settled. Such a receipt is never
//! retried: a new process promotes it to `outcome_unknown` and requires user
//! reconciliation. This preserves at-most-once effects without pretending a
//! cross-database transaction exists.

use std::sync::Arc;

use nomifun_agent_contracts::{StrictJsonValue, digest_payload};
use nomifun_agent_domain_wave1::Wave1HostPortError;
use sqlx::{Row, SqlitePool};

pub(super) const IDEMPOTENCY_CONFLICT: &str =
    "WAVE1_MEMORY_IDEMPOTENCY_CONFLICT";
pub(super) const ACTION_IN_PROGRESS: &str =
    "WAVE1_MEMORY_ACTION_IN_PROGRESS";
pub(super) const ACTION_OUTCOME_UNKNOWN: &str =
    "WAVE1_MEMORY_ACTION_OUTCOME_UNKNOWN";
const LEDGER_FAILED: &str = "WAVE1_MEMORY_IDEMPOTENCY_LEDGER_FAILED";

// Reclaim only old receipts whose Session no longer exists. The grace period
// protects a newly-created Session from a transient visibility/order gap, and
// the batch bound keeps cleanup latency independent from historical volume.
const ORPHAN_GRACE_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const ORPHAN_SWEEP_BATCH: i64 = 128;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ReceiptKey {
    owner_user_id: String,
    agent_session_id: String,
    capability_id: String,
    idempotency_key: String,
}

#[derive(Debug)]
pub(super) enum ReceiptAdmission {
    Execute(MemoryActionGuard),
    Return(Result<StrictJsonValue, Wave1HostPortError>),
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MemoryActionReceiptContext<'a> {
    pub owner_user_id: &'a str,
    pub agent_session_id: &'a str,
    pub capability_id: &'a str,
    pub action_id: &'a str,
    pub idempotency_key: &'a str,
    pub target_companion_id: &'a str,
}

#[derive(Clone, Debug)]
pub(super) struct Wave1MemoryActionLedger {
    pool: SqlitePool,
    process_lease_id: Arc<str>,
}

impl Wave1MemoryActionLedger {
    pub(super) fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            process_lease_id: Arc::from(nomifun_common::generate_id()),
        }
    }

    #[cfg(test)]
    fn with_lease(pool: SqlitePool, process_lease_id: &str) -> Self {
        Self {
            pool,
            process_lease_id: Arc::from(process_lease_id),
        }
    }

    pub(super) async fn admit(
        &self,
        context: MemoryActionReceiptContext<'_>,
        request: &StrictJsonValue,
    ) -> Result<ReceiptAdmission, Wave1HostPortError> {
        let request_envelope = StrictJsonValue(serde_json::json!({
            "action_id": context.action_id,
            "target": {
                "resource_kind": "companion_memory",
                "resource_id": context.target_companion_id,
            },
            "request": &request.0,
        }));
        let request_digest = digest_payload(&request_envelope)
            .map_err(|error| ledger_error(error.to_string()))?;
        let key = ReceiptKey {
            owner_user_id: context.owner_user_id.to_owned(),
            agent_session_id: context.agent_session_id.to_owned(),
            capability_id: context.capability_id.to_owned(),
            idempotency_key: context.idempotency_key.to_owned(),
        };
        let now = nomifun_common::now_ms();
        let inserted = sqlx::query(
            "INSERT OR IGNORE INTO nomi_wave1_memory_action_receipts(
                owner_user_id, agent_session_id, capability_id, idempotency_key,
                request_digest, state, process_lease_id, created_at, updated_at
             ) VALUES (?, ?, ?, ?, ?, 'in_flight', ?, ?, ?)",
        )
        .bind(&key.owner_user_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .bind(request_digest.as_ref())
        .bind(self.process_lease_id.as_ref())
        .bind(now)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?;

        if inserted.rows_affected() == 1 {
            // Cleanup is deliberately best-effort and cannot affect this new
            // receipt because it is neither old nor orphan-qualified.
            if let Err(error) = self.reclaim_old_orphans(now).await {
                tracing::warn!(error = %error, "Wave 1 memory receipt cleanup failed");
            }
            return Ok(ReceiptAdmission::Execute(MemoryActionGuard {
                ledger: self.clone(),
                key,
                armed: true,
            }));
        }

        let row = sqlx::query(
            "SELECT request_digest, state, process_lease_id, output_json,
                    error_code, error_message
             FROM nomi_wave1_memory_action_receipts
             WHERE owner_user_id = ? AND agent_session_id = ?
               AND capability_id = ? AND idempotency_key = ?",
        )
        .bind(&key.owner_user_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(ledger_error)?
        .ok_or_else(|| ledger_error("receipt disappeared after key conflict"))?;

        let existing_digest: String =
            row.try_get("request_digest").map_err(ledger_error)?;
        if existing_digest != request_digest.as_ref() {
            return Err(Wave1HostPortError::new(
                IDEMPOTENCY_CONFLICT,
                "the Wave 1 memory idempotency key was reused with a different request or Companion target",
            ));
        }

        let state: String = row.try_get("state").map_err(ledger_error)?;
        let result = match state.as_str() {
            "completed" => {
                let output: String = row.try_get("output_json").map_err(ledger_error)?;
                let value = serde_json::from_str(&output).map_err(|error| {
                    ledger_error(format!("persisted output is invalid: {error}"))
                })?;
                Ok(StrictJsonValue(value))
            }
            "failed" => {
                let code: String = row.try_get("error_code").map_err(ledger_error)?;
                let message: String =
                    row.try_get("error_message").map_err(ledger_error)?;
                Err(Wave1HostPortError::new(code, message))
            }
            "outcome_unknown" => Err(outcome_unknown()),
            "in_flight" => {
                let lease: String =
                    row.try_get("process_lease_id").map_err(ledger_error)?;
                if lease == self.process_lease_id.as_ref() {
                    Err(Wave1HostPortError::new(
                        ACTION_IN_PROGRESS,
                        "the original Wave 1 memory action is still in flight",
                    ))
                } else {
                    sqlx::query(
                        "UPDATE nomi_wave1_memory_action_receipts
                         SET state = 'outcome_unknown', process_lease_id = NULL,
                             updated_at = ?
                         WHERE owner_user_id = ? AND agent_session_id = ?
                           AND capability_id = ? AND idempotency_key = ?
                           AND state = 'in_flight' AND process_lease_id = ?",
                    )
                    .bind(now)
                    .bind(&key.owner_user_id)
                    .bind(&key.agent_session_id)
                    .bind(&key.capability_id)
                    .bind(&key.idempotency_key)
                    .bind(lease)
                    .execute(&self.pool)
                    .await
                    .map_err(ledger_error)?;
                    Err(outcome_unknown())
                }
            }
            other => {
                return Err(ledger_error(format!(
                    "receipt has unsupported state {other}"
                )));
            }
        };
        Ok(ReceiptAdmission::Return(result))
    }

    pub(super) async fn settle(
        &self,
        guard: &mut MemoryActionGuard,
        result: &Result<StrictJsonValue, Wave1HostPortError>,
    ) -> Result<(), Wave1HostPortError> {
        let now = nomifun_common::now_ms();
        let changed = match result {
            Ok(output) => {
                let output = match serde_json::to_string(&output.0) {
                    Ok(output) => output,
                    Err(error) => {
                        tracing::error!(
                            error = %error,
                            "Wave 1 memory result could not be encoded for its receipt"
                        );
                        let _ = self.mark_outcome_unknown(&guard.key).await;
                        return Err(outcome_unknown());
                    }
                };
                sqlx::query(
                    "UPDATE nomi_wave1_memory_action_receipts
                     SET state = 'completed', process_lease_id = NULL,
                         output_json = ?, updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(output)
                .bind(now)
                .bind(&guard.key.owner_user_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
            }
            Err(error) if error.code.as_ref() == ACTION_OUTCOME_UNKNOWN => {
                sqlx::query(
                    "UPDATE nomi_wave1_memory_action_receipts
                     SET state = 'outcome_unknown', process_lease_id = NULL,
                         updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(now)
                .bind(&guard.key.owner_user_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
            }
            Err(error) => {
                sqlx::query(
                    "UPDATE nomi_wave1_memory_action_receipts
                     SET state = 'failed', process_lease_id = NULL,
                         error_code = ?, error_message = ?, updated_at = ?
                     WHERE owner_user_id = ? AND agent_session_id = ?
                       AND capability_id = ? AND idempotency_key = ?
                       AND state = 'in_flight' AND process_lease_id = ?",
                )
                .bind(error.code.as_ref())
                .bind(&error.message)
                .bind(now)
                .bind(&guard.key.owner_user_id)
                .bind(&guard.key.agent_session_id)
                .bind(&guard.key.capability_id)
                .bind(&guard.key.idempotency_key)
                .bind(self.process_lease_id.as_ref())
                .execute(&self.pool)
                .await
            }
        };
        let changed = match changed {
            Ok(changed) => changed,
            Err(error) => {
                tracing::error!(
                    error = %error,
                    "Wave 1 memory action finished but its receipt could not be settled"
                );
                let _ = self.mark_outcome_unknown(&guard.key).await;
                return Err(outcome_unknown());
            }
        };
        if changed.rows_affected() != 1 {
            return Err(outcome_unknown());
        }
        guard.armed = false;
        Ok(())
    }

    async fn mark_outcome_unknown(
        &self,
        key: &ReceiptKey,
    ) -> Result<(), Wave1HostPortError> {
        sqlx::query(
            "UPDATE nomi_wave1_memory_action_receipts
             SET state = 'outcome_unknown', process_lease_id = NULL,
                 updated_at = ?
             WHERE owner_user_id = ? AND agent_session_id = ?
               AND capability_id = ? AND idempotency_key = ?
               AND state = 'in_flight' AND process_lease_id = ?",
        )
        .bind(nomifun_common::now_ms())
        .bind(&key.owner_user_id)
        .bind(&key.agent_session_id)
        .bind(&key.capability_id)
        .bind(&key.idempotency_key)
        .bind(self.process_lease_id.as_ref())
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?;
        Ok(())
    }

    async fn reclaim_old_orphans(&self, now: i64) -> Result<u64, Wave1HostPortError> {
        let cutoff = now.saturating_sub(ORPHAN_GRACE_MS);
        Ok(sqlx::query(
            "DELETE FROM nomi_wave1_memory_action_receipts
             WHERE rowid IN (
                 SELECT receipt.rowid
                 FROM nomi_wave1_memory_action_receipts receipt
                 WHERE receipt.updated_at < ?
                   AND NOT EXISTS (
                       SELECT 1 FROM conversations conversation
                       WHERE conversation.conversation_id = receipt.agent_session_id
                         AND conversation.user_id = receipt.owner_user_id
                   )
                 ORDER BY receipt.updated_at ASC
                 LIMIT ?
             )",
        )
        .bind(cutoff)
        .bind(ORPHAN_SWEEP_BATCH)
        .execute(&self.pool)
        .await
        .map_err(ledger_error)?
        .rows_affected())
    }
}

#[derive(Debug)]
pub(super) struct MemoryActionGuard {
    ledger: Wave1MemoryActionLedger,
    key: ReceiptKey,
    armed: bool,
}

impl Drop for MemoryActionGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let ledger = self.ledger.clone();
        let key = self.key.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = ledger.mark_outcome_unknown(&key).await {
                    tracing::error!(
                        error = %error,
                        "cancelled Wave 1 memory receipt could not be fenced"
                    );
                }
            });
        }
    }
}

fn outcome_unknown() -> Wave1HostPortError {
    Wave1HostPortError::new(
        ACTION_OUTCOME_UNKNOWN,
        "the original Wave 1 memory action may have committed; automatic replay is forbidden",
    )
}

fn ledger_error(error: impl std::fmt::Display) -> Wave1HostPortError {
    Wave1HostPortError::new(
        LEDGER_FAILED,
        format!("Wave 1 memory idempotency ledger failed: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use nomifun_agent_domain_wave1::{
        MEMORY_COMPANION_WRITE, MEMORY_COMPANION_WRITE_ACTION,
    };

    use super::*;

    const OWNER_ID: &str = "0199a000-0000-7000-8000-000000000000";
    const SESSION_ID: &str = "0199a000-0000-7000-8000-000000000001";
    const LEASE_A: &str = "0199a000-0000-7000-8000-000000000002";
    const LEASE_B: &str = "0199a000-0000-7000-8000-000000000003";
    const LEASE_C: &str = "0199a000-0000-7000-8000-000000000004";
    const LEASE_D: &str = "0199a000-0000-7000-8000-000000000005";
    const DIGEST: &str =
        "0000000000000000000000000000000000000000000000000000000000000000";

    fn context(idempotency_key: &str) -> MemoryActionReceiptContext<'_> {
        MemoryActionReceiptContext {
            owner_user_id: OWNER_ID,
            agent_session_id: SESSION_ID,
            capability_id: MEMORY_COMPANION_WRITE,
            action_id: MEMORY_COMPANION_WRITE_ACTION,
            idempotency_key,
            target_companion_id: "companion-a",
        }
    }

    #[tokio::test]
    async fn completed_receipt_replays_after_restart_and_changed_request_conflicts() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let first = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_A,
        );
        let context = context("same-key");
        let request = StrictJsonValue(serde_json::json!({"content":"alpha"}));
        let ReceiptAdmission::Execute(mut guard) = first
            .admit(context, &request)
            .await
            .unwrap()
        else {
            panic!("first request must execute");
        };
        let output = StrictJsonValue(serde_json::json!({"memory_id":"m1"}));
        first.settle(&mut guard, &Ok(output.clone())).await.unwrap();

        let restarted = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_B,
        );
        let ReceiptAdmission::Return(Ok(replayed)) = restarted
            .admit(context, &request)
            .await
            .unwrap()
        else {
            panic!("completed action must replay its original output");
        };
        assert_eq!(replayed, output);

        let changed = StrictJsonValue(serde_json::json!({"content":"beta"}));
        assert_eq!(
            restarted
                .admit(context, &changed)
                .await
                .unwrap_err()
                .code
                .as_ref(),
            IDEMPOTENCY_CONFLICT
        );
        assert_eq!(
            restarted
                .admit(
                    MemoryActionReceiptContext {
                        target_companion_id: "companion-b",
                        ..context
                    },
                    &request,
                )
                .await
                .unwrap_err()
                .code
                .as_ref(),
            IDEMPOTENCY_CONFLICT
        );
        database.close().await;
    }

    #[tokio::test]
    async fn crashed_process_is_promoted_to_durable_outcome_unknown() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let context = context("crash-key");
        let request = StrictJsonValue(serde_json::json!({"content":"alpha"}));
        let crashed = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_C,
        );
        let ReceiptAdmission::Execute(guard) = crashed
            .admit(context, &request)
            .await
            .unwrap()
        else {
            panic!("first request must execute");
        };
        std::mem::forget(guard);

        let restarted = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_D,
        );
        let ReceiptAdmission::Return(Err(error)) = restarted
            .admit(context, &request)
            .await
            .unwrap()
        else {
            panic!("an abandoned receipt must never execute again");
        };
        assert_eq!(error.code.as_ref(), ACTION_OUTCOME_UNKNOWN);
        let state: String = sqlx::query_scalar(
            "SELECT state FROM nomi_wave1_memory_action_receipts
             WHERE idempotency_key = 'crash-key'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(state, "outcome_unknown");
        database.close().await;
    }

    #[tokio::test]
    async fn cancellation_marks_unknown_and_concurrent_same_key_is_fenced() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let ledger = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_A,
        );
        let context = context("cancel-key");
        let request = StrictJsonValue(serde_json::json!({"content":"alpha"}));
        let (left, right) = tokio::join!(
            ledger.admit(context, &request),
            ledger.admit(context, &request)
        );
        let (guard, in_progress) = match (left.unwrap(), right.unwrap()) {
            (
                ReceiptAdmission::Execute(guard),
                ReceiptAdmission::Return(Err(error)),
            )
            | (
                ReceiptAdmission::Return(Err(error)),
                ReceiptAdmission::Execute(guard),
            ) => (guard, error),
            unexpected => panic!(
                "exactly one concurrent duplicate must execute, got {unexpected:?}"
            ),
        };
        assert_eq!(in_progress.code.as_ref(), ACTION_IN_PROGRESS);
        drop(guard);

        let mut state = String::new();
        for _ in 0..100 {
            state = sqlx::query_scalar(
                "SELECT state FROM nomi_wave1_memory_action_receipts
                 WHERE idempotency_key = 'cancel-key'",
            )
            .fetch_one(database.pool())
            .await
            .unwrap();
            if state == "outcome_unknown" {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(state, "outcome_unknown");
        let ReceiptAdmission::Return(Err(unknown)) = ledger
            .admit(context, &request)
            .await
            .unwrap()
        else {
            panic!("cancelled action must never execute again");
        };
        assert_eq!(unknown.code.as_ref(), ACTION_OUTCOME_UNKNOWN);
        database.close().await;
    }

    #[tokio::test]
    async fn orphan_cleanup_is_age_gated_bounded_and_preserves_live_sessions() {
        let database = nomifun_db::init_database_memory().await.unwrap();
        let old = nomifun_common::now_ms() - ORPHAN_GRACE_MS - 1;
        sqlx::query(
            "WITH RECURSIVE seq(value) AS (
                 SELECT 0 UNION ALL SELECT value + 1 FROM seq WHERE value < 199
             )
             INSERT INTO nomi_wave1_memory_action_receipts(
                 owner_user_id, agent_session_id, capability_id, idempotency_key,
                 request_digest, state, output_json, created_at, updated_at
             )
             SELECT ?, '0199a000-0000-7000-8001-' || printf('%012x', value),
                    'memory.companion.write', printf('old-%d', value),
                    ?, 'completed', '{}', ?, ?
             FROM seq",
        )
        .bind(OWNER_ID)
        .bind(DIGEST)
        .bind(old)
        .bind(old)
        .execute(database.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO nomi_wave1_memory_action_receipts(
                 owner_user_id, agent_session_id, capability_id, idempotency_key,
                 request_digest, state, output_json, created_at, updated_at
             ) VALUES (?, '0199a000-0000-7000-8002-000000000000',
                       'memory.companion.write', 'recent', ?,
                       'completed', '{}', ?, ?)",
        )
        .bind(OWNER_ID)
        .bind(DIGEST)
        .bind(nomifun_common::now_ms())
        .bind(nomifun_common::now_ms())
        .execute(database.pool())
        .await
        .unwrap();
        let ledger = Wave1MemoryActionLedger::with_lease(
            database.pool().clone(),
            LEASE_B,
        );
        assert_eq!(
            ledger
                .reclaim_old_orphans(nomifun_common::now_ms())
                .await
                .unwrap(),
            ORPHAN_SWEEP_BATCH as u64
        );
        let old_remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM nomi_wave1_memory_action_receipts
             WHERE idempotency_key LIKE 'old-%'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(old_remaining, 200 - ORPHAN_SWEEP_BATCH);
        let recent: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM nomi_wave1_memory_action_receipts
             WHERE idempotency_key = 'recent'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
        assert_eq!(recent, 1);
        database.close().await;
    }
}
