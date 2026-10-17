//! A provider stop ends one response, not the accepted task. Only this reducer
//! decides whether an incomplete response needs review, live waiting or pause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopResolution { Complete, Review, Park, Pause }

pub(crate) struct StopFacts {
    pub needs_account: bool,
    pub account_current: bool,
    pub plan_open: bool,
    pub running_processes: bool,
    pub review_used: bool,
    pub can_review: bool,
}

pub(crate) fn resolve(facts: StopFacts) -> StopResolution {
    let unfinished = facts.running_processes || facts.needs_account && (facts.plan_open || !facts.account_current);
    if !unfinished { return StopResolution::Complete; }
    if !facts.review_used && facts.can_review { return StopResolution::Review; }
    attention(facts.running_processes)
}

/// Exhausted declarations still need attention; they cannot make unfinished
/// business work successful or failed. Original live ownership decides how.
pub(crate) fn attention(running_processes: bool) -> StopResolution {
    if running_processes { StopResolution::Park } else { StopResolution::Pause }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_work_never_becomes_failure_or_success_from_a_provider_stop() {
        for running_processes in [false, true] {
            let mut facts = StopFacts { needs_account: true, account_current: false, plan_open: true,
                running_processes, review_used: false, can_review: true };
            assert_eq!(resolve(facts), StopResolution::Review);
            facts = StopFacts { needs_account: true, account_current: false, plan_open: true,
                running_processes, review_used: true, can_review: true };
            assert_eq!(resolve(facts), if running_processes { StopResolution::Park } else { StopResolution::Pause });
        }
    }
}
