//! Provider limits are platform facts, not an engine's context strategy.
//! No credentials, mutable route selection or tokenizer claims are exposed.
use nomifun_chat_model_broker::ChatRouteSelection;
use nomifun_common::AppError;
use nomifun_db::{SqlitePool, sqlx};

use super::engine_session_host::AdmittedEngineSession;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineModelLimits {
    pub context_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct EngineRouteCandidateFacts {
    pub provider_id: String,
    pub model: String,
    pub limits: EngineModelLimits,
}

/// One consistent database observation of the exact saved primary + failovers.
/// Unknown limits remain None; engines explicitly choose how to handle them.
/// This is not a reservation: the Broker revalidates routes on each invocation.
#[derive(Clone, Debug)]
pub struct EngineRouteModelFacts {
    route: ChatRouteSelection,
    candidates: Vec<EngineRouteCandidateFacts>,
}

impl EngineRouteModelFacts {
    pub fn route(&self) -> &ChatRouteSelection {
        &self.route
    }
    pub fn candidates(&self) -> &[EngineRouteCandidateFacts] {
        &self.candidates
    }

    /// Apply the caller's explicit unknown-limit policy to EVERY candidate,
    /// then intersect. None if any bound remains unknown (or policy is zero).
    /// A known large primary must not mask an unknown/smaller failover.
    pub fn envelope_with_unknown_policy(&self, unknown: EngineModelLimits) -> Option<(u32, u32)> {
        self.candidates
            .iter()
            .try_fold((u32::MAX, u32::MAX), |acc, candidate| {
                let context = candidate.limits.context_tokens.or(unknown.context_tokens)?;
                let output = candidate.limits.output_tokens.or(unknown.output_tokens)?;
                (context > 0 && output > 0).then_some((acc.0.min(context), acc.1.min(output)))
            })
    }
}

fn failure(message: impl std::fmt::Display) -> AppError {
    AppError::Conflict(format!("Engine model facts: {message}"))
}

pub(super) async fn load(
    pool: &SqlitePool,
    session: &AdmittedEngineSession,
) -> Result<EngineRouteModelFacts, AppError> {
    let route = session
        .snapshot()
        .content
        .chat_route_identity
        .as_ref()
        .ok_or_else(|| failure("Session has no exact Chat route"))?;
    let record = session
        .revision()
        .payload
        .chat_route_records
        .get(&route.model_task)
        .ok_or_else(|| failure("exact route has no persisted model facts"))?;
    record.validate_for(route).map_err(failure)?;
    if record.failovers.len() > 32 {
        return Err(failure("route candidate bound exceeded"));
    }
    let mut tx = pool.begin().await.map_err(failure)?;
    let mut candidates = Vec::with_capacity(record.failovers.len() + 1);
    for candidate in std::iter::once(&record.primary).chain(record.failovers.iter()) {
        let row: Option<(Option<i64>, Option<i64>)> = sqlx::query_as(
            "SELECT context_limit, output_limit FROM provider_model_capabilities WHERE provider_id = ? AND model = ? AND task = 'chat'")
            .bind(&candidate.provider_id).bind(&candidate.model)
            .fetch_optional(&mut *tx).await.map_err(failure)?;
        let (context, output) =
            row.ok_or_else(|| failure("selected model capability no longer exists"))?;
        let positive = |value: Option<i64>| -> Result<Option<u32>, AppError> {
            value
                .map(|value| {
                    u32::try_from(value)
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| failure("invalid provider token limit"))
                })
                .transpose()
        };
        candidates.push(EngineRouteCandidateFacts {
            provider_id: candidate.provider_id.clone(),
            model: candidate.model.clone(),
            limits: EngineModelLimits {
                context_tokens: positive(context)?,
                output_tokens: positive(output)?,
            },
        });
    }
    tx.commit().await.map_err(failure)?;
    Ok(EngineRouteModelFacts {
        route: route.clone(),
        candidates,
    })
}
