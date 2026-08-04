//! Bounded admission ledger for physical browser cleanup authority.
//!
//! A token is reserved before a physical Lane or Host is started and remains
//! live while that exact resource moves through active, retiring, orphaned,
//! and pending-cleanup states.  It is released only after an exact shutdown
//! proof.  Keeping this accounting independent from those lifecycle
//! containers prevents a failed cleanup from disappearing between them or
//! being counted more than once when a retry republishes the same authority.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;
use std::sync::{Mutex, MutexGuard};

pub(crate) const CLEANUP_BUDGET_GLOBAL_HARD_MAX: usize = 512;
// ResourcePolicy permits up to 32 live Lanes for one task. In isolated mode
// each Lane owns both a Lane and a Host token (64 authorities), so the cleanup
// fence must cover that legal steady state plus one full failed-generation
// handoff. This is a per-task authority bound, not a global RSS ceiling.
pub(crate) const CLEANUP_BUDGET_TASK_HARD_MAX: usize =
    crate::resource::MAX_TASK_OPEN_LANES.saturating_mul(4);
/// The user-visible task family has the same physical-authority envelope as a
/// single logical task. Runtime rotation must not multiply this allowance.
pub(crate) const CLEANUP_BUDGET_FAMILY_HARD_MAX: usize =
    CLEANUP_BUDGET_TASK_HARD_MAX;
pub(crate) const CLEANUP_BUDGET_HOST_HARD_MAX: usize = 256;

const LOW_WATER_NUMERATOR: usize = 3;
const LOW_WATER_DENOMINATOR: usize = 4;

const fn low_water(hard_max: usize) -> usize {
    hard_max.saturating_mul(LOW_WATER_NUMERATOR) / LOW_WATER_DENOMINATOR
}

pub(crate) const CLEANUP_BUDGET_GLOBAL_LOW_WATER: usize =
    low_water(CLEANUP_BUDGET_GLOBAL_HARD_MAX);
pub(crate) const CLEANUP_BUDGET_TASK_LOW_WATER: usize =
    low_water(CLEANUP_BUDGET_TASK_HARD_MAX);
pub(crate) const CLEANUP_BUDGET_FAMILY_LOW_WATER: usize =
    low_water(CLEANUP_BUDGET_FAMILY_HARD_MAX);
pub(crate) const CLEANUP_BUDGET_HOST_LOW_WATER: usize =
    low_water(CLEANUP_BUDGET_HOST_HARD_MAX);

/// The accounting scope which saturated physical admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CleanupBudgetScope {
    Global,
    /// Exact user + runtime cleanup scope.
    Task,
    /// User-visible logical task shared by sibling runtimes.
    Family,
    Host,
}

/// A stable Lane key and a stable Host authority key intentionally have
/// separate types.  The Host authority key may include an epoch while the
/// Host accounting key can remain the logical private `HostKey` from Hub.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CleanupTokenKey<LaneKey, HostAuthorityKey> {
    Lane(LaneKey),
    Host(HostAuthorityKey),
}

/// An opaque exact-authority proof.  The monotonically allocated identity
/// prevents a stale proof from releasing a later reservation which happens to
/// reuse the same external key (the ABA case).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct CleanupBudgetToken<LaneKey, HostAuthorityKey> {
    id: u64,
    key: CleanupTokenKey<LaneKey, HostAuthorityKey>,
}

impl<LaneKey, HostAuthorityKey> CleanupBudgetToken<LaneKey, HostAuthorityKey> {
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    pub(crate) fn key(&self) -> &CleanupTokenKey<LaneKey, HostAuthorityKey> {
        &self.key
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CleanupBudgetSaturation {
    pub(crate) scope: CleanupBudgetScope,
    pub(crate) count: usize,
    pub(crate) requested_units: usize,
    pub(crate) hard_max: usize,
    pub(crate) low_water: usize,
    /// Saturation always installs the sticky fence before this value is
    /// returned.  It is exposed to make that fail-closed contract observable.
    pub(crate) latched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CleanupBudgetError {
    Saturated(CleanupBudgetSaturation),
    /// The same exact physical key was presented with different ownership.
    /// Silently moving it would let two lifecycle paths claim one token.
    AttributionConflict,
    TokenMissing,
    StaleToken,
}

impl CleanupBudgetError {
    pub(crate) fn saturation(&self) -> Option<&CleanupBudgetSaturation> {
        match self {
            Self::Saturated(saturation) => Some(saturation),
            Self::AttributionConflict | Self::TokenMissing | Self::StaleToken => None,
        }
    }
}

impl fmt::Display for CleanupBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Saturated(saturation) => write!(
                formatter,
                "browser cleanup {:?} budget is fenced at {}/{} ({} new unit(s) requested)",
                saturation.scope,
                saturation.count,
                saturation.hard_max,
                saturation.requested_units,
            ),
            Self::AttributionConflict => formatter.write_str(
                "the exact browser cleanup key is already reserved by another attribution",
            ),
            Self::TokenMissing => {
                formatter.write_str("the browser cleanup token is no longer reserved")
            }
            Self::StaleToken => formatter.write_str(
                "the browser cleanup token is stale for the current exact reservation",
            ),
        }
    }
}

