use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    BrowserErrorCode, BrowserLaneId, BrowserPlatformError, Clock, QueueMetadata, QueueRequestId,
};

const DEFAULT_REASON_CODE: &str = "browser_capacity_queued";
const FIRST_LANE_WEIGHT: usize = 4;
const EXPANSION_LANE_WEIGHT: usize = 1;
const PRIORITY_CYCLE_LEN: usize = FIRST_LANE_WEIGHT + EXPANSION_LANE_WEIGHT;

/// Priority class used by the unit-cost weighted deficit round-robin queue.
///
/// Lane admissions all cost one permit, so the 4:1 deficit schedule is
/// equivalent to a repeating four-first-lane, one-expansion-lane schedule.
/// Owner round-robin is applied independently inside both priority classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanePriority {
    First,
    Expansion,
}

impl LanePriority {
    pub const fn weight(self) -> usize {
        match self {
            Self::First => FIRST_LANE_WEIGHT,
            Self::Expansion => EXPANSION_LANE_WEIGHT,
        }
    }
}

/// One request for an open-lane permit.
///
/// `owner_id` is an internal, trusted owner key (normally an owner lease or
/// runtime identity), not a model-provided label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueRequest {
    pub request_id: QueueRequestId,
    pub lane_id: BrowserLaneId,
    pub owner_id: String,
    pub first_lane: bool,
    pub enqueued_at_ms: u64,
    pub reason_code: String,
}

impl QueueRequest {
    pub fn new(
        lane_id: BrowserLaneId,
        owner_id: impl Into<String>,
        first_lane: bool,
        enqueued_at_ms: u64,
    ) -> Self {
        Self {
            request_id: QueueRequestId::new(),
            lane_id,
            owner_id: owner_id.into(),
            first_lane,
            enqueued_at_ms,
            reason_code: DEFAULT_REASON_CODE.to_owned(),
        }
    }

    pub fn pending(
        lane_id: BrowserLaneId,
        owner_id: impl Into<String>,
        first_lane: bool,
    ) -> Self {
        Self::new(lane_id, owner_id, first_lane, 0)
    }

