/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Browser platform renderer contract.
 *
 * The backend owns the authoritative BrowserSessionHub model.  The renderer
 * deliberately keeps non-critical fields optional so an older desktop can
 * still render a useful management surface while the backend adds inventory
 * metadata.
 */

type FutureValue<Known extends string> = Known | (string & {});

export type BrowserLaneLifecycleState = FutureValue<
  'queued' | 'starting' | 'running' | 'frozen' | 'stopping' | 'failed'
>;
export type BrowserIdentityMode = FutureValue<'primary' | 'anonymous' | 'authenticated_replica' | 'isolated'>;
export type BrowserResourcePressureState = FutureValue<'normal' | 'pressured' | 'critical'>;
/**
 * The user's visibility *policy*.
 *
 * `headless` and `external` pin the browser silent or visible; `auto` (the
 * default) lets the trusted host decide per lane, keeping routine agent work
 * silent and surfacing a window only when the user needs to step in.
 */
export type BrowserDisplayMode = 'headless' | 'auto' | 'external';
/**
 * The binary mechanism a managed host is running with right now.
 *
 * Reported separately from the policy because it cannot be inferred from it:
 * both `auto` and `headless` present as `headless`.
 */
export type BrowserEffectiveVisibility = 'headless' | 'headful';

export interface IBrowserLaneOwner {
  user_id?: string | null;
  conversation_id?: string | null;
  runtime_instance_id?: string | null;
  execution_id?: string | null;
  attempt_id?: string | null;
  cluster_node_id?: string | null;
  agent_id?: string | null;
  agent_name?: string | null;
  surface?: string | null;
  label?: string | null;
}

export interface IBrowserLaneIdentity {
  mode: BrowserIdentityMode;
  label?: string | null;
  generation?: number | null;
  shared_live?: boolean;
}

export interface IBrowserLaneQueue {
  position?: number | null;
  reason_code?: string | null;
  reason?: string | null;
  retry_delay_ms?: number | null;
  recommended_concurrency?: number | null;
  owner_active?: number | null;
  owner_queued?: number | null;
  global_active?: number | null;
  global_queued?: number | null;
}

export interface IBrowserTab {
  tab_id: string;
  title?: string | null;
  url?: string | null;
  active?: boolean;
  crashed?: boolean;
}

export interface IBrowserLane {
  lane_id: string;
  lane_name?: string | null;
  lifecycle_state: BrowserLaneLifecycleState;
  /**
   * Epoch of the managed Browser Host currently serving this Lane. This is
   * the only host linkage lane payloads carry: BrowserLaneDto serializes no
   * host_id, so host resolution goes through the epoch (see
   * matchBrowserLaneHost).
   */
  browser_epoch?: number | null;

  conversation_id?: string | null;
  conversation_title?: string | null;
  runtime_instance_id?: string | null;
  runtime_label?: string | null;
  execution_id?: string | null;
  attempt_id?: string | null;
  cluster_node_id?: string | null;
  cluster_node_label?: string | null;

  owner?: IBrowserLaneOwner | null;
  identity?: IBrowserLaneIdentity | null;
  queue?: IBrowserLaneQueue | null;
  tabs: IBrowserTab[];
  active_tab_id?: string | null;

  title?: string | null;
  url?: string | null;
  last_active_at?: number | null;
  created_at?: number | null;
  resource_estimate_bytes?: number | null;
  active_operation?: boolean;
  active_operation_count?: number | null;
  error_code?: string | null;
  error_message?: string | null;
  recoverable?: boolean;
}

/** Result returned after explicitly presenting a managed Primary Lane. */
export interface IBrowserForegroundResult {
  foregrounded: boolean;
  lane_id?: string;
}

/** Result returned after returning a managed Primary Lane to silent headless mode. */
export interface IBrowserBackgroundResult {
  backgrounded: boolean;
  lane_id?: string;
}

/** Installation-wide default visibility policy, owned and persisted by the backend. */
export interface IBrowserDisplayModePolicy {
  display_mode: BrowserDisplayMode;
  /**
   * What the managed host is actually running with now. Read-only: `PUT` accepts
   * `display_mode` alone and rejects a client-supplied value here.
   *
   * Optional because a backend predating the policy/mechanism split omits it.
   * `normalizeBrowserDisplayModePolicy` always populates it on the real response
   * path; treat an absent value as unknown and assume silent, which is the safe
   * direction.
   */
  effective_visibility?: BrowserEffectiveVisibility;
}