impl std::error::Error for CleanupBudgetError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CleanupBudgetScopeSnapshot {
    pub(crate) count: usize,
    pub(crate) hard_max: usize,
    pub(crate) low_water: usize,
    pub(crate) latched: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CleanupBudgetSnapshot<HostScopeKey> {
    pub(crate) global: CleanupBudgetScopeSnapshot,
    pub(crate) tasks: HashMap<String, CleanupBudgetScopeSnapshot>,
    pub(crate) families: HashMap<String, CleanupBudgetScopeSnapshot>,
    pub(crate) hosts: HashMap<HostScopeKey, CleanupBudgetScopeSnapshot>,
    pub(crate) lane_tokens: usize,
    pub(crate) host_tokens: usize,
}

#[derive(Clone, Debug)]
struct CleanupBudgetEntry<HostScopeKey> {
    id: u64,
    /// Exact runtime cleanup attribution.
    task_key: String,
    /// User-visible quota attribution. Never used as teardown authority.
    family_key: String,
    host_key: HostScopeKey,
}

struct CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey> {
    next_id: u64,
    entries: HashMap<
        CleanupTokenKey<LaneKey, HostAuthorityKey>,
        CleanupBudgetEntry<HostScopeKey>,
    >,
    task_counts: HashMap<String, usize>,
    family_counts: HashMap<String, usize>,
    host_counts: HashMap<HostScopeKey, usize>,
    global_latched: bool,
    task_latches: HashSet<String>,
    family_latches: HashSet<String>,
    host_latches: HashSet<HostScopeKey>,
}

impl<LaneKey, HostAuthorityKey, HostScopeKey> Default
    for CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>
{
    fn default() -> Self {
        Self {
            // Zero remains reserved as an invalid/debug sentinel.
            next_id: 1,
            entries: HashMap::new(),
            task_counts: HashMap::new(),
            family_counts: HashMap::new(),
            host_counts: HashMap::new(),
            global_latched: false,
            task_latches: HashSet::new(),
            family_latches: HashSet::new(),
            host_latches: HashSet::new(),
        }
    }
}

/// A mutex-backed transaction ledger. Every admission, release, and scope
/// transfer updates exact tokens plus global, runtime, family, and Host
/// counters and sticky fences under one lock, so callers never observe or
/// create a partial reservation.
pub(crate) struct CleanupBudget<LaneKey, HostAuthorityKey, HostScopeKey> {
    state: Mutex<CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>>,
}

impl<LaneKey, HostAuthorityKey, HostScopeKey> Default
    for CleanupBudget<LaneKey, HostAuthorityKey, HostScopeKey>
{
    fn default() -> Self {
        Self {
            state: Mutex::new(CleanupBudgetState::default()),
        }
    }
}

impl<LaneKey, HostAuthorityKey, HostScopeKey>
    CleanupBudget<LaneKey, HostAuthorityKey, HostScopeKey>
where
    LaneKey: Clone + Eq + Hash,
    HostAuthorityKey: Clone + Eq + Hash,
    HostScopeKey: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reserves one exact Lane authority.  Retrying the same key with the same
    /// attribution returns the original token without consuming another unit.
    pub(crate) fn reserve_lane(
        &self,
        task_key: impl Into<String>,
        host_key: HostScopeKey,
        lane_key: LaneKey,
    ) -> Result<CleanupBudgetToken<LaneKey, HostAuthorityKey>, CleanupBudgetError> {
        let task_key = task_key.into();
        self.reserve_lane_for_family(task_key.clone(), task_key, host_key, lane_key)
    }

    pub(crate) fn reserve_lane_for_family(
        &self,
        task_key: impl Into<String>,
        family_key: impl Into<String>,
        host_key: HostScopeKey,
        lane_key: LaneKey,
    ) -> Result<CleanupBudgetToken<LaneKey, HostAuthorityKey>, CleanupBudgetError> {
        let tokens = self.reserve_exact(
            task_key.into(),
            family_key.into(),
            host_key,
            vec![CleanupTokenKey::Lane(lane_key)],
        )?;
        Ok(tokens
            .into_iter()
            .next()
            .expect("one cleanup reservation must return one token"))
    }

    /// Reserves one exact Host authority.  `host_authority_key` can include an
    /// epoch while `host_key` remains the logical Host accounting scope.
    pub(crate) fn reserve_host(
        &self,
        task_key: impl Into<String>,
        host_key: HostScopeKey,
        host_authority_key: HostAuthorityKey,
    ) -> Result<CleanupBudgetToken<LaneKey, HostAuthorityKey>, CleanupBudgetError> {
        let task_key = task_key.into();
        self.reserve_host_for_family(
            task_key.clone(),
            task_key,
            host_key,
            host_authority_key,
        )
    }

    pub(crate) fn reserve_host_for_family(
        &self,
        task_key: impl Into<String>,
        family_key: impl Into<String>,
        host_key: HostScopeKey,
        host_authority_key: HostAuthorityKey,
    ) -> Result<CleanupBudgetToken<LaneKey, HostAuthorityKey>, CleanupBudgetError> {
        let tokens = self.reserve_exact(
            task_key.into(),
            family_key.into(),
            host_key,
            vec![CleanupTokenKey::Host(host_authority_key)],
        )?;
        Ok(tokens
            .into_iter()
            .next()
            .expect("one cleanup reservation must return one token"))
    }

