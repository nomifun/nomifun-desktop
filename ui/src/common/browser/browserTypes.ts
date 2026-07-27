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

export interface IBrowserCapacityOverview {
  active?: number | null;
  queued?: number | null;
  max_active?: number | null;
  max_open_lanes?: number | null;
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
};
