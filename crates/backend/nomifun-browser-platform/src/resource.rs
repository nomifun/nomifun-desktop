use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ResourcePressureState;

pub const MIB: u64 = 1024 * 1024;
pub const GIB: u64 = 1024 * MIB;
pub const MAX_ACTIVE_OPERATIONS: usize = 64;
pub const MAX_OPEN_LANES: usize = 128;
pub const MAX_GLOBAL_QUEUE: usize = 256;
pub const MAX_OWNER_QUEUE: usize = 32;
pub const MIN_BROWSER_MEMORY_RATIO: f64 = 0.1;
pub const MAX_BROWSER_MEMORY_RATIO: f64 = 0.8;
pub const MIN_TASK_MEMORY_BYTES: u64 = 256 * MIB;
pub const MAX_TASK_MEMORY_BYTES: u64 = 16 * GIB;
// A managed Chromium Host costs several hundred MiB before it renders anything:
// the browser process, the GPU process, network/storage utilities and the crash
// handler all exist independently of page content. A task that is alone on a
// Host is attributed that entire baseline, so the budget has to cover the
// baseline *plus* real pages. A 1 GiB budget did not: an idle nine-process
// Chromium tree already measured ~700 MiB, leaving no room for content and
// getting ordinary sessions reclaimed within seconds.
pub const AUTOMATIC_TASK_MEMORY_BYTES: u64 = 2 * GIB;
pub const RESOURCE_SAVING_TASK_MEMORY_BYTES: u64 = 5 * GIB / 4;
pub const HIGH_CONCURRENCY_TASK_MEMORY_BYTES: u64 = 2 * GIB;
pub const MAX_TASK_ACTIVE_OPERATIONS: usize = 16;
pub const MAX_TASK_OPEN_LANES: usize = 32;
pub const MAX_TASK_TABS: usize = 64;
pub const MAX_DERIVED_RESERVED_MEMORY_BYTES: u64 = 8 * GIB;
pub const MIN_RESERVED_MEMORY_BYTES: u64 = 256 * MIB;
pub const MAX_RESERVED_MEMORY_BYTES: u64 = 512 * GIB;

const fn default_task_memory_bytes() -> u64 {
    AUTOMATIC_TASK_MEMORY_BYTES
}

const fn default_task_active_operations() -> usize {
    2
}

const fn default_task_open_lanes() -> usize {
    4
}

const fn default_task_tabs() -> usize {
    16
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("invalid browser resource policy field `{field}`: {reason}")]
pub struct ResourcePolicyValidationError {
    pub field: &'static str,
    pub reason: &'static str,
}