    /// Atomically reserves the two authorities needed by a cold Host + Lane
    /// start.  If either scope cannot accept both missing units, neither token
    /// is inserted.  Existing exact tokens are treated as idempotent retries.
    pub(crate) fn reserve_lane_and_host(
        &self,
        task_key: impl Into<String>,
        host_key: HostScopeKey,
        lane_key: LaneKey,
        host_authority_key: HostAuthorityKey,
    ) -> Result<
        (
            CleanupBudgetToken<LaneKey, HostAuthorityKey>,
            CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        ),
        CleanupBudgetError,
    > {
        let task_key = task_key.into();
        self.reserve_lane_and_host_for_family(
            task_key.clone(),
            task_key,
            host_key,
            lane_key,
            host_authority_key,
        )
    }

    pub(crate) fn reserve_lane_and_host_for_family(
        &self,
        task_key: impl Into<String>,
        family_key: impl Into<String>,
        host_key: HostScopeKey,
        lane_key: LaneKey,
        host_authority_key: HostAuthorityKey,
    ) -> Result<
        (
            CleanupBudgetToken<LaneKey, HostAuthorityKey>,
            CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        ),
        CleanupBudgetError,
    > {
        let mut tokens = self.reserve_exact(
            task_key.into(),
            family_key.into(),
            host_key,
            vec![
                CleanupTokenKey::Lane(lane_key),
                CleanupTokenKey::Host(host_authority_key),
            ],
        )?;
        debug_assert_eq!(tokens.len(), 2);
        let host = tokens.pop().expect("Host cleanup token must exist");
        let lane = tokens.pop().expect("Lane cleanup token must exist");
        Ok((lane, host))
    }

    /// Moves an existing exact token between task/Host accounting scopes.
    /// The global unit never changes.  This is the authority transfer used by
    /// Lane rebind: the exact Lane key remains stable while its Host changes.
    /// Retrying an already completed transfer is a no-op.
    pub(crate) fn reattribute(
        &self,
        token: &CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        new_task_key: impl Into<String>,
        new_host_key: HostScopeKey,
    ) -> Result<(), CleanupBudgetError> {
        let new_task_key = new_task_key.into();
        self.reattribute_for_family(
            token,
            new_task_key.clone(),
            new_task_key,
            new_host_key,
        )
    }

    pub(crate) fn reattribute_for_family(
        &self,
        token: &CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        new_task_key: impl Into<String>,
        new_family_key: impl Into<String>,
        new_host_key: HostScopeKey,
    ) -> Result<(), CleanupBudgetError> {
        let new_task_key = new_task_key.into();
        let new_family_key = new_family_key.into();
        let mut state = self.state();
        let Some(existing) = state.entries.get(&token.key) else {
            return Err(CleanupBudgetError::TokenMissing);
        };
        if existing.id != token.id {
            return Err(CleanupBudgetError::StaleToken);
        }

        let old_task_key = existing.task_key.clone();
        let old_family_key = existing.family_key.clone();
        let old_host_key = existing.host_key.clone();
        if old_task_key == new_task_key
            && old_family_key == new_family_key
            && old_host_key == new_host_key
        {
            return Ok(());
        }

        if old_task_key != new_task_key {
            refresh_task_latch(&mut state, &new_task_key);
            ensure_task_increase(&mut state, &new_task_key, 1)?;
        }
        if old_family_key != new_family_key {
            refresh_family_latch(&mut state, &new_family_key);
            ensure_family_increase(&mut state, &new_family_key, 1)?;
        }
        if old_host_key != new_host_key {
            refresh_host_latch(&mut state, &new_host_key);
            ensure_host_increase(&mut state, &new_host_key, 1)?;
        }

        if old_task_key != new_task_key {
            decrement_map_count(&mut state.task_counts, &old_task_key, 1);
            refresh_task_latch(&mut state, &old_task_key);
            increment_map_count(&mut state.task_counts, new_task_key.clone(), 1);
            latch_task_at_high_water(&mut state, &new_task_key);
        }
        if old_family_key != new_family_key {
            decrement_map_count(&mut state.family_counts, &old_family_key, 1);
            refresh_family_latch(&mut state, &old_family_key);
            increment_map_count(&mut state.family_counts, new_family_key.clone(), 1);
            latch_family_at_high_water(&mut state, &new_family_key);
        }
        if old_host_key != new_host_key {
            decrement_map_count(&mut state.host_counts, &old_host_key, 1);
            refresh_host_latch(&mut state, &old_host_key);
            increment_map_count(&mut state.host_counts, new_host_key.clone(), 1);
            latch_host_at_high_water(&mut state, &new_host_key);
        }

        let existing = state
            .entries
            .get_mut(&token.key)
            .expect("validated cleanup token must remain under the ledger lock");
        existing.task_key = new_task_key;
        existing.family_key = new_family_key;
        existing.host_key = new_host_key;
        Ok(())
    }

    /// Alias describing the lifecycle meaning of [`Self::reattribute`].
    pub(crate) fn transfer(
        &self,
        token: &CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        new_task_key: impl Into<String>,
        new_host_key: HostScopeKey,
    ) -> Result<(), CleanupBudgetError> {
        self.reattribute(token, new_task_key, new_host_key)
    }

    pub(crate) fn transfer_for_family(
        &self,
        token: &CleanupBudgetToken<LaneKey, HostAuthorityKey>,
        new_task_key: impl Into<String>,
        new_family_key: impl Into<String>,
        new_host_key: HostScopeKey,
    ) -> Result<(), CleanupBudgetError> {
        self.reattribute_for_family(token, new_task_key, new_family_key, new_host_key)
    }

