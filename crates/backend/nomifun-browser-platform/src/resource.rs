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
pub const MIN_RESERVED_MEMORY_BYTES: u64 = 256 * MIB;
pub const MAX_RESERVED_MEMORY_BYTES: u64 = 512 * GIB;

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
    pub lane_cold_start_bytes: u64,
    pub lane_ewma_min_bytes: u64,
    pub lane_ewma_max_bytes: u64,
    pub max_active_operations: usize,
    pub max_open_lanes: usize,
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
            .max(2 * GIB)
            .min(MAX_RESERVED_MEMORY_BYTES);
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
            lane_cold_start_bytes: 256 * MIB,
            lane_ewma_min_bytes: 128 * MIB,
            lane_ewma_max_bytes: GIB,
            max_active_operations: active,
            max_open_lanes: active.saturating_mul(4).min(MAX_OPEN_LANES),
            max_global_queue: MAX_GLOBAL_QUEUE,
            max_owner_queue: MAX_OWNER_QUEUE,
            sample_period_ms: 5_000,
            lifecycle_sweep_period_ms: 30_000,
            idle_expiry_ms: 10 * 60_000,
            pressured_idle_expiry_ms: 2 * 60_000,
            host_warm_ms: 60_000,
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
            }
            ResourcePolicyPreset::HighConcurrency => {
                policy.max_active_operations =
                    policy.max_active_operations.saturating_mul(2).min(MAX_ACTIVE_OPERATIONS);
                policy.max_open_lanes = policy
                    .max_active_operations
                    .saturating_mul(4)
                    .min(MAX_OPEN_LANES);
                policy.max_browser_memory_ratio = 0.5;
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
            ("host_warm_ms", self.host_warm_ms),
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
    pub viewer_lanes: usize,
    pub primary_lanes: usize,
    pub active_operation_permits: usize,
    /// Weighted visual/PDF/frame-encoding permits.
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
    /// Ordinary operations cost one unit and visual/PDF/frame work costs two.
    /// Unlike `recommended_concurrency`, this is a total live hard limit and
    /// therefore does not subtract operations which already hold permits.
    pub operation_weight_limit: usize,
    pub recommended_concurrency: usize,
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
        let reserved = self
            .reserved_memory_bytes
            .max(((total as f64) * 0.2) as u64);
        let browser_limit = ((total as f64) * self.max_browser_memory_ratio) as u64;
        let predicted_lane_bytes = predicted_lane_cost(self, workload);
        let system_headroom = telemetry.available_memory_bytes.saturating_sub(reserved);
        let browser_headroom = browser_limit.saturating_sub(telemetry.chromium_rss_bytes);
        let admission_headroom = system_headroom.min(browser_headroom);
        // Heavy operations and live viewers can allocate between samples. Keep
        // a bounded transient reserve so a Lane promotion does not spend their
        // entire safety margin using stale RSS.
        let transient_reserve = predicted_lane_bytes
            .saturating_mul(
                u64::try_from(workload.active_heavy_operation_permits).unwrap_or(u64::MAX),
            )
            .saturating_div(2)
            .saturating_add(
                predicted_lane_bytes
                    .saturating_mul(u64::try_from(workload.viewer_lanes).unwrap_or(u64::MAX))
                    .saturating_div(4),
            );
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
        let first_budget_available = admission_headroom >= first_lane_budget;
        let expansion_budget_available = admission_headroom >= expansion_lane_budget;
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
        .saturating_sub(workload.viewer_lanes)
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
        assert!(policy.max_active_operations <= MAX_ACTIVE_OPERATIONS);
        assert!(policy.max_open_lanes <= MAX_OPEN_LANES);
        assert_eq!(policy.max_global_queue, MAX_GLOBAL_QUEUE);
        assert_eq!(policy.max_owner_queue, MAX_OWNER_QUEUE);
    }

    #[test]
    fn validation_rejects_non_finite_ratio_and_absolute_cap_bypasses() {
        let mut policy = ResourcePolicy::default();
        policy.max_browser_memory_ratio = f64::NAN;
        assert_eq!(
            policy.validate().unwrap_err().field,
            "max_browser_memory_ratio"
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
    fn pressure_stops_expansion_before_first_lane() {
        let policy = ResourcePolicy::automatic(8 * GIB, 8);
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: 6 * GIB,
            chromium_rss_bytes: ((8 * GIB) as f64 * policy.max_browser_memory_ratio) as u64
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
    fn first_lane_budget_shortfall_has_system_memory_pressure_reason() {
        let policy = ResourcePolicy::automatic(8 * GIB, 8);
        let decision = policy.decide(&ResourceTelemetry {
            total_memory_bytes: 8 * GIB,
            available_memory_bytes: policy
                .reserved_memory_bytes
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
    fn measured_cpu_pressure_reduces_concurrency_with_healthy_memory() {
        let policy = ResourcePolicy::automatic(16 * GIB, 8);
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
            (policy.max_active_operations / 2).max(1)
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
        let policy = ResourcePolicy::automatic(16 * GIB, 8);
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
                    active_lane_ewma_bytes: 4 * GIB,
                    queued_lane_estimate_bytes: 2 * GIB,
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
                    active_lane_ewma_bytes: 4 * GIB,
                    queued_lane_estimate_bytes: 2 * GIB,
                    ..Default::default()
                },
            )
            .recommended_concurrency;

        assert!(expensive < baseline);
        assert!(frozen < expensive);
    }

    #[test]
    fn permits_viewers_and_primary_reserve_real_recommended_capacity() {
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
                    viewer_lanes: 1,
                    primary_lanes: 2,
                    active_operation_permits: 2,
                    active_heavy_operation_permits: 1,
                    active_lane_ewma_bytes: 3 * policy.lane_cold_start_bytes,
                    ..Default::default()
                },
            )
            .recommended_concurrency;

        assert_eq!(loaded, baseline.saturating_sub(6).max(1));
    }

    #[test]
    fn measured_logical_cpu_count_caps_configured_operation_concurrency() {
        let mut policy = ResourcePolicy::automatic(64 * GIB, 64);
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
    fn process_id_rss_join_index_is_never_serialized() {
        let telemetry = ResourceTelemetry {
            chromium_rss_bytes: 123,
            host_rss_by_process_id: HashMap::from([(4_242, 123)]),
            ..Default::default()
        };

        let json = serde_json::to_value(telemetry).unwrap();
        assert_eq!(json["chromium_rss_bytes"], 123);
        assert!(json.get("host_rss_by_process_id").is_none());
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