    pub fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = normalize_reason_code(reason_code.into());
        self
    }

    pub const fn priority(&self) -> LanePriority {
        if self.first_lane {
            LanePriority::First
        } else {
            LanePriority::Expansion
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Admission {
    Ready,
    Queued(QueueRequest),
}

impl Admission {
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Dynamic eligibility applied when an already queued Lane is considered for
/// promotion.
///
/// Eligibility is based on the request's original class. An expansion request
/// that has aged into the first-lane scheduling class remains an expansion for
/// resource admission and therefore cannot bypass pressure policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromotionPolicy {
    pub admit_first_lane: bool,
    pub admit_expansion_lane: bool,
    pub first_lane_denied_reason: String,
    pub expansion_lane_denied_reason: String,
}

impl PromotionPolicy {
    pub fn allow_all() -> Self {
        Self {
            admit_first_lane: true,
            admit_expansion_lane: true,
            first_lane_denied_reason: DEFAULT_REASON_CODE.to_owned(),
            expansion_lane_denied_reason: DEFAULT_REASON_CODE.to_owned(),
        }
    }

    pub fn new(
        admit_first_lane: bool,
        admit_expansion_lane: bool,
        first_lane_denied_reason: impl Into<String>,
        expansion_lane_denied_reason: impl Into<String>,
    ) -> Self {
        Self {
            admit_first_lane,
            admit_expansion_lane,
            first_lane_denied_reason: normalize_reason_code(first_lane_denied_reason.into()),
            expansion_lane_denied_reason: normalize_reason_code(
                expansion_lane_denied_reason.into(),
            ),
        }
    }

    fn admits(&self, request: &QueueRequest) -> bool {
        if request.first_lane {
            self.admit_first_lane
        } else {
            self.admit_expansion_lane
        }
    }

    fn denied_reason(&self, request: &QueueRequest) -> &str {
        if request.first_lane {
            &self.first_lane_denied_reason
        } else {
            &self.expansion_lane_denied_reason
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerConfig {
    pub max_open_lanes: usize,
    pub max_global_queue: usize,
    pub max_owner_queue: usize,
    pub recommended_concurrency: usize,
    pub retry_delay_ms: u64,
    /// An expansion request is promoted to the first-lane class after three
    /// aging intervals. This preserves the normal 4:1 share while bounding
    /// starvation when new first-lane work keeps arriving.
    pub aging_interval_ms: u64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            max_open_lanes: 128,
            max_global_queue: 256,
            max_owner_queue: 32,
            recommended_concurrency: 4,
            retry_delay_ms: 1_000,
            aging_interval_ms: 30_000,
        }
    }
}

#[derive(Clone)]
pub struct BrowserLaneScheduler {
    inner: Arc<SchedulerInner>,
}

struct SchedulerInner {
    clock: Arc<dyn Clock>,
    state: Mutex<SchedulerState>,
}

#[derive(Clone)]
struct SchedulerState {
    config: SchedulerConfig,
    active: BTreeMap<BrowserLaneId, QueueRequest>,
    queued: Vec<QueueRequest>,
    promotion_policy: PromotionPolicy,
    owner_ring: VecDeque<String>,
    priority_slot: usize,
    last_first_owner: Option<String>,
    last_expansion_owner: Option<String>,
}

impl BrowserLaneScheduler {
    pub fn new(config: SchedulerConfig, clock: Arc<dyn Clock>) -> Self {
        let mut config = config;
        config.aging_interval_ms = config.aging_interval_ms.max(1);
        Self {
            inner: Arc::new(SchedulerInner {
                clock,
                state: Mutex::new(SchedulerState {
                    config,
                    active: BTreeMap::new(),
                    queued: Vec::new(),
                    promotion_policy: PromotionPolicy::allow_all(),
                    owner_ring: VecDeque::new(),
                    priority_slot: 0,
                    last_first_owner: None,
                    last_expansion_owner: None,
                }),
            }),
        }
    }

    pub fn active_count(&self) -> usize {
        self.state().active.len()
    }

    pub fn queued_count(&self) -> usize {
        self.state().queued.len()
    }

    pub fn admit(
        &self,
        owner_id: impl Into<String>,
        lane_id: BrowserLaneId,
        priority: LanePriority,
        allow_immediate: bool,
        reason_code: impl Into<String>,
    ) -> Result<Admission, BrowserPlatformError> {
        let request = QueueRequest::pending(
            lane_id,
            owner_id,
            matches!(priority, LanePriority::First),
        );
        self.admit_request_with_policy(request, allow_immediate, reason_code)
    }

    /// Admits a lane or places it in the visible queue.
    ///
    /// Resource policy can set `allow_immediate` to false to force a request
    /// into the queue even while the static open-lane limit has room.
    pub fn admit_request_with_policy(
        &self,
        mut request: QueueRequest,
        allow_immediate: bool,
        reason_code: impl Into<String>,
    ) -> Result<Admission, BrowserPlatformError> {
        let now_ms = self.inner.clock.now_ms();
        let reason_code = normalize_reason_code(reason_code.into());
        let mut state = self.state();

        validate_request(&request)?;

        if let Some(existing) = state.active.get(&request.lane_id) {
            ensure_same_owner(existing, &request)?;
            return Ok(Admission::Ready);
        }
        if let Some(existing) = state
            .queued
            .iter()
            .find(|existing| existing.lane_id == request.lane_id)
            .cloned()
        {
            ensure_same_owner(&existing, &request)?;
            return Ok(Admission::Queued(existing));
        }
        if state
            .active
            .values()
            .chain(state.queued.iter())
            .any(|existing| existing.request_id == request.request_id)
        {
            return Err(BrowserPlatformError::new(
                BrowserErrorCode::InvalidCallerIdentity,
                "The browser queue request identifier is already in use.",
                false,
                "Request a fresh browser lane capability.",
            ));
        }

        request.enqueued_at_ms = now_ms;
        request.reason_code = reason_code;

        if allow_immediate
            && state.queued.is_empty()
            && state.active.len() < state.config.max_open_lanes
        {
            state
                .active
                .insert(request.lane_id.clone(), request.clone());
            return Ok(Admission::Ready);
        }

        ensure_queue_capacity(&state, &request)?;
        push_queued(&mut state, request.clone());
        Ok(Admission::Queued(request))
    }

    /// Releases an active permit without promoting queued work.
    ///
    /// The Hub uses this split form so it can take a fresh resource snapshot
    /// after capacity is released and before any queued Lane becomes active.
    pub fn release_without_promotion(&self, lane_id: &BrowserLaneId) -> bool {
        self.state().active.remove(lane_id).is_some()
    }

    /// Promotes at most one resource-eligible request while preserving the
    /// weighted priority and per-owner round-robin state.
    ///
    /// Ineligible requests remain queued and receive the policy's stable reason
    /// code. They do not consume a priority slot or advance an owner cursor.
    pub fn promote_one_with_policy(&self, policy: &PromotionPolicy) -> Option<QueueRequest> {
        let now_ms = self.inner.clock.now_ms();
        let mut state = self.state();
        state.promotion_policy = policy.clone();
        promote_one_locked(&mut state, now_ms, policy)
    }

    pub fn cancel_lane(&self, lane_id: &BrowserLaneId) -> Option<QueueRequest> {
        let mut state = self.state();
        let index = state
            .queued
            .iter()
            .position(|request| &request.lane_id == lane_id)?;
        let removed = state.queued.remove(index);
        prune_owner_ring(&mut state);
        Some(removed)
    }

    /// Applies mutable limits without promoting queued work.
    ///
    /// This is the resource-aware Hub path: it updates queue metadata and the
    /// static ceiling atomically, then performs policy-aware promotions one at
    /// a time with a fresh workload snapshot between each candidate.
    pub fn update_policy_limits_without_promotion(
        &self,
        max_open_lanes: usize,
        max_global_queue: usize,
        max_owner_queue: usize,
        recommended_concurrency: usize,
    ) {
        let mut state = self.state();
        state.config.max_open_lanes = max_open_lanes;
        state.config.max_global_queue = max_global_queue;
        state.config.max_owner_queue = max_owner_queue;
        state.config.recommended_concurrency = recommended_concurrency;
    }

    /// Refreshes the recommendation copied into queue metadata without
    /// changing admission state.
    pub fn update_recommended_concurrency(&self, recommended_concurrency: usize) {
        self.state().config.recommended_concurrency = recommended_concurrency;
    }

    pub fn queue_metadata(&self, request_id: &QueueRequestId) -> Option<QueueMetadata> {
        let now_ms = self.inner.clock.now_ms();
        let state = self.state();
        queue_metadata_locked(
            &state,
            request_id,
            now_ms,
            &state.promotion_policy,
        )
    }

    pub fn metadata(
        &self,
        request_id: &QueueRequestId,
    ) -> Result<QueueMetadata, BrowserPlatformError> {
        self.queue_metadata(request_id).ok_or_else(|| {
            BrowserPlatformError::new(
                BrowserErrorCode::LaneNotFound,
                "The browser queue request no longer exists.",
                false,
                "Refresh the browser inventory.",
            )
        })
    }

    pub fn active_requests(&self) -> Vec<QueueRequest> {
        self.state().active.values().cloned().collect()
    }

    /// Returns queued requests without simulating promotion order.
    ///
    /// Workload accounting only needs membership, class and estimates; the
    /// full queue_order simulation is O(queue^2) under the scheduler mutex
    /// and must not run on every admission/release decision.
    pub fn queued_requests_unordered(&self) -> Vec<QueueRequest> {
        self.state().queued.clone()
    }

    fn state(&self) -> MutexGuard<'_, SchedulerState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn normalize_reason_code(reason_code: String) -> String {
    let reason_code = reason_code.trim();
    if reason_code.is_empty() {
        DEFAULT_REASON_CODE.to_owned()
    } else {
        reason_code.to_owned()
    }
}

fn validate_request(request: &QueueRequest) -> Result<(), BrowserPlatformError> {
    if request.owner_id.trim().is_empty() {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::InvalidCallerIdentity,
            "The browser queue owner is missing.",
            false,
            "Request a fresh browser capability.",
        ));
    }
    Ok(())
}

fn ensure_same_owner(
    existing: &QueueRequest,
    incoming: &QueueRequest,
) -> Result<(), BrowserPlatformError> {
    if existing.owner_id != incoming.owner_id {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::OperationNotAllowed,
            "The browser lane is owned by another runtime.",
            false,
            "Use the lane handle issued to this runtime.",
        )
        .for_lane(incoming.lane_id.clone()));
    }
    Ok(())
}