    /// Releases an exact token after cleanup proof.  Missing and stale tokens
    /// are harmless no-ops, making repeated proof publication idempotent.
    pub(crate) fn release(
        &self,
        token: &CleanupBudgetToken<LaneKey, HostAuthorityKey>,
    ) -> bool {
        let mut state = self.state();
        let Some(existing) = state.entries.get(&token.key) else {
            return false;
        };
        if existing.id != token.id {
            return false;
        }
        let existing = state
            .entries
            .remove(&token.key)
            .expect("validated cleanup token must remain under the ledger lock");

        decrement_map_count(&mut state.task_counts, &existing.task_key, 1);
        decrement_map_count(&mut state.family_counts, &existing.family_key, 1);
        decrement_map_count(&mut state.host_counts, &existing.host_key, 1);
        refresh_global_latch(&mut state);
        refresh_task_latch(&mut state, &existing.task_key);
        refresh_family_latch(&mut state, &existing.family_key);
        refresh_host_latch(&mut state, &existing.host_key);
        true
    }

    pub(crate) fn snapshot(&self) -> CleanupBudgetSnapshot<HostScopeKey> {
        let state = self.state();
        let mut lane_tokens = 0;
        let mut host_tokens = 0;
        for key in state.entries.keys() {
            match key {
                CleanupTokenKey::Lane(_) => lane_tokens += 1,
                CleanupTokenKey::Host(_) => host_tokens += 1,
            }
        }

        let tasks = state
            .task_counts
            .iter()
            .map(|(task_key, count)| {
                (
                    task_key.clone(),
                    CleanupBudgetScopeSnapshot {
                        count: *count,
                        hard_max: CLEANUP_BUDGET_TASK_HARD_MAX,
                        low_water: CLEANUP_BUDGET_TASK_LOW_WATER,
                        latched: state.task_latches.contains(task_key),
                    },
                )
            })
            .collect();
        let families = state
            .family_counts
            .iter()
            .map(|(family_key, count)| {
                (
                    family_key.clone(),
                    CleanupBudgetScopeSnapshot {
                        count: *count,
                        hard_max: CLEANUP_BUDGET_FAMILY_HARD_MAX,
                        low_water: CLEANUP_BUDGET_FAMILY_LOW_WATER,
                        latched: state.family_latches.contains(family_key),
                    },
                )
            })
            .collect();
        let hosts = state
            .host_counts
            .iter()
            .map(|(host_key, count)| {
                (
                    host_key.clone(),
                    CleanupBudgetScopeSnapshot {
                        count: *count,
                        hard_max: CLEANUP_BUDGET_HOST_HARD_MAX,
                        low_water: CLEANUP_BUDGET_HOST_LOW_WATER,
                        latched: state.host_latches.contains(host_key),
                    },
                )
            })
            .collect();

        CleanupBudgetSnapshot {
            global: CleanupBudgetScopeSnapshot {
                count: state.entries.len(),
                hard_max: CLEANUP_BUDGET_GLOBAL_HARD_MAX,
                low_water: CLEANUP_BUDGET_GLOBAL_LOW_WATER,
                latched: state.global_latched,
            },
            tasks,
            families,
            hosts,
            lane_tokens,
            host_tokens,
        }
    }

    fn reserve_exact(
        &self,
        task_key: String,
        family_key: String,
        host_key: HostScopeKey,
        requested_keys: Vec<CleanupTokenKey<LaneKey, HostAuthorityKey>>,
    ) -> Result<
        Vec<CleanupBudgetToken<LaneKey, HostAuthorityKey>>,
        CleanupBudgetError,
    > {
        let mut state = self.state();
        let mut missing_keys = Vec::new();

        for key in &requested_keys {
            if let Some(existing) = state.entries.get(key) {
                if existing.task_key != task_key
                    || existing.family_key != family_key
                    || existing.host_key != host_key
                {
                    return Err(CleanupBudgetError::AttributionConflict);
                }
            } else if !missing_keys.contains(key) {
                missing_keys.push(key.clone());
            }
        }

        let requested_units = missing_keys.len();
        if requested_units > 0 {
            refresh_global_latch(&mut state);
            refresh_task_latch(&mut state, &task_key);
            refresh_family_latch(&mut state, &family_key);
            refresh_host_latch(&mut state, &host_key);
            ensure_global_increase(&mut state, requested_units)?;
            ensure_task_increase(&mut state, &task_key, requested_units)?;
            ensure_family_increase(&mut state, &family_key, requested_units)?;
            ensure_host_increase(&mut state, &host_key, requested_units)?;

            for key in missing_keys {
                let id = allocate_token_id(&mut state);
                let replaced = state.entries.insert(
                    key,
                    CleanupBudgetEntry {
                        id,
                        task_key: task_key.clone(),
                        family_key: family_key.clone(),
                        host_key: host_key.clone(),
                    },
                );
                debug_assert!(replaced.is_none());
            }
            increment_map_count(&mut state.task_counts, task_key.clone(), requested_units);
            increment_map_count(
                &mut state.family_counts,
                family_key.clone(),
                requested_units,
            );
            increment_map_count(&mut state.host_counts, host_key.clone(), requested_units);
            latch_global_at_high_water(&mut state);
            latch_task_at_high_water(&mut state, &task_key);
            latch_family_at_high_water(&mut state, &family_key);
            latch_host_at_high_water(&mut state, &host_key);
        }

        Ok(requested_keys
            .into_iter()
            .map(|key| {
                let entry = state
                    .entries
                    .get(&key)
                    .expect("reserved cleanup authority must have a token");
                CleanupBudgetToken { id: entry.id, key }
            })
            .collect())
    }

