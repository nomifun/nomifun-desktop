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
    pub context_is_input_only: bool,
    pub compaction_threshold_pct: Option<u8>,
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
    pub(super) fn from_candidates(route: ChatRouteSelection, candidates: Vec<EngineRouteCandidateFacts>) -> Self {
        Self { route, candidates }
    }
    pub fn route(&self) -> &ChatRouteSelection {
        &self.route
    }
    pub fn candidates(&self) -> &[EngineRouteCandidateFacts] {
        &self.candidates
    }

    /// Plan for the selected primary, not the smallest unused backup. Broker
    /// applies each actual attempt's configuration independently on failover.
    /// None remains unknown; it is not a synthetic 32K/4K provider capability.
    pub fn primary_limits(&self) -> Option<EngineModelLimits> {
        self.candidates.first().map(|candidate| candidate.limits)
    }

    /// Preserve the primary's user-selected strategy. A backup's threshold
    /// cannot prematurely summarize the primary's large-context history.
    pub fn compaction_threshold_pct(&self) -> u8 {
        self.primary_limits()
            .and_then(|limits| limits.compaction_threshold_pct)
            .unwrap_or(75)
    }

    /// Only configured output limits become provider wire ceilings. An
    /// unknown/default candidate must not synthesize a 4096-token cap.
    pub fn configured_output_ceiling(&self)->Option<u32> {
        self.primary_limits().and_then(|limits| limits.output_tokens)
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
        let row: Option<(Option<i64>, Option<i64>, Option<i64>, String)> = sqlx::query_as(
            "SELECT context_limit, output_limit, compaction_threshold_pct, provider_params FROM provider_model_capabilities WHERE provider_id = ? AND model = ? AND task = 'chat'")
            .bind(&candidate.provider_id).bind(&candidate.model)
            .fetch_optional(&mut *tx).await.map_err(failure)?;
        let (context, output, threshold, params) =
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
                context_is_input_only: serde_json::from_str::<serde_json::Value>(&params)
                    .map_err(failure)?.get("_nomifun_context_limit_kind").and_then(serde_json::Value::as_str) == Some("input_only"),
                compaction_threshold_pct: threshold
                    .map(|value| {
                        u8::try_from(value)
                            .ok()
                            .filter(|value| (50..=95).contains(value))
                            .ok_or_else(|| failure("invalid provider compaction threshold"))
                    })
                    .transpose()?,
            },
        });
    }
    tx.commit().await.map_err(failure)?;
    Ok(EngineRouteModelFacts {
        route: route.clone(),
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_agent_contracts::{ChatRouteIdentity, ModelRouteId};

    #[test]
    fn output_defaults_do_not_invent_a_wire_ceiling_and_custom_limits_remain_exact() {
        let mut facts=EngineRouteModelFacts {
            route:ChatRouteIdentity::new("preset","agent.chat",ModelRouteId::from("route"),1),
            candidates:vec![EngineRouteCandidateFacts {provider_id:"primary".into(),model:"provider-default".into(),
                limits:EngineModelLimits {context_tokens:Some(1_000_000),output_tokens:None,context_is_input_only:false,compaction_threshold_pct:None}}],
        };
        assert_eq!(facts.configured_output_ceiling(),None);
        facts.candidates[0].limits.output_tokens=Some(100_000);
        assert_eq!(facts.configured_output_ceiling(),Some(100_000));
        facts.candidates.push(EngineRouteCandidateFacts {provider_id:"backup".into(),model:"default".into(),
            limits:EngineModelLimits {context_tokens:Some(1_000_000),output_tokens:None,context_is_input_only:false,compaction_threshold_pct:None}});
        assert_eq!(facts.configured_output_ceiling(),Some(100_000),"unknown failover must not synthesize 4096");
        facts.candidates[1].limits.output_tokens=Some(32_000);
        assert_eq!(facts.configured_output_ceiling(),Some(100_000), "unused smaller backup cannot cap primary output");
        facts.candidates[1].limits.context_tokens = Some(32_768);
        assert_eq!(facts.primary_limits().unwrap().context_tokens, Some(1_000_000));
        facts.candidates[0].limits.context_tokens = None;
        assert_eq!(facts.primary_limits().unwrap().context_tokens, None, "unknown primary remains provider-defined");
    }

    #[test]
    fn route_preserves_primary_compaction_threshold_despite_smaller_failover() {
        let candidates = vec![
            EngineRouteCandidateFacts {
                provider_id: "primary".into(),
                model: "large".into(),
                limits: EngineModelLimits {
                    context_tokens: Some(128_000),
                    output_tokens: Some(8_000),
                    context_is_input_only: false,
                    compaction_threshold_pct: Some(90),
                },
            },
            EngineRouteCandidateFacts {
                provider_id: "backup".into(),
                model: "small".into(),
                limits: EngineModelLimits {
                    context_tokens: Some(64_000),
                    output_tokens: Some(4_000),
                    context_is_input_only: false,
                    compaction_threshold_pct: Some(60),
                },
            },
        ];
        let facts = EngineRouteModelFacts {
            route: ChatRouteIdentity::new("preset", "agent.chat", ModelRouteId::from("route"), 1),
            candidates,
        };
        assert_eq!(facts.compaction_threshold_pct(), 90);

        let mut only_primary = facts.clone();
        only_primary.candidates[1].limits.compaction_threshold_pct = None;
        assert_eq!(only_primary.compaction_threshold_pct(), 90);
    }
}
