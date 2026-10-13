//! Provider limits are platform facts, not an engine's context strategy.
//! No credentials, mutable route selection or tokenizer claims are exposed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineModelLimits {
    pub context_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub context_is_input_only: bool,
    pub compaction_threshold_pct: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct EngineRouteCandidateFacts {
    pub limits: EngineModelLimits,
}

/// One consistent database observation of the exact saved primary + failovers.
/// Unknown limits remain None; engines explicitly choose how to handle them.
/// This is not a reservation: the Broker revalidates routes on each invocation.
#[derive(Clone, Debug)]
pub struct EngineRouteModelFacts {
    candidates: Vec<EngineRouteCandidateFacts>,
}

impl EngineRouteModelFacts {
    pub(super) fn from_candidates(candidates: Vec<EngineRouteCandidateFacts>) -> Self {
        Self { candidates }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_defaults_do_not_invent_a_wire_ceiling_and_custom_limits_remain_exact() {
        let mut facts=EngineRouteModelFacts {
            candidates:vec![EngineRouteCandidateFacts {
                limits:EngineModelLimits {context_tokens:Some(1_000_000),output_tokens:None,context_is_input_only:false,compaction_threshold_pct:None}}],
        };
        assert_eq!(facts.configured_output_ceiling(),None);
        facts.candidates[0].limits.output_tokens=Some(100_000);
        assert_eq!(facts.configured_output_ceiling(),Some(100_000));
        facts.candidates.push(EngineRouteCandidateFacts {
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
                limits: EngineModelLimits {
                    context_tokens: Some(128_000),
                    output_tokens: Some(8_000),
                    context_is_input_only: false,
                    compaction_threshold_pct: Some(90),
                },
            },
            EngineRouteCandidateFacts {
                limits: EngineModelLimits {
                    context_tokens: Some(64_000),
                    output_tokens: Some(4_000),
                    context_is_input_only: false,
                    compaction_threshold_pct: Some(60),
                },
            },
        ];
        let facts = EngineRouteModelFacts {
            candidates,
        };
        assert_eq!(facts.compaction_threshold_pct(), 90);

        let mut only_primary = facts.clone();
        only_primary.candidates[1].limits.compaction_threshold_pct = None;
        assert_eq!(only_primary.compaction_threshold_pct(), 90);
    }
}