    fn state(
        &self,
    ) -> MutexGuard<'_, CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn allocate_token_id<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
) -> u64 {
    // At most 512 identities can be live, but avoiding reuse altogether also
    // protects exact proof from ABA across an extremely long-lived process.
    let id = state.next_id;
    state.next_id = state
        .next_id
        .checked_add(1)
        .expect("browser cleanup token identity space exhausted");
    id
}

fn ensure_global_increase<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    requested_units: usize,
) -> Result<(), CleanupBudgetError> {
    let count = state.entries.len();
    if state.global_latched
        || count.saturating_add(requested_units) > CLEANUP_BUDGET_GLOBAL_HARD_MAX
    {
        state.global_latched = true;
        return Err(saturation(
            CleanupBudgetScope::Global,
            count,
            requested_units,
            CLEANUP_BUDGET_GLOBAL_HARD_MAX,
            CLEANUP_BUDGET_GLOBAL_LOW_WATER,
        ));
    }
    Ok(())
}

fn ensure_task_increase<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    task_key: &str,
    requested_units: usize,
) -> Result<(), CleanupBudgetError>
where
    HostScopeKey: Eq + Hash,
{
    let count = state.task_counts.get(task_key).copied().unwrap_or(0);
    if state.task_latches.contains(task_key)
        || count.saturating_add(requested_units) > CLEANUP_BUDGET_TASK_HARD_MAX
    {
        state.task_latches.insert(task_key.to_owned());
        return Err(saturation(
            CleanupBudgetScope::Task,
            count,
            requested_units,
            CLEANUP_BUDGET_TASK_HARD_MAX,
            CLEANUP_BUDGET_TASK_LOW_WATER,
        ));
    }
    Ok(())
}

fn ensure_family_increase<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    family_key: &str,
    requested_units: usize,
) -> Result<(), CleanupBudgetError>
where
    HostScopeKey: Eq + Hash,
{
    let count = state.family_counts.get(family_key).copied().unwrap_or(0);
    if state.family_latches.contains(family_key)
        || count.saturating_add(requested_units) > CLEANUP_BUDGET_FAMILY_HARD_MAX
    {
        state.family_latches.insert(family_key.to_owned());
        return Err(saturation(
            CleanupBudgetScope::Family,
            count,
            requested_units,
            CLEANUP_BUDGET_FAMILY_HARD_MAX,
            CLEANUP_BUDGET_FAMILY_LOW_WATER,
        ));
    }
    Ok(())
}

fn ensure_host_increase<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    host_key: &HostScopeKey,
    requested_units: usize,
) -> Result<(), CleanupBudgetError>
where
    HostScopeKey: Clone + Eq + Hash,
{
    let count = state.host_counts.get(host_key).copied().unwrap_or(0);
    if state.host_latches.contains(host_key)
        || count.saturating_add(requested_units) > CLEANUP_BUDGET_HOST_HARD_MAX
    {
        state.host_latches.insert(host_key.clone());
        return Err(saturation(
            CleanupBudgetScope::Host,
            count,
            requested_units,
            CLEANUP_BUDGET_HOST_HARD_MAX,
            CLEANUP_BUDGET_HOST_LOW_WATER,
        ));
    }
    Ok(())
}

fn saturation(
    scope: CleanupBudgetScope,
    count: usize,
    requested_units: usize,
    hard_max: usize,
    low_water: usize,
) -> CleanupBudgetError {
    CleanupBudgetError::Saturated(CleanupBudgetSaturation {
        scope,
        count,
        requested_units,
        hard_max,
        low_water,
        latched: true,
    })
}

fn latch_global_at_high_water<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
) {
    if state.entries.len() >= CLEANUP_BUDGET_GLOBAL_HARD_MAX {
        state.global_latched = true;
    }
}

fn latch_task_at_high_water<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    task_key: &str,
) {
    if state.task_counts.get(task_key).copied().unwrap_or(0) >= CLEANUP_BUDGET_TASK_HARD_MAX {
        state.task_latches.insert(task_key.to_owned());
    }
}

fn latch_family_at_high_water<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    family_key: &str,
) {
    if state.family_counts.get(family_key).copied().unwrap_or(0)
        >= CLEANUP_BUDGET_FAMILY_HARD_MAX
    {
        state.family_latches.insert(family_key.to_owned());
    }
}

fn latch_host_at_high_water<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    host_key: &HostScopeKey,
) where
    HostScopeKey: Clone + Eq + Hash,
{
    if state.host_counts.get(host_key).copied().unwrap_or(0) >= CLEANUP_BUDGET_HOST_HARD_MAX {
        state.host_latches.insert(host_key.clone());
    }
}

fn refresh_global_latch<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
) {
    if state.global_latched && state.entries.len() <= CLEANUP_BUDGET_GLOBAL_LOW_WATER {
        state.global_latched = false;
    }
}

