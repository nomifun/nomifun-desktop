//! An original tool/resource owner's immutable return awaiting its journal.
//! This owns no dispatch, execution permit or retry policy. A failed write
//! retains the exact record and result; only that record can be committed again.
use std::sync::Mutex;

use nomifun_common::AppError;

use super::EngineTurnJournal;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SettlementFailure {
    WriteUnconfirmed(String),
    OwnerStateUnconfirmed(String),
}

pub(in crate::router) struct OwnedJournalSettlement<T> {
    journal: EngineTurnJournal,
    payload: String,
    outcome: Mutex<Option<T>>,
    first_failure: Mutex<Option<SettlementFailure>>,
    committed: tokio::sync::Mutex<bool>,
}

impl<T> OwnedJournalSettlement<T> {
    pub(in crate::router) fn new(journal: EngineTurnJournal, payload: String, outcome: T) -> Self {
        Self { journal, payload, outcome: Mutex::new(Some(outcome)),
            first_failure: Mutex::new(None), committed: tokio::sync::Mutex::new(false) }
    }

    pub(in crate::router) fn is_unconfirmed(&self) -> Result<bool, AppError> {
        Ok(self.first_failure.lock().map_err(|_| failure("settlement diagnostic lock poisoned"))?.is_some())
    }

    pub(in crate::router) fn retain_owner_failure(&self, reason: &str) -> Result<(), AppError> {
        self.first_failure.lock().map_err(|_| failure("settlement diagnostic lock poisoned"))?
            .get_or_insert_with(|| SettlementFailure::OwnerStateUnconfirmed(reason.to_owned()));
        Ok(())
    }

    /// Returns the original result only to its original live caller. Cleanup
    /// commits the same record and then releases the retained result unread.
    pub(in crate::router) fn take_outcome(&self) -> Result<T, AppError> {
        self.outcome.lock().map_err(|_| failure("settlement outcome lock poisoned"))?
            .take().ok_or_else(|| failure("original settlement outcome was already consumed"))
    }

    pub(in crate::router) async fn commit(&self) -> Result<(), AppError> {
        let mut committed = self.committed.lock().await;
        if *committed { return Ok(()); }
        let write = async {
            self.journal.retry_pending_owner_observations().await?;
            self.journal.append_owner_settlement(self.payload.clone()).await
        }.await;
        match write {
            Ok(()) => { *committed = true; Ok(()) }
            Err(error) => {
                let mut original = self.first_failure.lock().map_err(|_| failure("settlement diagnostic lock poisoned"))?;
                let retained = original.get_or_insert_with(|| SettlementFailure::WriteUnconfirmed(
                        super::bounded_error(&nomi_redact::redact_secrets_owned(error.to_string()))));
                let (SettlementFailure::WriteUnconfirmed(first_failure) | SettlementFailure::OwnerStateUnconfirmed(first_failure)) = retained;
                tracing::warn!(%first_failure,"original owner result remains pending its exact journal receipt");
                Err(error)
            }
        }
    }
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Original owner settlement: {message}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::StrictJsonValue;
    use nomifun_db::sqlx;
    use serde_json::{Value, json};

    #[tokio::test]
    async fn resource_return_is_retained_until_its_original_record_is_confirmed() {
        let (journal, pool) = crate::router::engine_journal::test_fixture().await;
        journal.append(json!({"event":"host_resource_dispatch","operation_id":"resource-operation",
            "call_id":"resource-call","resource_kind":"mcp_server","resource_binding_id":"resource-binding",
            "request_sha256":"a".repeat(64)}).to_string(), None,
            crate::router::engine_journal::EngineJournalWrite::Progress).await.unwrap();
        sqlx::query("CREATE TRIGGER reject_resource_return BEFORE INSERT ON agent_events WHEN NEW.kind='runtime/progress-recorded' AND json_extract(NEW.inline_json,'$.event.event')='host_resource_settled' BEGIN SELECT RAISE(FAIL,'fixture resource return write unavailable'); END")
            .execute(&pool).await.unwrap();
        let original = StrictJsonValue(json!({"contents":[{"uri":"fixture://original","text":"original owner bytes"}]}));
        let pending = OwnedJournalSettlement::new(journal,
            json!({"event":"host_resource_settled","operation_id":"resource-operation","owner_returned":true}).to_string(),
            Ok::<_,AppError>(original.clone()));
        assert!(pending.commit().await.is_err());
        assert!(pending.is_unconfirmed().unwrap());
        sqlx::query("DROP TRIGGER reject_resource_return").execute(&pool).await.unwrap();
        pending.commit().await.unwrap();
        pending.commit().await.unwrap();
        assert_eq!(pending.take_outcome().unwrap().unwrap(), original);
        let records: Vec<String> = sqlx::query_scalar("SELECT inline_json FROM agent_events WHERE kind='runtime/progress-recorded' AND json_extract(inline_json,'$.event.event')='host_resource_settled'")
            .fetch_all(&pool).await.unwrap();
        assert_eq!(records.len(), 1);
        let value: Value = serde_json::from_str(&records[0]).unwrap();
        assert_eq!(value["event"], json!({"event":"host_resource_settled","operation_id":"resource-operation","owner_returned":true}));
        pool.close().await;
    }
}