export interface IBrowserCapacityOverview {
  /** Scheduler-admitted Lanes (running or starting), not driver operations. */
  active_lanes?: number | null;
  /** @deprecated Compatibility alias for backends predating active_lanes. */
  active?: number | null;
  queued?: number | null;
  /** Global weighted-operation ceiling; it is not the active Lane denominator. */
  max_active?: number | null;
  max_open_lanes?: number | null;
  /** Elastic machine-wide pressure threshold, not a fixed aggregate quota. */
  global_memory_pressure_threshold_bytes?: number | null;
  /** Estimated attributed-memory budget for one task on shared Hosts. */
  max_task_memory_bytes?: number | null;
  /** Exact per-task structural limits. */
  max_task_active_operations?: number | null;
  max_task_open_lanes?: number | null;
  max_task_tabs?: number | null;
  recommended_concurrency?: number | null;
  reason_code?: string | null;
}

export type BrowserHostLifecycleState = FutureValue<
  'stopped' | 'starting' | 'running' | 'restarting' | 'stopping' | 'failed'
>;

/**
 * Renderer-safe Browser Host diagnostics.
 *
 * Keep this contract as an explicit allow-list. Process identifiers, raw CDP
 * addresses, debugging ports, and profile locations are intentionally absent.
 */
export interface IBrowserHost {
  host_id: string;
  state: BrowserHostLifecycleState;
  epoch?: number | null;
  /** Actual launch visibility of this Host, not merely the configured default. */
  headful?: boolean | null;
  identity_mode?: BrowserIdentityMode | null;
  lane_count?: number | null;
  rss_bytes?: number | null;
}

export interface IBrowserOverview {
  /** Explicit false hides the capability entry. Missing means "unknown, show". */
  supported?: boolean;
  enabled?: boolean;
  running_lanes: number;
  queued_lanes: number;
  total_lanes?: number;
  pressure_state?: BrowserResourcePressureState | null;
  capacity?: IBrowserCapacityOverview | null;
  hosts?: IBrowserHost[];
  /** Includes managed Hosts that currently have no attached Lane. */
  managed_host_count?: number | null;
  /** Lane/target cleanup work that has not yet reached a terminal state. */
  pending_cleanup_count?: number | null;
  /** Privileged installation-wide actions require an explicit true. */
  can_close_all?: boolean;
  can_manage_browser_settings?: boolean;
  can_manage_primary_identity?: boolean;
  updated_at?: number | null;
}

export interface IBrowserOverviewCapabilities {
  canCloseAll: boolean;
  canManageBrowserSettings: boolean;
  canManagePrimaryIdentity: boolean;
}

/** Missing or malformed privilege fields fail closed. */
export const resolveBrowserOverviewCapabilities = (
  overview:
    | Pick<
        IBrowserOverview,
        'can_close_all' | 'can_manage_browser_settings' | 'can_manage_primary_identity'
      >
    | null
    | undefined
): IBrowserOverviewCapabilities => ({
  canCloseAll: overview?.can_close_all === true,
  canManageBrowserSettings: overview?.can_manage_browser_settings === true,
  canManagePrimaryIdentity: overview?.can_manage_primary_identity === true,
});

export interface IBrowserInventoryChangedEvent {
  sequence?: number;
  change_kind?: string;
  lane_id?: string | null;
  user_id?: string | null;
  conversation_id?: string | null;
  /** Wall-clock timestamp supplied by the browser platform event source. */
  at_ms?: number;
  /** Set when the receiver must discard local deltas and fetch a snapshot. */
  resync_required?: boolean;
  /** Backward-compatible spelling accepted by the realtime consumer. */
  requires_resync?: boolean;
  /** Number of events discarded before the resync marker was emitted. */
  skipped?: number;
}

export type BrowserCloseResult = {
  closed?: number;
  already_closed?: boolean;
  /** Authoritative post-close inventory counts. Close-all is confirmed only at zero. */
  remaining_lane_count?: number;
  remaining_cleanup_count?: number;
  remaining_managed_host_count?: number;
};