fn refresh_task_latch<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    task_key: &str,
) where
    HostScopeKey: Eq + Hash,
{
    if state.task_counts.get(task_key).copied().unwrap_or(0) <= CLEANUP_BUDGET_TASK_LOW_WATER {
        state.task_latches.remove(task_key);
    }
}

fn refresh_family_latch<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    family_key: &str,
) where
    HostScopeKey: Eq + Hash,
{
    if state.family_counts.get(family_key).copied().unwrap_or(0)
        <= CLEANUP_BUDGET_FAMILY_LOW_WATER
    {
        state.family_latches.remove(family_key);
    }
}

fn refresh_host_latch<LaneKey, HostAuthorityKey, HostScopeKey>(
    state: &mut CleanupBudgetState<LaneKey, HostAuthorityKey, HostScopeKey>,
    host_key: &HostScopeKey,
) where
    HostScopeKey: Eq + Hash,
{
    if state.host_counts.get(host_key).copied().unwrap_or(0) <= CLEANUP_BUDGET_HOST_LOW_WATER {
        state.host_latches.remove(host_key);
    }
}

fn increment_map_count<Key>(counts: &mut HashMap<Key, usize>, key: Key, amount: usize)
where
    Key: Eq + Hash,
{
    *counts.entry(key).or_insert(0) += amount;
}