fn ensure_queue_capacity(
    state: &SchedulerState,
    request: &QueueRequest,
) -> Result<(), BrowserPlatformError> {
    let owner_active = state
        .active
        .values()
        .filter(|active| active.owner_id == request.owner_id)
        .count();
    let owner_queued = state
        .queued
        .iter()
        .filter(|queued| queued.owner_id == request.owner_id)
        .count();

    let limit_reason = if state.queued.len() >= state.config.max_global_queue {
        Some("global_queue_limit")
    } else if owner_queued >= state.config.max_owner_queue {
        Some("owner_queue_limit")
    } else {
        None
    };

    if let Some(limit_reason) = limit_reason {
        return Err(BrowserPlatformError::new(
            BrowserErrorCode::BrowserCapacityQueued,
            "The browser queue has reached its safety limit.",
            true,
            "Reuse an existing lane, reduce concurrency, or retry after capacity is released.",
        )
        .for_lane(request.lane_id.clone())
        .with_metadata(json!({
            "reason_code": limit_reason,
            "recommended_concurrency": state.config.recommended_concurrency,
            "owner_active": owner_active,
            "owner_queued": owner_queued,
            "global_active": state.active.len(),
            "global_queued": state.queued.len(),
            "retry_delay_ms": state.config.retry_delay_ms,
        })));
    }

    Ok(())
}

fn push_queued(state: &mut SchedulerState, request: QueueRequest) {
    if !state
        .owner_ring
        .iter()
        .any(|owner| owner == &request.owner_id)
    {
        state.owner_ring.push_back(request.owner_id.clone());
    }
    state.queued.push(request);
}

fn prune_owner_ring(state: &mut SchedulerState) {
    let previous_ring = state.owner_ring.clone();
    state.owner_ring.retain(|owner| {
        state
            .queued
            .iter()
            .any(|request| &request.owner_id == owner)
    });
    repair_owner_cursor(
        &previous_ring,
        &state.owner_ring,
        &mut state.last_first_owner,
    );
    repair_owner_cursor(
        &previous_ring,
        &state.owner_ring,
        &mut state.last_expansion_owner,
    );
}

/// Preserve the successor of a just-served owner when that owner's final
/// queued request is removed.
///
/// Clearing the cursor outright restarts selection at ring index zero. For
/// `A, B, C` with two requests from A, serving and removing B would then yield
/// `A, B, A, C` rather than the required owner rotation `A, B, C, A`. Pointing
/// at the nearest retained predecessor makes the next scan begin at the exact
/// successor that followed the removed owner in the previous ring.
fn repair_owner_cursor(
    previous_ring: &VecDeque<String>,
    current_ring: &VecDeque<String>,
    cursor: &mut Option<String>,
) {
    let Some(last_owner) = cursor.as_ref() else {
        return;
    };
    if current_ring.contains(last_owner) {
        return;
    }
    let Some(position) = previous_ring.iter().position(|owner| owner == last_owner) else {
        *cursor = None;
        return;
    };
    for step in 1..=previous_ring.len() {
        let predecessor =
            &previous_ring[(position + previous_ring.len() - step) % previous_ring.len()];
        if current_ring.contains(predecessor) {
            *cursor = Some(predecessor.clone());
            return;
        }
    }
    *cursor = None;
}