impl ResourcePolicyValidationError {
    fn new(field: &'static str, reason: &'static str) -> Self {
        Self { field, reason }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourcePolicyPreset {
    Automatic,
    ResourceSaving,
    HighConcurrency,
    Custom,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResourcePolicy {
    pub preset: ResourcePolicyPreset,
    pub reserved_memory_bytes: u64,
    pub max_browser_memory_ratio: f64,
    /// Estimated memory share allowed to one trusted user-visible task family.
    ///
    /// Chromium Hosts may be shared by many tasks, so this is deliberately
    /// separate from the machine-wide ratio. The Hub attributes Host RSS to
    /// live Lanes and reclaims only the task that persistently exceeds this
    /// budget; aggregate browser memory remains elastic for many tasks.
    #[serde(default = "default_task_memory_bytes")]
    pub max_task_memory_bytes: u64,
    pub lane_cold_start_bytes: u64,
    pub lane_ewma_min_bytes: u64,
    pub lane_ewma_max_bytes: u64,
    pub max_active_operations: usize,
    pub max_open_lanes: usize,
    /// Weighted in-flight driver-operation limit for one task family. Sibling
    /// runtimes in the same conversation share it.
    #[serde(default = "default_task_active_operations")]
    pub max_task_active_operations: usize,
    /// Live Lane limit for one task family. Runtime rotation cannot create
    /// another allowance. This is not a physical renderer-process limit:
    /// cross-origin iframes/OOPIFs may fan one page out across renderers.
    #[serde(default = "default_task_open_lanes")]
    pub max_task_open_lanes: usize,
    /// Top-level page/target limit across all Lanes in one task family. It is a
    /// structural quota, not a renderer-process ceiling; physical process
    /// isolation requires a dedicated Host or OS-level containment.
    #[serde(default = "default_task_tabs")]
    pub max_task_tabs: usize,
    pub max_global_queue: usize,
    pub max_owner_queue: usize,
    pub sample_period_ms: u64,
    pub lifecycle_sweep_period_ms: u64,
    pub idle_expiry_ms: u64,
    pub pressured_idle_expiry_ms: u64,
    pub host_warm_ms: u64,
}

impl ResourcePolicy {
    pub fn automatic(total_memory_bytes: u64, logical_cpus: usize) -> Self {
        let reserved_memory_bytes = (total_memory_bytes / 5)
            .clamp(2 * GIB, MAX_DERIVED_RESERVED_MEMORY_BYTES);
        let browser_budget = ((total_memory_bytes as f64) * 0.4) as u64;
        let memory_permits = browser_budget
            .saturating_sub(reserved_memory_bytes.min(browser_budget))
            / (256 * MIB);
        let active = usize::try_from(memory_permits)
            .unwrap_or(MAX_ACTIVE_OPERATIONS)
            .min(logical_cpus.saturating_mul(2))
            .clamp(1, MAX_ACTIVE_OPERATIONS);
        Self {
            preset: ResourcePolicyPreset::Automatic,
            reserved_memory_bytes,
            max_browser_memory_ratio: 0.4,
            max_task_memory_bytes: AUTOMATIC_TASK_MEMORY_BYTES,
            lane_cold_start_bytes: 256 * MIB,
            lane_ewma_min_bytes: 128 * MIB,
            // This predictor must be able to cross the per-task watchdog
            // threshold; clamping it at the threshold hides a leaking Lane.
            lane_ewma_max_bytes: 4 * GIB,
            max_active_operations: active,
            max_open_lanes: active.saturating_mul(4).min(MAX_OPEN_LANES),
            max_task_active_operations: default_task_active_operations(),
            max_task_open_lanes: default_task_open_lanes(),
            max_task_tabs: default_task_tabs(),
            max_global_queue: MAX_GLOBAL_QUEUE,
            max_owner_queue: MAX_OWNER_QUEUE,
            sample_period_ms: 5_000,
            lifecycle_sweep_period_ms: 30_000,
            idle_expiry_ms: 2 * 60_000,
            pressured_idle_expiry_ms: 30_000,
            host_warm_ms: 0,
        }
    }

    pub fn preset(
        preset: ResourcePolicyPreset,
        total_memory_bytes: u64,
        logical_cpus: usize,
    ) -> Self {
        let mut policy = Self::automatic(total_memory_bytes, logical_cpus);
        policy.preset = preset;
        match preset {
            ResourcePolicyPreset::Automatic | ResourcePolicyPreset::Custom => {}
            ResourcePolicyPreset::ResourceSaving => {
                policy.max_active_operations = (policy.max_active_operations / 2).max(1);
                policy.max_open_lanes = policy.max_active_operations.saturating_mul(3).min(96);
                policy.max_browser_memory_ratio = 0.3;
                policy.max_task_memory_bytes = RESOURCE_SAVING_TASK_MEMORY_BYTES;
                policy.max_task_active_operations = 1;
                policy.max_task_open_lanes = 2;
                policy.max_task_tabs = 8;
                policy.idle_expiry_ms = 60_000;
                policy.pressured_idle_expiry_ms = 15_000;
                policy.host_warm_ms = 0;
            }
            ResourcePolicyPreset::HighConcurrency => {
                policy.max_active_operations =
                    policy.max_active_operations.saturating_mul(2).min(MAX_ACTIVE_OPERATIONS);
                policy.max_open_lanes = policy
                    .max_active_operations
                    .saturating_mul(4)
                    .min(MAX_OPEN_LANES);
                policy.max_browser_memory_ratio = 0.5;
                // High concurrency raises aggregate capacity, not the share
                // one task may monopolize.
                policy.max_task_memory_bytes = HIGH_CONCURRENCY_TASK_MEMORY_BYTES;
                policy.max_task_active_operations = 2;
                policy.max_task_open_lanes = 4;
                policy.max_task_tabs = 16;
                policy.idle_expiry_ms = 10 * 60_000;
                policy.pressured_idle_expiry_ms = 2 * 60_000;
                policy.host_warm_ms = 60_000;
            }
        }
        policy
    }

    /// Validates the process-wide safety boundary for every resource-policy
    /// entry point.
    ///
    /// HTTP validation is useful for field-specific user feedback, but it
    /// cannot be the authority because startup restore and in-process callers
    /// can construct [`ResourcePolicy`] directly. The Hub must call this
    /// method before applying a policy.
    pub fn validate(&self) -> Result<(), ResourcePolicyValidationError> {
        if !self.max_browser_memory_ratio.is_finite()
            || !(MIN_BROWSER_MEMORY_RATIO..=MAX_BROWSER_MEMORY_RATIO)
                .contains(&self.max_browser_memory_ratio)
        {
            return Err(ResourcePolicyValidationError::new(
                "max_browser_memory_ratio",
                "must be finite and between 0.1 and 0.8",
            ));
        }
        if !(MIN_TASK_MEMORY_BYTES..=MAX_TASK_MEMORY_BYTES)
            .contains(&self.max_task_memory_bytes)
        {
            return Err(ResourcePolicyValidationError::new(
                "max_task_memory_bytes",
                "is outside the supported range",
            ));
        }
        if !(MIN_RESERVED_MEMORY_BYTES..=MAX_RESERVED_MEMORY_BYTES)
            .contains(&self.reserved_memory_bytes)
        {
            return Err(ResourcePolicyValidationError::new(
                "reserved_memory_bytes",
                "is outside the supported range",
            ));
        }
        if self.lane_cold_start_bytes == 0 {
            return Err(ResourcePolicyValidationError::new(
                "lane_cold_start_bytes",
                "must be greater than zero",
            ));
        }
        if self.lane_ewma_min_bytes == 0 {
            return Err(ResourcePolicyValidationError::new(
                "lane_ewma_min_bytes",
                "must be greater than zero",
            ));
        }
        if self.lane_ewma_max_bytes == 0 {
            return Err(ResourcePolicyValidationError::new(
                "lane_ewma_max_bytes",
                "must be greater than zero",
            ));
        }
        if self.lane_ewma_min_bytes > self.lane_ewma_max_bytes {
            return Err(ResourcePolicyValidationError::new(
                "lane_ewma_min_bytes",
                "cannot exceed lane_ewma_max_bytes",
            ));
        }
        if !(1..=MAX_ACTIVE_OPERATIONS).contains(&self.max_active_operations) {
            return Err(ResourcePolicyValidationError::new(
                "max_active_operations",
                "must be between 1 and 64",
            ));
        }
        if !(1..=MAX_OPEN_LANES).contains(&self.max_open_lanes) {
            return Err(ResourcePolicyValidationError::new(
                "max_open_lanes",
                "must be between 1 and 128",
            ));
        }
        if !(1..=MAX_TASK_ACTIVE_OPERATIONS).contains(&self.max_task_active_operations) {
            return Err(ResourcePolicyValidationError::new(
                "max_task_active_operations",
                "must be between 1 and 16",
            ));
        }
        if !(1..=MAX_TASK_OPEN_LANES).contains(&self.max_task_open_lanes) {
            return Err(ResourcePolicyValidationError::new(
                "max_task_open_lanes",
                "must be between 1 and 32",
            ));
        }
        if !(1..=MAX_TASK_TABS).contains(&self.max_task_tabs) {
            return Err(ResourcePolicyValidationError::new(
                "max_task_tabs",
                "must be between 1 and 64",
            ));
        }
        if !(1..=MAX_GLOBAL_QUEUE).contains(&self.max_global_queue) {
            return Err(ResourcePolicyValidationError::new(
                "max_global_queue",
                "must be between 1 and 256",
            ));
        }
        if !(1..=MAX_OWNER_QUEUE).contains(&self.max_owner_queue) {
            return Err(ResourcePolicyValidationError::new(
                "max_owner_queue",
                "must be between 1 and 32",
            ));
        }
        if self.max_owner_queue > self.max_global_queue {
            return Err(ResourcePolicyValidationError::new(
                "max_owner_queue",
                "cannot exceed max_global_queue",
            ));
        }
        for (field, value) in [
            ("sample_period_ms", self.sample_period_ms),
            (
                "lifecycle_sweep_period_ms",
                self.lifecycle_sweep_period_ms,
            ),
            ("idle_expiry_ms", self.idle_expiry_ms),
            (
                "pressured_idle_expiry_ms",
                self.pressured_idle_expiry_ms,
            ),
        ] {
            if value == 0 {
                return Err(ResourcePolicyValidationError::new(
                    field,
                    "must be greater than zero",
                ));
            }
        }
        if self.pressured_idle_expiry_ms > self.idle_expiry_ms {
            return Err(ResourcePolicyValidationError::new(
                "pressured_idle_expiry_ms",
                "cannot exceed idle_expiry_ms",
            ));
        }
        Ok(())
    }
}

impl Default for ResourcePolicy {
    fn default() -> Self {
        Self::automatic(8 * GIB, 4)
    }
}

pub(crate) fn next_lane_resource_ewma(
    current_bytes: u64,
    sample_bytes: u64,
    min_bytes: u64,
    max_bytes: u64,
) -> u64 {
    let min_bytes = min_bytes.min(max_bytes);
    let current = current_bytes.clamp(min_bytes, max_bytes);
    let sample = sample_bytes.clamp(min_bytes, max_bytes);
    current
        .saturating_mul(3)
        .saturating_add(sample)
        .saturating_div(4)
        .clamp(min_bytes, max_bytes)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResourceTelemetry {
    pub total_memory_bytes: u64,
    pub available_memory_bytes: u64,
    pub chromium_rss_bytes: u64,
    pub logical_cpus: usize,
    pub cpu_pressure: f64,
    /// GPU pressure in the inclusive range 0.0-1.0 when a real collector is
    /// available. `None` means unknown; it must not be presented or reasoned
    /// about as a measured zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_pressure: Option<f64>,
    /// Per-managed-host RSS keyed by its OS root process ID.
    ///
    /// This is strictly an in-process join index for `BrowserSessionHub`.
    /// Process IDs must never cross an HTTP/WebSocket boundary; management
    /// projections expose only the matched byte count on safe host records.
    #[serde(skip)]
    pub host_rss_by_process_id: HashMap<u32, u64>,
    /// Per-managed-host CPU load keyed by its OS root process ID.
    ///
    /// Values are normalized against total logical machine capacity: `1.0`
    /// means the process tree consumed all logical CPUs during the sampling
    /// interval. This is another in-process join index only. A shared Host can
    /// serve several tasks, so the value must never be presented as precise
    /// task CPU attribution or serialized across a management boundary.
    #[serde(skip)]
    pub host_cpu_pressure_by_process_id: HashMap<u32, f64>,
}

/// Point-in-time browser workload used alongside OS telemetry for admission
/// and concurrency recommendations.
///
/// Byte fields are aggregates of the per-Lane bounded EWMA stored in inventory,
/// not another copy of process RSS. Keeping both lets policy use process RSS as
/// the measured hard limit and Lane EWMA as the cost predictor for queued work.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResourceWorkload {
    /// Scheduler-active Lanes, including frozen Lanes that still own capacity.
    pub active_lanes: usize,
    pub queued_lanes: usize,
    pub queued_first_lanes: usize,
    pub frozen_lanes: usize,
    pub primary_lanes: usize,
    pub active_operation_permits: usize,
    /// Weighted Agent screenshot/PDF permits.
    pub active_heavy_operation_permits: usize,
    pub active_lane_ewma_bytes: u64,
    pub queued_lane_estimate_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceDecision {
    pub state: ResourcePressureState,
    pub admit_first_lane: bool,
    pub admit_expansion_lane: bool,
    /// Shared weighted budget for newly dispatched driver operations.
    ///
    /// Ordinary operations cost one unit and screenshot/PDF work costs two.
    /// Unlike `recommended_concurrency`, this is a total live hard limit and
    /// therefore does not subtract operations which already hold permits.
    pub operation_weight_limit: usize,
    pub recommended_concurrency: usize,
    /// The memory reserve this decision actually enforced. For the `Custom`
    /// preset this is the validated user-configured value; derived presets may
    /// raise it to the machine-size floor. Surfacing it lets management
    /// responses show the value in force instead of silently diverging from
    /// configuration.
    pub effective_reserved_memory_bytes: u64,
    /// Machine-wide Chromium RSS pressure threshold derived from total memory.
    ///
    /// This is an elastic admission/shedding signal, not the per-task budget
    /// and not an absolute installation-wide allocation cap.
    pub effective_browser_memory_limit_bytes: u64,
    pub reason_code: Option<&'static str>,
    pub first_lane_reason_code: Option<&'static str>,
    pub expansion_lane_reason_code: Option<&'static str>,
}

impl ResourcePolicy {
    pub fn decide(&self, telemetry: &ResourceTelemetry) -> ResourceDecision {
        self.decide_with_workload(telemetry, &ResourceWorkload::default())
    }

    pub fn decide_with_workload(
        &self,
        telemetry: &ResourceTelemetry,
        workload: &ResourceWorkload,
    ) -> ResourceDecision {
        // Sampling starts asynchronously after application startup.  An
        // all-zero sample means "not collected yet", not "zero bytes free";
        // static lane and operation caps remain the safety boundary until the
        // first real sample arrives.
        let initial_sample_missing = telemetry.total_memory_bytes == 0
            && telemetry.available_memory_bytes == 0
            && telemetry.chromium_rss_bytes == 0
            && telemetry.logical_cpus == 0
            && telemetry.cpu_pressure == 0.0
            && telemetry.gpu_pressure.is_none()
            && telemetry.host_rss_by_process_id.is_empty();
        if initial_sample_missing {
            let static_recommendation =
                available_operation_concurrency(self.max_active_operations, workload);
            return ResourceDecision {
                state: ResourcePressureState::Normal,
                admit_first_lane: true,
                admit_expansion_lane: true,
                operation_weight_limit: self.max_active_operations.max(1),
                recommended_concurrency: static_recommendation,
                effective_reserved_memory_bytes: self.reserved_memory_bytes,
                effective_browser_memory_limit_bytes: 0,
                reason_code: None,
                first_lane_reason_code: None,
                expansion_lane_reason_code: None,
            };
        }
        let total = telemetry.total_memory_bytes.max(1);
        let logical_cpu_limit = if telemetry.logical_cpus == 0 {
            self.max_active_operations
        } else {
            telemetry.logical_cpus.saturating_mul(2).min(64)
        };
        let operation_limit = self
            .max_active_operations
            .min(logical_cpu_limit)
            .max(1);
        let static_recommendation = available_operation_concurrency(operation_limit, workload);
        // The 20%-of-total floor protects presets whose reserve was derived
        // for a different machine size, but it is capped on large workstations
        // so a large installation can still scale across many independent
        // tasks. An explicit `Custom` reserve remains authoritative.
        let reserved = if self.preset == ResourcePolicyPreset::Custom {
            self.reserved_memory_bytes
        } else {
            self.reserved_memory_bytes
                .max(((total as f64) * 0.2) as u64)
                .min(MAX_DERIVED_RESERVED_MEMORY_BYTES)
        };
        let browser_limit = ((total as f64) * self.max_browser_memory_ratio) as u64;
        let predicted_lane_bytes = predicted_lane_cost(self, workload);
        let system_headroom = telemetry.available_memory_bytes.saturating_sub(reserved);
        // Preserve one global basic-availability Lane while the machine is
        // pressured but not critical. Requiring the full reserve before the
        // very first Lane can start makes Browser Use completely unavailable
        // on large-memory machines that still have several GiB free (for
        // example, 64 GiB total / 8.7 GiB available). Expansion Lanes continue
        // to require the full reserve, and once any Lane owns capacity the
        // emergency allowance is closed until telemetry recovers.
        let critical_system_reserve = reserved / 2;
        let first_lane_system_headroom = if workload.active_lanes == 0 {
            telemetry
                .available_memory_bytes
                .saturating_sub(critical_system_reserve)
        } else {
            system_headroom
        };
        let browser_headroom = browser_limit.saturating_sub(telemetry.chromium_rss_bytes);
        let first_lane_admission_headroom =
            first_lane_system_headroom.min(browser_headroom);
        let expansion_lane_admission_headroom = system_headroom.min(browser_headroom);
        // Heavy operations can allocate between samples. Keep a bounded
        // transient reserve so a Lane promotion does not spend their entire
        // safety margin using stale RSS.
        let transient_reserve = predicted_lane_bytes
            .saturating_mul(
                u64::try_from(workload.active_heavy_operation_permits).unwrap_or(u64::MAX),
            )
            .saturating_div(2);
        let first_lane_budget = predicted_lane_bytes.saturating_add(transient_reserve);
        // Expansion may not consume the budget needed by the oldest waiting
        // first Lane. An active Primary identity also keeps a small interactive
        // reserve in accordance with the basic-availability-first policy.
        let expansion_reserve = predicted_lane_bytes
            .saturating_mul(
                u64::try_from(workload.queued_first_lanes.min(1)).unwrap_or_default(),
            )
            .saturating_add(
                if workload.primary_lanes > 0 {
                    predicted_lane_bytes / 4
                } else {
                    0
                },
            );
        let expansion_lane_budget = first_lane_budget.saturating_add(expansion_reserve);

        let mut critical = telemetry.available_memory_bytes < reserved / 2
            || telemetry.chromium_rss_bytes > browser_limit;
        let operation_load = workload
            .active_operation_permits
            .saturating_add(
                workload
                    .active_heavy_operation_permits
                    .saturating_mul(2),
            );
        let permit_saturated = operation_load >= operation_limit;
        let gpu_pressured = telemetry
            .gpu_pressure
            .filter(|pressure| pressure.is_finite())
            .is_some_and(|pressure| pressure >= 0.9);
        let first_budget_available = first_lane_admission_headroom >= first_lane_budget;
        let expansion_budget_available =
            expansion_lane_admission_headroom >= expansion_lane_budget;
        let mut pressured = !critical
            && (telemetry.available_memory_bytes < reserved
                || telemetry.chromium_rss_bytes > browser_limit.saturating_mul(85) / 100
                || telemetry.cpu_pressure >= 0.9
                || gpu_pressured
                || permit_saturated
                || !first_budget_available
                || !expansion_budget_available);
        // A zero browser budget is critical even if a custom ratio was
        // malformed into zero; this keeps first-Lane admission fail closed.
        if browser_limit == 0 {
            critical = true;
            pressured = false;
        }
        let state = if critical {
            ResourcePressureState::Critical
        } else if pressured {
            ResourcePressureState::Pressured
        } else {
            ResourcePressureState::Normal
        };

        let safe_total_browser_bytes = browser_limit.min(
            telemetry
                .chromium_rss_bytes
                .saturating_add(system_headroom),
        );
        let lane_capacity = usize::try_from(safe_total_browser_bytes / predicted_lane_bytes.max(1))
            .unwrap_or(usize::MAX)
            .saturating_sub(workload.frozen_lanes);
        let pressure_cap = match state {
            ResourcePressureState::Normal => operation_limit,
            ResourcePressureState::Pressured => (operation_limit / 2).max(1),
            ResourcePressureState::Critical => 1,
        };
        let operation_weight_limit = pressure_cap.min(lane_capacity.max(1)).max(1);
        let recommended_concurrency = pressure_cap
            .min(lane_capacity.max(1))
            .min(static_recommendation)
            .max(1);
        let admit_first_lane = !critical && first_budget_available;
        let admit_expansion_lane =
            state == ResourcePressureState::Normal && expansion_budget_available;
        let first_lane_reason_code =
            (!admit_first_lane).then_some("system_memory_pressure");
        let expansion_lane_reason_code = if admit_expansion_lane {
            None
        } else if !admit_first_lane || state == ResourcePressureState::Critical {
            Some("system_memory_pressure")
        } else {
            Some("browser_resource_pressure")
        };
        ResourceDecision {
            state,
            admit_first_lane,
            admit_expansion_lane,
            operation_weight_limit,
            recommended_concurrency,
            effective_reserved_memory_bytes: reserved,
            effective_browser_memory_limit_bytes: browser_limit,
            reason_code: first_lane_reason_code.or(expansion_lane_reason_code),
            first_lane_reason_code,
            expansion_lane_reason_code,
        }
    }
}

fn available_operation_concurrency(
    operation_limit: usize,
    workload: &ResourceWorkload,
) -> usize {
    operation_limit
        .saturating_sub(workload.active_operation_permits)
        .saturating_sub(
            workload
                .active_heavy_operation_permits
                .saturating_mul(2),
        )
        .saturating_sub(usize::from(workload.primary_lanes > 0))
        .max(1)
}

fn predicted_lane_cost(policy: &ResourcePolicy, workload: &ResourceWorkload) -> u64 {
    let estimated_lanes = workload
        .active_lanes
        .saturating_add(workload.queued_lanes);
    let aggregate_estimate = workload
        .active_lane_ewma_bytes
        .saturating_add(workload.queued_lane_estimate_bytes);
    let measured_average = if estimated_lanes == 0 || aggregate_estimate == 0 {
        policy.lane_cold_start_bytes
    } else {
        aggregate_estimate
            .saturating_add(u64::try_from(estimated_lanes - 1).unwrap_or(u64::MAX))
            / u64::try_from(estimated_lanes).unwrap_or(u64::MAX).max(1)
    };
    measured_average
        .max(policy.lane_cold_start_bytes)
        .clamp(policy.lane_ewma_min_bytes.min(policy.lane_ewma_max_bytes), policy.lane_ewma_max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_matches_required_bounds() {
        let policy = ResourcePolicy::automatic(16 * GIB, 8);
        assert_eq!(policy.validate(), Ok(()));
        assert_eq!(policy.reserved_memory_bytes, 16 * GIB / 5);
        assert_eq!(policy.max_task_memory_bytes, AUTOMATIC_TASK_MEMORY_BYTES);
        assert!(policy.max_active_operations <= MAX_ACTIVE_OPERATIONS);
        assert!(policy.max_open_lanes <= MAX_OPEN_LANES);
        assert_eq!(policy.max_task_active_operations, 2);
        assert_eq!(policy.max_task_open_lanes, 4);
        assert_eq!(policy.max_task_tabs, 16);
        assert_eq!(policy.idle_expiry_ms, 2 * 60_000);
        assert_eq!(policy.pressured_idle_expiry_ms, 30_000);
        assert_eq!(policy.host_warm_ms, 0);
        assert_eq!(policy.max_global_queue, MAX_GLOBAL_QUEUE);
        assert_eq!(policy.max_owner_queue, MAX_OWNER_QUEUE);

        let workstation = ResourcePolicy::automatic(64 * GIB, 32);
        assert_eq!(
            workstation.reserved_memory_bytes,
            MAX_DERIVED_RESERVED_MEMORY_BYTES
        );
        assert_eq!(
            workstation
                .decide(&ResourceTelemetry {
                    total_memory_bytes: 64 * GIB,
                    available_memory_bytes: 12 * GIB,
                    logical_cpus: 32,
                    ..Default::default()
                })
                .effective_reserved_memory_bytes,
            MAX_DERIVED_RESERVED_MEMORY_BYTES
        );
    }

    #[test]
    fn legacy_serialized_policy_gets_default_task_budget() {
        let policy = ResourcePolicy::automatic(16 * GIB, 8);
        let mut value = serde_json::to_value(&policy).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("max_task_memory_bytes");

        let restored: ResourcePolicy = serde_json::from_value(value).unwrap();
        assert_eq!(
            restored.max_task_memory_bytes,
            AUTOMATIC_TASK_MEMORY_BYTES
        );
    }

    #[test]
    fn validation_rejects_non_finite_ratio_and_task_limit_bypasses() {
        let mut policy = ResourcePolicy::default();
        policy.max_browser_memory_ratio = f64::NAN;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "max_browser_memory_ratio"
        );

        let mut policy = ResourcePolicy::default();
        policy.max_task_memory_bytes = MIN_TASK_MEMORY_BYTES - 1;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "max_task_memory_bytes"
        );

        let mut policy = ResourcePolicy::default();
        policy.max_active_operations = MAX_ACTIVE_OPERATIONS + 1;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "max_active_operations"
        );

        let mut policy = ResourcePolicy::default();
        policy.max_open_lanes = MAX_OPEN_LANES + 1;
        assert_eq!(policy.validate().unwrap_err().field, "max_open_lanes");

        let mut policy = ResourcePolicy::default();
        policy.max_global_queue = MAX_GLOBAL_QUEUE + 1;
        assert_eq!(policy.validate().unwrap_err().field, "max_global_queue");

        let mut policy = ResourcePolicy::default();
        policy.max_owner_queue = MAX_OWNER_QUEUE + 1;
        assert_eq!(policy.validate().unwrap_err().field, "max_owner_queue");
    }

    #[test]
    fn validation_rejects_zero_costs_periods_and_incoherent_ranges() {
        let mut policy = ResourcePolicy::default();
        policy.lane_cold_start_bytes = 0;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "lane_cold_start_bytes"
        );

        let mut policy = ResourcePolicy::default();
        policy.lane_ewma_min_bytes = policy.lane_ewma_max_bytes + 1;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "lane_ewma_min_bytes"
        );

        let mut policy = ResourcePolicy::default();
        policy.sample_period_ms = 0;
        assert_eq!(policy.validate().unwrap_err().field, "sample_period_ms");

        let mut policy = ResourcePolicy::default();
        policy.pressured_idle_expiry_ms = policy.idle_expiry_ms + 1;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "pressured_idle_expiry_ms"
        );