fn decrement_map_count<Key>(counts: &mut HashMap<Key, usize>, key: &Key, amount: usize)
where
    Key: Eq + Hash,
{
    let remove = match counts.get_mut(key) {
        Some(count) => {
            debug_assert!(*count >= amount);
            *count = count.saturating_sub(amount);
            *count == 0
        }
        None => false,
    };
    if remove {
        counts.remove(key);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};
    use std::thread;

    use super::*;

    type TestBudget = CleanupBudget<u16, u16, u16>;

    fn saturation_from(error: CleanupBudgetError) -> CleanupBudgetSaturation {
        match error {
            CleanupBudgetError::Saturated(saturation) => saturation,
            other => panic!("expected saturation, got {other:?}"),
        }
    }

    #[test]
    fn each_hard_limit_rejects_before_the_count_can_exceed_it() {
        let task_budget = TestBudget::new();
        for lane in 0..CLEANUP_BUDGET_TASK_HARD_MAX as u16 {
            task_budget.reserve_lane("one-task", lane, lane).unwrap();
        }
        let task_saturation = saturation_from(
            task_budget
                .reserve_lane("one-task", u16::MAX, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(task_saturation.scope, CleanupBudgetScope::Task);
        assert!(task_saturation.latched);
        assert_eq!(task_budget.snapshot().global.count, CLEANUP_BUDGET_TASK_HARD_MAX);

        let host_budget = TestBudget::new();
        for lane in 0..CLEANUP_BUDGET_HOST_HARD_MAX as u16 {
            host_budget
                .reserve_lane(format!("task-{lane}"), 7, lane)
                .unwrap();
        }
        let host_saturation = saturation_from(
            host_budget
                .reserve_lane("next-task", 7, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(host_saturation.scope, CleanupBudgetScope::Host);
        assert!(host_saturation.latched);
        assert_eq!(host_budget.snapshot().global.count, CLEANUP_BUDGET_HOST_HARD_MAX);

        let global_budget = TestBudget::new();
        for lane in 0..CLEANUP_BUDGET_GLOBAL_HARD_MAX as u16 {
            global_budget
                .reserve_lane(format!("task-{lane}"), lane, lane)
                .unwrap();
        }
        let global_saturation = saturation_from(
            global_budget
                .reserve_lane("overflow", u16::MAX, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(global_saturation.scope, CleanupBudgetScope::Global);
        assert!(global_saturation.latched);
        assert_eq!(global_budget.snapshot().global.count, CLEANUP_BUDGET_GLOBAL_HARD_MAX);
    }

    #[test]
    fn sticky_task_fence_clears_only_at_seventy_five_percent_low_water() {
        let budget = TestBudget::new();
        let mut tokens = Vec::new();
        for lane in 0..CLEANUP_BUDGET_TASK_HARD_MAX as u16 {
            tokens.push(budget.reserve_lane("task", lane, lane).unwrap());
        }
        assert!(budget.snapshot().tasks["task"].latched);

        let releases_before_low_water = CLEANUP_BUDGET_TASK_HARD_MAX
            .saturating_sub(CLEANUP_BUDGET_TASK_LOW_WATER)
            .saturating_sub(1);
        for token in tokens.drain(..releases_before_low_water) {
            assert!(budget.release(&token));
        }
        assert_eq!(
            budget.snapshot().tasks["task"].count,
            CLEANUP_BUDGET_TASK_LOW_WATER + 1
        );
        assert!(budget.snapshot().tasks["task"].latched);
        let saturation = saturation_from(
            budget
                .reserve_lane("task", u16::MAX, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Task);

        assert!(budget.release(&tokens.remove(0)));
        let at_low_water = budget.snapshot();
        assert_eq!(at_low_water.tasks["task"].count, CLEANUP_BUDGET_TASK_LOW_WATER);
        assert!(!at_low_water.tasks["task"].latched);
        budget.reserve_lane("task", u16::MAX, u16::MAX).unwrap();
        assert_eq!(
            budget.snapshot().tasks["task"].count,
            CLEANUP_BUDGET_TASK_LOW_WATER + 1
        );
    }

    #[test]
    fn saturated_task_is_fail_closed_without_blocking_another_task() {
        let budget = TestBudget::new();
        for lane in 0..CLEANUP_BUDGET_TASK_HARD_MAX as u16 {
            budget.reserve_lane("leaking-task", lane, lane).unwrap();
        }
        let saturation = saturation_from(
            budget
                .reserve_lane("leaking-task", 1000, 1000)
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Task);
        assert_eq!(saturation.hard_max, CLEANUP_BUDGET_TASK_HARD_MAX);

        budget
            .reserve_lane("healthy-task", 1001, 1001)
            .expect("a task-local cleanup fence must not become a global cap");
        let snapshot = budget.snapshot();
        assert_eq!(
            snapshot.tasks["leaking-task"].count,
            CLEANUP_BUDGET_TASK_HARD_MAX
        );
        assert!(snapshot.tasks["leaking-task"].latched);
        assert_eq!(snapshot.tasks["healthy-task"].count, 1);
        assert!(!snapshot.global.latched);
    }

    #[test]
    fn sibling_runtimes_share_one_atomic_family_fence() {
        let budget = TestBudget::new();
        let mut runtime_a = Vec::new();
        let mut runtime_b = Vec::new();
        for lane in 0..CLEANUP_BUDGET_FAMILY_HARD_MAX as u16 {
            let token = budget
                .reserve_lane_for_family(
                    if lane % 2 == 0 { "runtime-a" } else { "runtime-b" },
                    "conversation-family",
                    lane,
                    lane,
                )
                .unwrap();
            if lane % 2 == 0 {
                runtime_a.push(token);
            } else {
                runtime_b.push(token);
            }
        }
        let full = budget.snapshot();
        assert_eq!(full.tasks["runtime-a"].count, runtime_a.len());
        assert_eq!(full.tasks["runtime-b"].count, runtime_b.len());
        assert_eq!(
            full.families["conversation-family"].count,
            CLEANUP_BUDGET_FAMILY_HARD_MAX
        );
        assert!(full.families["conversation-family"].latched);

        let before = budget.snapshot();
        let saturation = saturation_from(
            budget
                .reserve_lane_for_family(
                    "runtime-c",
                    "conversation-family",
                    u16::MAX,
                    u16::MAX,
                )
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Family);
        let after = budget.snapshot();
        assert_eq!(after.global.count, before.global.count);
        assert!(!after.tasks.contains_key("runtime-c"));
        assert_eq!(
            after.families["conversation-family"].count,
            before.families["conversation-family"].count,
            "a later family-scope failure must leave earlier counters untouched"
        );

        let released_a = runtime_a.remove(0);
        assert!(budget.release(&released_a));
        let released = budget.snapshot();
        assert_eq!(released.tasks["runtime-a"].count, runtime_a.len());
        assert_eq!(released.tasks["runtime-b"].count, runtime_b.len());
        assert_eq!(
            released.families["conversation-family"].count,
            CLEANUP_BUDGET_FAMILY_HARD_MAX - 1
        );
    }

    #[test]
    fn stale_proof_and_failed_reattribution_preserve_both_attributions() {
        let budget = TestBudget::new();
        let stale = budget
            .reserve_lane_for_family("runtime-old", "family-old", 1, 1)
            .unwrap();
        assert!(budget.release(&stale));
        let current = budget
            .reserve_lane_for_family("runtime-new", "family-new", 1, 1)
            .unwrap();
        assert!(!budget.release(&stale));
        let current_snapshot = budget.snapshot();
        assert_eq!(current_snapshot.tasks["runtime-new"].count, 1);
        assert_eq!(current_snapshot.families["family-new"].count, 1);

        let source = budget
            .reserve_lane_for_family("runtime-source", "family-source", 2, 2)
            .unwrap();
        let mut destination = Vec::new();
        for lane in 10..(10 + CLEANUP_BUDGET_FAMILY_HARD_MAX as u16) {
            destination.push(
                budget
                    .reserve_lane_for_family(
                        format!("runtime-destination-{}", lane % 2),
                        "family-destination",
                        lane,
                        lane,
                    )
                    .unwrap(),
            );
        }
        let saturation = saturation_from(
            budget
                .reattribute_for_family(
                    &source,
                    "runtime-destination-0",
                    "family-destination",
                    999,
                )
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Family);
        let after = budget.snapshot();
        assert_eq!(after.tasks["runtime-source"].count, 1);
        assert_eq!(after.families["family-source"].count, 1);
        assert_eq!(
            after.families["family-destination"].count,
            CLEANUP_BUDGET_FAMILY_HARD_MAX
        );
        assert_eq!(after.hosts[&2].count, 1);
        assert!(!after.hosts.contains_key(&999));
        assert_eq!(destination.len(), CLEANUP_BUDGET_FAMILY_HARD_MAX);
        assert!(budget.release(&current));
    }

    #[test]
    fn retries_do_not_add_units_and_conflicting_attribution_fails_closed() {
        let budget = TestBudget::new();
        let lane = budget.reserve_lane("task", 3, 9).unwrap();
        let lane_retry = budget.reserve_lane("task", 3, 9).unwrap();
        assert_eq!(lane, lane_retry);
        assert_eq!(budget.snapshot().global.count, 1);

        assert_eq!(
            budget.reserve_lane("other-task", 3, 9),
            Err(CleanupBudgetError::AttributionConflict)
        );
        assert_eq!(
            budget.reserve_lane("task", 4, 9),
            Err(CleanupBudgetError::AttributionConflict)
        );
        assert_eq!(budget.snapshot().global.count, 1);

        let host = budget.reserve_host("task", 3, 11).unwrap();
        let host_retry = budget.reserve_host("task", 3, 11).unwrap();
        assert_eq!(host, host_retry);
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.global.count, 2);
        assert_eq!(snapshot.lane_tokens, 1);
        assert_eq!(snapshot.host_tokens, 1);
    }

    #[test]
    fn release_is_idempotent_and_stale_proof_cannot_release_reused_key() {
        let budget = TestBudget::new();
        let old = budget.reserve_lane("task", 1, 4).unwrap();
        assert!(budget.release(&old));
        assert!(!budget.release(&old));

        let current = budget.reserve_lane("task", 1, 4).unwrap();
        assert_ne!(old.id(), current.id());
        assert!(!budget.release(&old));
        assert_eq!(budget.snapshot().global.count, 1);
        assert!(budget.release(&current));
        assert_eq!(budget.snapshot().global.count, 0);
    }

    #[test]
    fn reattribution_is_atomic_idempotent_and_does_not_add_a_global_unit() {
        let budget = TestBudget::new();
        let token = budget.reserve_lane("old-task", 1, 4).unwrap();
        budget.reattribute(&token, "new-task", 2).unwrap();
        budget.reattribute(&token, "new-task", 2).unwrap();
        budget.transfer(&token, "new-task", 2).unwrap();

        let snapshot = budget.snapshot();
        assert_eq!(snapshot.global.count, 1);
        assert!(!snapshot.tasks.contains_key("old-task"));
        assert_eq!(snapshot.tasks["new-task"].count, 1);
        assert!(!snapshot.hosts.contains_key(&1));
        assert_eq!(snapshot.hosts[&2].count, 1);
    }

    #[test]
    fn failed_reattribution_keeps_the_old_attribution_intact() {
        let budget = TestBudget::new();
        let source = budget.reserve_lane("source", 500, 500).unwrap();
        let mut destination_tokens = Vec::new();
        for lane in 0..CLEANUP_BUDGET_TASK_HARD_MAX as u16 {
            destination_tokens.push(
                budget
                    .reserve_lane("destination", lane, lane)
                    .unwrap(),
            );
        }

        let saturation = saturation_from(
            budget
                .reattribute(&source, "destination", 600)
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Task);
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.global.count, CLEANUP_BUDGET_TASK_HARD_MAX + 1);
        assert_eq!(snapshot.tasks["source"].count, 1);
        assert_eq!(snapshot.tasks["destination"].count, CLEANUP_BUDGET_TASK_HARD_MAX);
        assert_eq!(snapshot.hosts[&500].count, 1);
        assert!(!snapshot.hosts.contains_key(&600));

        // Keep the authorities live for the entire assertion; dropping a
        // token is deliberately not a release operation.
        assert_eq!(destination_tokens.len(), CLEANUP_BUDGET_TASK_HARD_MAX);
    }

    #[test]
    fn lane_and_host_reservation_is_one_transaction() {
        let budget = TestBudget::new();
        let mut tokens = Vec::new();
        for lane in 0..(CLEANUP_BUDGET_TASK_HARD_MAX - 1) as u16 {
            tokens.push(budget.reserve_lane("task", lane, lane).unwrap());
        }
        let before = budget.snapshot();
        assert_eq!(before.global.count, CLEANUP_BUDGET_TASK_HARD_MAX - 1);

        let saturation = saturation_from(
            budget
                .reserve_lane_and_host("task", u16::MAX, u16::MAX, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Task);
        assert_eq!(saturation.requested_units, 2);
        let after = budget.snapshot();
        assert_eq!(after.global.count, before.global.count);
        assert_eq!(after.lane_tokens, before.lane_tokens);
        assert_eq!(after.host_tokens, 0);
        assert_eq!(tokens.len(), CLEANUP_BUDGET_TASK_HARD_MAX - 1);
    }

    #[test]
    fn sixty_four_concurrent_reservations_stop_exactly_at_global_boundary() {
        let budget = Arc::new(TestBudget::new());
        for lane in 0..448_u16 {
            budget
                .reserve_lane(format!("prefill-{lane}"), lane, lane)
                .unwrap();
        }

        let barrier = Arc::new(Barrier::new(65));
        let mut workers = Vec::new();
        for offset in 0..64_u16 {
            let budget = Arc::clone(&budget);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                let key = 448 + offset;
                budget.reserve_lane(format!("worker-{offset}"), key, key)
            }));
        }
        barrier.wait();

        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        let snapshot = budget.snapshot();
        assert_eq!(snapshot.global.count, CLEANUP_BUDGET_GLOBAL_HARD_MAX);
        assert!(snapshot.global.latched);
        let saturation = saturation_from(
            budget
                .reserve_lane("overflow", u16::MAX, u16::MAX)
                .unwrap_err(),
        );
        assert_eq!(saturation.scope, CleanupBudgetScope::Global);
        assert_eq!(budget.snapshot().global.count, CLEANUP_BUDGET_GLOBAL_HARD_MAX);
    }
}