#[cfg(test)]
fn promote_available_locked(
    state: &mut SchedulerState,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Vec<QueueRequest> {
    let mut promoted = Vec::new();
    while state.active.len() < state.config.max_open_lanes {
        let Some(request) = promote_one_locked(state, now_ms, policy) else {
            break;
        };
        promoted.push(request);
    }
    promoted
}

fn promote_one_locked(
    state: &mut SchedulerState,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Option<QueueRequest> {
    for request in &mut state.queued {
        if policy.admits(request) {
            // Pressure may have cleared while the static Lane ceiling remains
            // full. Refresh the visible reason even when no request can move.
            request.reason_code = DEFAULT_REASON_CODE.to_owned();
        } else {
            request.reason_code = policy.denied_reason(request).to_owned();
        }
    }
    if state.active.len() >= state.config.max_open_lanes {
        return None;
    }
    let index = select_next_index_with_policy(state, now_ms, policy)?;
    let request = state.queued.remove(index);
    state
        .active
        .insert(request.lane_id.clone(), request.clone());
    prune_owner_ring(state);
    Some(request)
}

fn effective_priority(
    request: &QueueRequest,
    now_ms: u64,
    aging_interval_ms: u64,
) -> LanePriority {
    if request.first_lane {
        return LanePriority::First;
    }
    let age_steps = now_ms
        .saturating_sub(request.enqueued_at_ms)
        .checked_div(aging_interval_ms.max(1))
        .unwrap_or_default();
    if age_steps >= (FIRST_LANE_WEIGHT - EXPANSION_LANE_WEIGHT) as u64 {
        LanePriority::First
    } else {
        LanePriority::Expansion
    }
}

fn priority_for_slot(slot: usize) -> LanePriority {
    if slot % PRIORITY_CYCLE_LEN < FIRST_LANE_WEIGHT {
        LanePriority::First
    } else {
        LanePriority::Expansion
    }
}

fn select_next_index_with_policy(
    state: &mut SchedulerState,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Option<usize> {
    if state.queued.is_empty() {
        return None;
    }

    for offset in 0..PRIORITY_CYCLE_LEN {
        let slot = (state.priority_slot + offset) % PRIORITY_CYCLE_LEN;
        let priority = priority_for_slot(slot);
        if let Some(index) = select_owner_fair_index(state, priority, now_ms, policy) {
            state.priority_slot = (slot + 1) % PRIORITY_CYCLE_LEN;
            return Some(index);
        }
    }
    None
}

fn select_owner_fair_index(
    state: &mut SchedulerState,
    priority: LanePriority,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Option<usize> {
    let last_owner = match priority {
        LanePriority::First => state.last_first_owner.as_ref(),
        LanePriority::Expansion => state.last_expansion_owner.as_ref(),
    };
    let start = last_owner
        .and_then(|last| state.owner_ring.iter().position(|owner| owner == last))
        .map_or(0, |position| position + 1);

    for offset in 0..state.owner_ring.len() {
        let owner = &state.owner_ring[(start + offset) % state.owner_ring.len()];
        if let Some(index) = state.queued.iter().position(|request| {
            let effective = effective_priority(request, now_ms, state.config.aging_interval_ms);
            let aged_expansion_fallback = priority == LanePriority::Expansion
                && !request.first_lane
                && effective == LanePriority::First
                && !policy.admit_first_lane
                && policy.admit_expansion_lane;
            &request.owner_id == owner
                && policy.admits(request)
                && (effective == priority || aged_expansion_fallback)
        }) {
            match priority {
                LanePriority::First => state.last_first_owner = Some(owner.clone()),
                LanePriority::Expansion => state.last_expansion_owner = Some(owner.clone()),
            }
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
fn queue_order(
    state: &SchedulerState,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Vec<QueueRequest> {
    let mut simulated = state.clone();
    let mut order = Vec::with_capacity(simulated.queued.len());
    while let Some(index) =
        select_next_index_with_policy(&mut simulated, now_ms, policy)
    {
        order.push(simulated.queued.remove(index));
        prune_owner_ring(&mut simulated);
    }

    // Deferred requests have no promotion order under the current policy.
    // Keep them visible after every eligible request, ordered by the same
    // scheduler state they would use if admission recovered.
    let allow_all = PromotionPolicy::allow_all();
    while !simulated.queued.is_empty() {
        let index =
            select_next_index_with_policy(&mut simulated, now_ms, &allow_all)
                .expect("allow-all policy must select a queued request");
        order.push(simulated.queued.remove(index));
        prune_owner_ring(&mut simulated);
    }
    order
}

fn queue_metadata_locked(
    state: &SchedulerState,
    request_id: &QueueRequestId,
    now_ms: u64,
    policy: &PromotionPolicy,
) -> Option<QueueMetadata> {
    let request = state
        .queued
        .iter()
        .find(|request| &request.request_id == request_id)?;
    // Only this request's position is needed; stop the promotion simulation
    // as soon as it is drained instead of ordering the entire queue while the
    // scheduler mutex is held.
    let mut simulated = state.clone();
    let mut position = 0;
    let mut found = false;
    while let Some(index) = select_next_index_with_policy(&mut simulated, now_ms, policy) {
        position += 1;
        let drained = simulated.queued.remove(index);
        if &drained.request_id == request_id {
            found = true;
            break;
        }
        prune_owner_ring(&mut simulated);
    }
    if !found {
        // Deferred requests order after every promotable request, in the
        // order they would use if admission recovered.
        let allow_all = PromotionPolicy::allow_all();
        while !simulated.queued.is_empty() {
            let index = select_next_index_with_policy(&mut simulated, now_ms, &allow_all)
                .expect("allow-all policy must select a queued request");
            position += 1;
            let drained = simulated.queued.remove(index);
            if &drained.request_id == request_id {
                break;
            }
            prune_owner_ring(&mut simulated);
        }
    }
    let owner_active = state
        .active
        .values()
        .filter(|active| active.owner_id == request.owner_id)
        .count();
    let owner_queued = state
        .queued
        .iter()
        .filter(|queued| queued.owner_id == request.owner_id)
        .count();

    Some(QueueMetadata {
        request_id: request.request_id.clone(),
        position,
        recommended_concurrency: state.config.recommended_concurrency,
        owner_active,
        owner_queued,
        global_active: state.active.len(),
        global_queued: state.queued.len(),
        retry_delay_ms: state.config.retry_delay_ms,
        reason_code: request.reason_code.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ManualClock;

    /// Test seams over the trimmed production API.
    ///
    /// The Hub's actual call set is the only production surface; these
    /// wrappers keep the scheduler-invariant tests expressed in admission /
    /// release / cancellation semantics without re-exposing dead entry points
    /// to production code.
    impl BrowserLaneScheduler {
        fn admit_request(
            &self,
            request: QueueRequest,
        ) -> Result<Admission, BrowserPlatformError> {
            self.admit_request_with_policy(request, true, DEFAULT_REASON_CODE)
        }

        /// Releases an active permit and returns the next promoted request.
        ///
        /// Promotion respects the currently installed [`PromotionPolicy`];
        /// resource-pressure denials survive this entry point instead of
        /// being clobbered by allow-all.
        fn release(&self, lane_id: &BrowserLaneId) -> Option<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let mut state = self.state();
            state.active.remove(lane_id)?;
            let policy = state.promotion_policy.clone();
            promote_one_locked(&mut state, now_ms, &policy)
        }

        /// Cancels a queued request while explicitly rejecting active requests.
        fn cancel(
            &self,
            request_id: &QueueRequestId,
        ) -> Result<Option<QueueRequest>, BrowserPlatformError> {
            let mut state = self.state();
            if let Some(request) = state
                .active
                .values()
                .find(|request| &request.request_id == request_id)
            {
                return Err(BrowserPlatformError::new(
                    BrowserErrorCode::OperationNotAllowed,
                    "The browser queue request has already been promoted and is active.",
                    false,
                    "Close the active browser lane instead of cancelling its queue request.",
                )
                .for_lane(request.lane_id.clone())
                .with_metadata(json!({
                    "request_id": request.request_id.as_str(),
                    "request_state": "active",
                })));
            }
            let Some(index) = state
                .queued
                .iter()
                .position(|request| &request.request_id == request_id)
            else {
                return Ok(None);
            };
            let removed = state.queued.remove(index);
            prune_owner_ring(&mut state);
            Ok(Some(removed))
        }

        /// Removes all of an owner's active and queued requests, then fills
        /// every newly available permit under the installed promotion policy.
        fn release_owner(&self, owner_id: &str) -> Vec<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let mut state = self.state();
            state
                .active
                .retain(|_, request| request.owner_id != owner_id);
            state
                .queued
                .retain(|request| request.owner_id != owner_id);
            prune_owner_ring(&mut state);
            let policy = state.promotion_policy.clone();
            promote_available_locked(&mut state, now_ms, &policy)
        }

        /// Promotes every available request the installed promotion policy admits.
        fn promote_available(&self) -> Vec<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let mut state = self.state();
            let policy = state.promotion_policy.clone();
            promote_available_locked(&mut state, now_ms, &policy)
        }

        /// Updates the dynamic resource-policy limit and promotes under the
        /// installed promotion policy.
        fn update_capacity(
            &self,
            max_open_lanes: usize,
            recommended_concurrency: usize,
        ) -> Vec<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let mut state = self.state();
            state.config.max_open_lanes = max_open_lanes;
            state.config.recommended_concurrency = recommended_concurrency;
            let policy = state.promotion_policy.clone();
            promote_available_locked(&mut state, now_ms, &policy)
        }

        /// Applies all mutable resource-policy limits and promotes under the
        /// installed promotion policy.
        fn update_policy_limits(
            &self,
            max_open_lanes: usize,
            max_global_queue: usize,
            max_owner_queue: usize,
            recommended_concurrency: usize,
        ) -> Vec<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let mut state = self.state();
            state.config.max_open_lanes = max_open_lanes;
            state.config.max_global_queue = max_global_queue;
            state.config.max_owner_queue = max_owner_queue;
            state.config.recommended_concurrency = recommended_concurrency;
            let policy = state.promotion_policy.clone();
            promote_available_locked(&mut state, now_ms, &policy)
        }

        /// Returns queued requests in their current effective promotion order.
        fn queued_requests(&self) -> Vec<QueueRequest> {
            let now_ms = self.inner.clock.now_ms();
            let state = self.state();
            queue_order(&state, now_ms, &state.promotion_policy)
        }
    }

    fn scheduler(config: SchedulerConfig) -> (BrowserLaneScheduler, ManualClock) {
        let clock = ManualClock::new(1_000);
        (
            BrowserLaneScheduler::new(config, Arc::new(clock.clone())),
            clock,
        )
    }

    fn request(owner: &str, name: &str, first_lane: bool) -> QueueRequest {
        QueueRequest::new(
            BrowserLaneId::parse(format!("{owner}-{name}")).unwrap(),
            owner,
            first_lane,
            0,
        )
    }

    fn admit_ready(
        scheduler: &BrowserLaneScheduler,
        request: QueueRequest,
    ) -> BrowserLaneId {
        let lane_id = request.lane_id.clone();
        assert_eq!(
            scheduler.admit_request(request).unwrap(),
            Admission::Ready
        );
        lane_id
    }

    fn queued_request(admission: Admission) -> QueueRequest {
        match admission {
            Admission::Queued(request) => request,
            Admission::Ready => panic!("expected queued admission"),
        }
    }

    #[test]
    fn admits_to_limit_then_returns_complete_queue_metadata() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            recommended_concurrency: 1,
            retry_delay_ms: 750,
            ..SchedulerConfig::default()
        });

        assert!(
            scheduler
                .admit_request(request("owner-a", "active", true))
                .unwrap()
                .is_ready()
        );
        let admission = scheduler
            .admit_request(request("owner-a", "queued", false))
            .unwrap();
        let queued = queued_request(admission);
        let metadata = scheduler.metadata(&queued.request_id).unwrap();
        assert_eq!(metadata.position, 1);
        assert_eq!(metadata.recommended_concurrency, 1);
        assert_eq!(metadata.owner_active, 1);
        assert_eq!(metadata.owner_queued, 1);
        assert_eq!(metadata.global_active, 1);
        assert_eq!(metadata.global_queued, 1);
        assert_eq!(metadata.retry_delay_ms, 750);
        assert_eq!(metadata.reason_code, DEFAULT_REASON_CODE);
    }

    #[test]
    fn rotates_owners_within_the_same_priority() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        scheduler
            .admit_request(request("owner-a", "one", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-a", "two", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-b", "one", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-b", "two", true))
            .unwrap();

        let first = scheduler.release(&active).unwrap();
        let second = scheduler.release(&first.lane_id).unwrap();
        let third = scheduler.release(&second.lane_id).unwrap();
        let fourth = scheduler.release(&third.lane_id).unwrap();

        assert_eq!(
            [
                first.owner_id.as_str(),
                second.owner_id.as_str(),
                third.owner_id.as_str(),
                fourth.owner_id.as_str(),
            ],
            ["owner-a", "owner-b", "owner-a", "owner-b"]
        );
    }

    #[test]
    fn removing_the_last_served_owner_preserves_its_ring_successor() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let mut active = admit_ready(&scheduler, request("seed", "active", true));
        scheduler
            .admit_request(request("owner-a", "one", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-a", "two", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-b", "only", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-c", "only", true))
            .unwrap();

        let mut owners = Vec::new();
        for _ in 0..4 {
            let promoted = scheduler.release(&active).unwrap();
            owners.push(promoted.owner_id.clone());
            active = promoted.lane_id;
        }

        assert_eq!(owners, ["owner-a", "owner-b", "owner-c", "owner-a"]);
    }

    #[test]
    fn applies_four_to_one_first_lane_weight() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let mut active = admit_ready(&scheduler, request("seed", "active", true));
        for index in 0..5 {
            scheduler
                .admit_request(request("first", &index.to_string(), true))
                .unwrap();
        }
        for index in 0..2 {
            scheduler
                .admit_request(request("expansion", &index.to_string(), false))
                .unwrap();
        }

        let mut owners = Vec::new();
        for _ in 0..5 {
            let promoted = scheduler.release(&active).unwrap();
            owners.push(promoted.owner_id.clone());
            active = promoted.lane_id;
        }
        assert_eq!(
            owners,
            ["first", "first", "first", "first", "expansion"]
        );
    }

    #[test]
    fn aging_promotes_an_expansion_ahead_of_new_first_lane_work() {
        let (scheduler, clock) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            aging_interval_ms: 10,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        scheduler
            .admit_request(request("old-expansion", "one", false))
            .unwrap();
        clock.advance(30);
        scheduler
            .admit_request(request("new-first", "one", true))
            .unwrap();

        let promoted = scheduler.release(&active).unwrap();
        assert_eq!(promoted.owner_id, "old-expansion");
    }

    #[test]
    fn aged_expansion_remains_promotable_when_first_lane_admission_is_denied() {
        let (scheduler, clock) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            aging_interval_ms: 10,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let expansion = queued_request(
            scheduler
                .admit_request(request("owner-expansion", "old", false))
                .unwrap(),
        );
        clock.advance(30);

        assert!(scheduler.release_without_promotion(&active));
        let promoted = scheduler
            .promote_one_with_policy(&PromotionPolicy::new(
                false,
                true,
                "system_memory_pressure",
                "browser_resource_pressure",
            ))
            .expect("an aged expansion must not be stranded in the first class");
        assert_eq!(promoted.request_id, expansion.request_id);
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.queued_count(), 0);
    }

    #[test]
    fn cancelling_an_owner_preserves_other_owners_fair_turn() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let cancelled = queued_request(
            scheduler
                .admit_request(request("owner-a", "cancelled", true))
                .unwrap(),
        );
        scheduler
            .admit_request(request("owner-b", "survivor", true))
            .unwrap();
        scheduler
            .admit_request(request("owner-c", "survivor", true))
            .unwrap();

        assert_eq!(
            scheduler
                .cancel(&cancelled.request_id)
                .unwrap()
                .unwrap()
                .request_id,
            cancelled.request_id
        );
        let promoted = scheduler.release(&active).unwrap();
        assert_eq!(promoted.owner_id, "owner-b");
    }

    #[test]
    fn cancellation_is_idempotent_and_updates_positions_immediately() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        scheduler
            .admit_request(request("seed", "active", true))
            .unwrap();
        let first = queued_request(
            scheduler
                .admit_request(request("owner-a", "first", true))
                .unwrap(),
        );
        let second = queued_request(
            scheduler
                .admit_request(request("owner-b", "second", true))
                .unwrap(),
        );
        assert_eq!(
            scheduler
                .queue_metadata(&second.request_id)
                .unwrap()
                .position,
            2
        );

        assert_eq!(
            scheduler
                .cancel(&first.request_id)
                .unwrap()
                .unwrap()
                .request_id,
            first.request_id
        );
        assert_eq!(scheduler.cancel(&first.request_id).unwrap(), None);
        assert_eq!(
            scheduler
                .queue_metadata(&second.request_id)
                .unwrap()
                .position,
            1
        );
    }

    #[test]
    fn cancel_distinguishes_a_promoted_request_from_not_found() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let waiting = queued_request(
            scheduler
                .admit_request(request("owner-a", "waiting", true))
                .unwrap(),
        );

        let promoted = scheduler.release(&active).unwrap();
        assert_eq!(promoted.request_id, waiting.request_id);

        let error = scheduler.cancel(&waiting.request_id).unwrap_err();
        assert_eq!(error.code, BrowserErrorCode::OperationNotAllowed);
        assert_eq!(error.lane_id, Some(waiting.lane_id));
        assert_eq!(error.metadata["request_id"], json!(waiting.request_id));
        assert_eq!(error.metadata["request_state"], json!("active"));

        let unknown = QueueRequestId::parse("unknown-request").unwrap();
        assert_eq!(scheduler.cancel(&unknown).unwrap(), None);
    }

    #[test]
    fn enforces_owner_and_global_queue_caps() {
        let (owner_limited, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            max_owner_queue: 1,
            ..SchedulerConfig::default()
        });
        owner_limited
            .admit_request(request("seed", "active", true))
            .unwrap();
        owner_limited
            .admit_request(request("owner-a", "one", false))
            .unwrap();
        let owner_error = owner_limited
            .admit_request(request("owner-a", "two", false))
            .unwrap_err();
        assert_eq!(
            owner_error.metadata["reason_code"],
            json!("owner_queue_limit")
        );

        let (globally_limited, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            max_global_queue: 1,
            max_owner_queue: 2,
            ..SchedulerConfig::default()
        });
        globally_limited
            .admit_request(request("seed", "active", true))
            .unwrap();
        globally_limited
            .admit_request(request("owner-a", "one", false))
            .unwrap();
        let global_error = globally_limited
            .admit_request(request("owner-b", "one", false))
            .unwrap_err();
        assert_eq!(
            global_error.metadata["reason_code"],
            json!("global_queue_limit")
        );
    }

    #[test]
    fn pressure_can_force_a_visible_queue_with_reason() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 4,
            ..SchedulerConfig::default()
        });
        let admission = scheduler
            .admit_request_with_policy(
                request("owner-a", "memory-wait", true),
                false,
                "system_memory_pressure",
            )
            .unwrap();
        let queued = queued_request(admission);
        let metadata = scheduler.metadata(&queued.request_id).unwrap();
        assert_eq!(scheduler.active_count(), 0);
        assert_eq!(metadata.reason_code, "system_memory_pressure");
    }

    #[test]
    fn promotion_policy_skips_expansion_without_consuming_first_lane_capacity() {
        let (scheduler, clock) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            aging_interval_ms: 10,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let expansion = queued_request(
            scheduler
                .admit_request(request("owner-a", "expansion", false))
                .unwrap(),
        );
        // Aging may elevate scheduling priority, but must never change the
        // request's resource-admission class.
        clock.advance(30);
        let first = queued_request(
            scheduler
                .admit_request(request("owner-b", "first", true))
                .unwrap(),
        );

        assert!(scheduler.release_without_promotion(&active));
        let promoted = scheduler
            .promote_one_with_policy(&PromotionPolicy::new(
                true,
                false,
                "system_memory_pressure",
                "browser_resource_pressure",
            ))
            .unwrap();

        assert_eq!(promoted.request_id, first.request_id);
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.queued_count(), 1);
        assert_eq!(
            scheduler
                .metadata(&expansion.request_id)
                .unwrap()
                .reason_code,
            "browser_resource_pressure"
        );
    }

    #[test]
    fn queue_order_and_metadata_follow_the_current_promotion_policy() {
        let (scheduler, clock) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            aging_interval_ms: 10,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let expansion = queued_request(
            scheduler
                .admit_request(request("owner-a", "expansion", false))
                .unwrap(),
        );
        clock.advance(30);
        let first = queued_request(
            scheduler
                .admit_request(request("owner-b", "first", true))
                .unwrap(),
        );
        let policy = PromotionPolicy::new(
            true,
            false,
            "system_memory_pressure",
            "browser_resource_pressure",
        );

        // A full-capacity promotion pass still installs the current policy for
        // observable queue ordering without moving either request.
        assert!(scheduler.promote_one_with_policy(&policy).is_none());
        assert_eq!(
            scheduler
                .queued_requests()
                .into_iter()
                .map(|request| request.request_id)
                .collect::<Vec<_>>(),
            vec![first.request_id.clone(), expansion.request_id.clone()]
        );
        assert_eq!(
            scheduler.metadata(&first.request_id).unwrap().position,
            1
        );
        assert_eq!(
            scheduler
                .metadata(&expansion.request_id)
                .unwrap()
                .position,
            2
        );

        assert!(scheduler.release_without_promotion(&active));
        let promoted = scheduler.promote_one_with_policy(&policy).unwrap();
        assert_eq!(promoted.request_id, first.request_id);
        assert_eq!(
            scheduler
                .metadata(&expansion.request_id)
                .unwrap()
                .reason_code,
            "browser_resource_pressure"
        );
    }

    #[test]
    fn denied_first_lane_stays_queued_until_budget_recovers_or_is_cancelled() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let waiting = queued_request(
            scheduler
                .admit_request(request("owner-a", "first", true))
                .unwrap(),
        );

        assert!(scheduler.release_without_promotion(&active));
        assert!(
            scheduler
                .promote_one_with_policy(&PromotionPolicy::new(
                    false,
                    false,
                    "system_memory_pressure",
                    "system_memory_pressure",
                ))
                .is_none()
        );
        assert_eq!(scheduler.active_count(), 0);
        assert_eq!(scheduler.queued_count(), 1);
        let metadata = scheduler.metadata(&waiting.request_id).unwrap();
        assert_eq!(metadata.position, 1);
        assert_eq!(metadata.reason_code, "system_memory_pressure");

        let promoted = scheduler
            .promote_one_with_policy(&PromotionPolicy::allow_all())
            .unwrap();
        assert_eq!(promoted.request_id, waiting.request_id);
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.queued_count(), 0);

        let next = queued_request(
            scheduler
                .admit_request(request("owner-b", "cancel-me", true))
                .unwrap(),
        );
        assert_eq!(
            scheduler
                .cancel(&next.request_id)
                .unwrap()
                .unwrap()
                .request_id,
            next.request_id
        );
        assert_eq!(scheduler.queued_count(), 0);
    }

    #[test]
    fn policy_metadata_can_refresh_without_accidental_promotion() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            recommended_concurrency: 4,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let waiting = queued_request(
            scheduler
                .admit_request(request("owner-a", "waiting", true))
                .unwrap(),
        );

        assert!(scheduler.release_without_promotion(&active));
        scheduler.update_policy_limits_without_promotion(2, 128, 16, 1);
        assert_eq!(scheduler.active_count(), 0);
        assert_eq!(scheduler.queued_count(), 1);
        assert_eq!(
            scheduler
                .metadata(&waiting.request_id)
                .unwrap()
                .recommended_concurrency,
            1
        );

        scheduler.update_recommended_concurrency(2);
        assert_eq!(
            scheduler
                .metadata(&waiting.request_id)
                .unwrap()
                .recommended_concurrency,
            2
        );
        assert_eq!(scheduler.active_count(), 0);
    }

    #[test]
    fn cleared_pressure_refreshes_reason_while_static_capacity_is_still_full() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        admit_ready(&scheduler, request("seed", "active", true));
        let waiting = queued_request(
            scheduler
                .admit_request_with_policy(
                    request("owner-a", "waiting", true),
                    false,
                    "system_memory_pressure",
                )
                .unwrap(),
        );
        assert_eq!(
            scheduler
                .metadata(&waiting.request_id)
                .unwrap()
                .reason_code,
            "system_memory_pressure"
        );

        assert!(
            scheduler
                .promote_one_with_policy(&PromotionPolicy::allow_all())
                .is_none()
        );
        assert_eq!(
            scheduler
                .metadata(&waiting.request_id)
                .unwrap()
                .reason_code,
            "browser_capacity_queued"
        );
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.queued_count(), 1);
    }

    #[test]
    fn release_and_capacity_updates_respect_the_installed_promotion_policy() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let expansion = queued_request(
            scheduler
                .admit_request(request("owner-a", "expansion", false))
                .unwrap(),
        );
        // Install a pressure policy that denies expansion promotion.
        let pressure = PromotionPolicy::new(
            true,
            false,
            "system_memory_pressure",
            "browser_resource_pressure",
        );
        assert!(scheduler.promote_one_with_policy(&pressure).is_none());

        // None of the public release/update entry points may clobber the
        // installed policy with allow-all and start the denied expansion lane.
        assert!(
            scheduler.release(&active).is_none(),
            "release must not promote an expansion lane denied by pressure"
        );
        assert!(scheduler.promote_available().is_empty());
        assert!(scheduler.release_owner("seed").is_empty());
        assert!(scheduler.update_capacity(2, 2).is_empty());
        assert!(scheduler.update_policy_limits(2, 128, 16, 2).is_empty());
        assert_eq!(scheduler.active_count(), 0);
        assert_eq!(scheduler.queued_count(), 1);
        assert_eq!(
            scheduler
                .metadata(&expansion.request_id)
                .unwrap()
                .reason_code,
            "browser_resource_pressure",
            "the pressure reason must survive release/update calls"
        );

        // Recovery still promotes once the policy admits expansion again.
        let promoted = scheduler
            .promote_one_with_policy(&PromotionPolicy::allow_all())
            .unwrap();
        assert_eq!(promoted.request_id, expansion.request_id);
    }

    #[test]
    fn release_is_idempotent_and_promotes_exactly_one_request() {
        let (scheduler, _) = scheduler(SchedulerConfig {
            max_open_lanes: 1,
            ..SchedulerConfig::default()
        });
        let active = admit_ready(&scheduler, request("seed", "active", true));
        let queued = queued_request(
            scheduler
                .admit_request(request("owner-a", "queued", true))
                .unwrap(),
        );

        assert_eq!(
            scheduler.release(&active).unwrap().request_id,
            queued.request_id
        );
        assert!(scheduler.release(&active).is_none());
        assert_eq!(scheduler.active_count(), 1);
        assert_eq!(scheduler.queued_count(), 0);
    }
}