        let mut policy = ResourcePolicy::default();
        policy.max_global_queue = 4;
        policy.max_owner_queue = 5;
        assert_eq!(policy.validate().unwrap_err().field, "max_owner_queue");
    }

    #[test]
    fn custom_preset_honors_explicit_reserve_below_the_automatic_floor() {
        // 64 GiB machine, user explicitly reserves 2 GiB (validated range).
        let mut policy = ResourcePolicy::automatic(64 * GIB, 16);
        policy.preset = ResourcePolicyPreset::Custom;
        policy.reserved_memory_bytes = 2 * GIB;
        assert_eq!(policy.validate(), Ok(()));
        // 7 GiB available is below the capped 8 GiB automatic floor but far above
        // the explicit 2 GiB reserve; the machine must not be Pressured.
        let telemetry = ResourceTelemetry {
            total_memory_bytes: 64 * GIB,
            available_memory_bytes: 7 * GIB,
            logical_cpus: 16,
            ..Default::default()
        };

        let decision = policy.decide(&telemetry);
        assert_eq!(decision.state, ResourcePressureState::Normal);
        assert!(decision.admit_first_lane);
        assert!(decision.admit_expansion_lane);
        assert_eq!(decision.effective_reserved_memory_bytes, 2 * GIB);

        // The same values under the Automatic preset keep the safety floor
        // and surface the raised effective reserve.
        let automatic = ResourcePolicy::automatic(64 * GIB, 16);
        let floored = automatic.decide(&telemetry);
        assert_eq!(floored.state, ResourcePressureState::Pressured);
        assert_eq!(
            floored.effective_reserved_memory_bytes,
            automatic
                .reserved_memory_bytes
                .max((64 * GIB) / 5)
                .min(MAX_DERIVED_RESERVED_MEMORY_BYTES)
        );
    }

    #[test]
    fn pressure_stops_expansion_before_first_lane() {
        let policy = ResourcePolicy::automatic(8 * GIB, 8);
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: 6 * GIB,
            chromium_rss_bytes: ((8 * GIB) as f64 * policy.max_browser_memory_ratio)
                as u64
                * 86
                / 100,
            logical_cpus: 8,
            ..Default::default()
        });
        assert_eq!(decision.state, ResourcePressureState::Pressured);
        assert!(decision.admit_first_lane);
        assert!(!decision.admit_expansion_lane);
    }

    #[test]
    fn first_lane_below_critical_floor_has_system_memory_pressure_reason() {
        let policy = ResourcePolicy::automatic(8 * GIB, 8);
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: policy
                .reserved_memory_bytes
                .saturating_div(2)
                .saturating_add(policy.lane_cold_start_bytes)
                .saturating_sub(1),
            logical_cpus: 8,
            ..Default::default()
        });

        assert_eq!(decision.state, ResourcePressureState::Pressured);
        assert!(!decision.admit_first_lane);
        assert!(!decision.admit_expansion_lane);
        assert_eq!(
            decision.first_lane_reason_code,
            Some("system_memory_pressure")
        );
        assert_eq!(
            decision.expansion_lane_reason_code,
            Some("system_memory_pressure")
        );
    }

    #[test]
    fn pressured_machine_admits_exactly_one_basic_availability_lane() {
        let policy = ResourcePolicy::automatic(64 * GIB, 16);
        let telemetry = ResourceTelemetry {
            total_memory_bytes: 64 * GIB,
            available_memory_bytes: 7 * GIB,
            logical_cpus: 16,
            ..Default::default()
        };

        let first = policy.decide(&telemetry);
        assert_eq!(first.state, ResourcePressureState::Pressured);
        assert!(first.admit_first_lane);
        assert!(!first.admit_expansion_lane);
        assert_eq!(first.recommended_concurrency, 1);

        let after_basic_lane = policy.decide_with_workload(
            &telemetry,
            &ResourceWorkload {
                active_lanes: 1,
                primary_lanes: 1,
                active_lane_ewma_bytes: policy.lane_cold_start_bytes,
                ..Default::default()
            },
        );
        assert_eq!(after_basic_lane.state, ResourcePressureState::Pressured);
        assert!(!after_basic_lane.admit_first_lane);
        assert!(!after_basic_lane.admit_expansion_lane);
        assert_eq!(
            after_basic_lane.first_lane_reason_code,
            Some("system_memory_pressure")
        );
    }

    #[test]
    fn measured_cpu_pressure_reduces_concurrency_with_healthy_memory() {
        let policy = ResourcePolicy::preset(
            ResourcePolicyPreset::HighConcurrency,
            16 * GIB,
            8,
        );
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 16 * GIB,
            available_memory_bytes: 12 * GIB,
            logical_cpus: 8,
            cpu_pressure: 0.95,
            ..Default::default()
        });

        assert_eq!(decision.state, ResourcePressureState::Pressured);
        assert!(decision.admit_first_lane);
        assert!(!decision.admit_expansion_lane);
        assert_eq!(
            decision.recommended_concurrency,
            (policy.max_active_operations.min(8 * 2) / 2).max(1)
        );
    }

    #[test]
    fn missing_initial_sample_uses_static_caps_instead_of_permanent_pressure() {
        let policy = ResourcePolicy::automatic(8 * GIB, 4);
        let decision = policy.decide(&ResourceTelemetry::default());
        assert_eq!(decision.state, ResourcePressureState::Normal);
        assert!(decision.admit_first_lane);
        assert!(decision.admit_expansion_lane);
    }

    #[test]
    fn partial_sample_without_total_memory_fails_closed() {
        let policy = ResourcePolicy::automatic(8 * GIB, 4);
        let decision = policy.decide(&ResourceTelemetry {
            chromium_rss_bytes: MIB,
            ..Default::default()
        });

        assert_eq!(decision.state, ResourcePressureState::Critical);
        assert!(!decision.admit_first_lane);
        assert!(!decision.admit_expansion_lane);
        assert_eq!(
            decision.first_lane_reason_code,
            Some("system_memory_pressure")
        );
    }

    #[test]
    fn workload_cost_and_live_counts_reduce_recommended_concurrency() {
        let mut policy = ResourcePolicy::preset(
            ResourcePolicyPreset::HighConcurrency,
            16 * GIB,
            8,
        );
        policy.lane_ewma_max_bytes = 4 * GIB;
        let telemetry = ResourceTelemetry {
            total_memory_bytes: 16 * GIB,
            available_memory_bytes: 12 * GIB,
            chromium_rss_bytes: GIB,
            logical_cpus: 8,
            ..Default::default()
        };
        let baseline = policy
            .decide_with_workload(&telemetry, &ResourceWorkload::default())
            .recommended_concurrency;

        let expensive = policy
            .decide_with_workload(
                &telemetry,
                &ResourceWorkload {
                    active_lanes: 4,
                    queued_lanes: 2,
                    active_lane_ewma_bytes: 8 * GIB,
                    queued_lane_estimate_bytes: 4 * GIB,
                    ..Default::default()
                },
            )
            .recommended_concurrency;
        let frozen = policy
            .decide_with_workload(
                &telemetry,
                &ResourceWorkload {
                    active_lanes: 4,
                    queued_lanes: 2,
                    frozen_lanes: 2,
                    active_lane_ewma_bytes: 8 * GIB,
                    queued_lane_estimate_bytes: 4 * GIB,
                    ..Default::default()
                },
            )
            .recommended_concurrency;

        assert!(expensive < baseline);
        assert!(frozen < expensive);
    }

    #[test]
    fn permits_and_primary_reserve_real_recommended_capacity() {
        let policy = ResourcePolicy::automatic(32 * GIB, 16);
        let telemetry = ResourceTelemetry {
            total_memory_bytes: 32 * GIB,
            available_memory_bytes: 28 * GIB,
            chromium_rss_bytes: GIB,
            logical_cpus: 16,
            ..Default::default()
        };
        let baseline = policy
            .decide_with_workload(&telemetry, &ResourceWorkload::default())
            .recommended_concurrency;
        let loaded = policy
            .decide_with_workload(
                &telemetry,
                &ResourceWorkload {
                    active_lanes: 3,
                    primary_lanes: 2,
                    active_operation_permits: 2,
                    active_heavy_operation_permits: 1,
                    active_lane_ewma_bytes: 3 * policy.lane_cold_start_bytes,
                    ..Default::default()
                },
            )
            .recommended_concurrency;

        assert_eq!(loaded, baseline.saturating_sub(5).max(1));
    }

    #[test]
    fn measured_logical_cpu_count_caps_configured_operation_concurrency() {
        let mut policy = ResourcePolicy::preset(
            ResourcePolicyPreset::HighConcurrency,
            64 * GIB,
            64,
        );
        policy.max_active_operations = 64;
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 64 * GIB,
            available_memory_bytes: 56 * GIB,
            logical_cpus: 4,
            ..Default::default()
        });

        assert_eq!(decision.state, ResourcePressureState::Normal);
        assert_eq!(decision.recommended_concurrency, 8);
    }

    #[test]
    fn unknown_gpu_pressure_is_distinct_from_measured_zero() {
        let policy = ResourcePolicy::automatic(16 * GIB, 8);
        let base = ResourceTelemetry {
            total_memory_bytes: 16 * GIB,
            available_memory_bytes: 12 * GIB,
            logical_cpus: 8,
            gpu_pressure: None,
            ..Default::default()
        };
        assert_eq!(policy.decide(&base).state, ResourcePressureState::Normal);

        let measured_zero = ResourceTelemetry {
            gpu_pressure: Some(0.0),
            ..base.clone()
        };
        assert_eq!(
            policy.decide(&measured_zero).state,
            ResourcePressureState::Normal
        );
        let measured_pressure = ResourceTelemetry {
            gpu_pressure: Some(0.95),
            ..base
        };
        assert_eq!(
            policy.decide(&measured_pressure).state,
            ResourcePressureState::Pressured
        );

        let unknown_json = serde_json::to_value(ResourceTelemetry::default()).unwrap();
        assert!(unknown_json.get("gpu_pressure").is_none());
        let zero_json = serde_json::to_value(measured_zero).unwrap();
        assert_eq!(zero_json["gpu_pressure"], 0.0);
    }

    #[test]
    fn process_id_join_indexes_are_never_serialized() {
        let telemetry = ResourceTelemetry {
            chromium_rss_bytes: 123,
            host_rss_by_process_id: HashMap::from([(4_242, 123)]),
            host_cpu_pressure_by_process_id: HashMap::from([(4_242, 0.75)]),
            ..Default::default()
        };

        let json = serde_json::to_value(telemetry).unwrap();
        assert_eq!(json["chromium_rss_bytes"], 123);
        assert!(json.get("host_rss_by_process_id").is_none());
        assert!(json.get("host_cpu_pressure_by_process_id").is_none());
        assert!(!json.to_string().contains("4242"));
    }

    #[test]
    fn lane_resource_ewma_is_deterministic_and_clamped() {
        assert_eq!(
            next_lane_resource_ewma(256 * MIB, 512 * MIB, 128 * MIB, GIB),
            320 * MIB
        );
        assert_eq!(
            next_lane_resource_ewma(0, 0, 128 * MIB, GIB),
            128 * MIB
        );
        assert_eq!(
            next_lane_resource_ewma(u64::MAX, u64::MAX, 128 * MIB, GIB),
            GIB
        );
    }
}
