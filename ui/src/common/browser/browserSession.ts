/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  httpGet,
  httpPost,
  httpPut,
  redactSensitiveText,
  withResponseMap,
  wsEmitter,
} from '@/common/adapter/httpBridge';
import type {
  BrowserDisplayMode,
  BrowserCloseResult,
  BrowserIdentityMode,
  BrowserLaneLifecycleState,
  BrowserResourcePressureState,
  IBrowserBackgroundResult,
  IBrowserCapacityOverview,
  IBrowserDisplayModePolicy,
  IBrowserForegroundResult,
  IBrowserHost,
  IBrowserInventoryChangedEvent,
  IBrowserLane,
  IBrowserLaneIdentity,
  IBrowserLaneOwner,
  IBrowserLaneQueue,
  IBrowserOverview,
  IBrowserTab,
} from './browserTypes';

type UnknownRecord = Record<string, unknown>;

const asRecord = (value: unknown): UnknownRecord =>
  value != null && typeof value === 'object' && !Array.isArray(value) ? (value as UnknownRecord) : {};

const firstString = (record: UnknownRecord, ...keys: string[]): string | undefined => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'string' && value.length > 0) return value;
  }
  return undefined;
};

const firstNumber = (record: UnknownRecord, ...keys: string[]): number | undefined => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'number' && Number.isFinite(value)) return value;
  }
  return undefined;
};

const firstBoolean = (record: UnknownRecord, ...keys: string[]): boolean | undefined => {
  for (const key of keys) {
    const value = record[key];
    if (typeof value === 'boolean') return value;
  }
  return undefined;
};

const nullableString = (record: UnknownRecord, ...keys: string[]): string | null | undefined => {
  for (const key of keys) {
    const value = record[key];
    if (value === null) return null;
    if (typeof value === 'string') return value;
  }
  return undefined;
};

const normalizeTab = (raw: unknown): IBrowserTab | null => {
  const value = asRecord(raw);
  const tabId = firstString(value, 'tab_id', 'id');
  if (!tabId) return null;
  return {
    tab_id: tabId,
    title: nullableString(value, 'title'),
    url: nullableString(value, 'url'),
    active: firstBoolean(value, 'active', 'is_active'),
    crashed: firstBoolean(value, 'crashed', 'is_crashed'),
  };
};

const normalizeOwner = (raw: unknown, lane: UnknownRecord): IBrowserLaneOwner | null => {
  const value = asRecord(raw);
  const owner: IBrowserLaneOwner = {
    user_id: nullableString(value, 'user_id') ?? nullableString(lane, 'user_id'),
    conversation_id:
      nullableString(value, 'conversation_id') ?? nullableString(lane, 'conversation_id'),
    runtime_instance_id:
      nullableString(value, 'runtime_instance_id', 'runtime_id') ??
      nullableString(lane, 'runtime_instance_id', 'runtime_id'),
    execution_id:
      nullableString(value, 'execution_id') ?? nullableString(lane, 'execution_id'),
    attempt_id: nullableString(value, 'attempt_id') ?? nullableString(lane, 'attempt_id'),
    cluster_node_id:
      nullableString(value, 'cluster_node_id') ?? nullableString(lane, 'cluster_node_id'),
    agent_id: nullableString(value, 'agent_id') ?? nullableString(lane, 'agent_id'),
    agent_name: nullableString(value, 'agent_name') ?? nullableString(lane, 'agent_name'),
    surface: nullableString(value, 'surface') ?? nullableString(lane, 'surface'),
    label: nullableString(value, 'label', 'owner_label') ?? nullableString(lane, 'owner_label'),
  };
  return Object.values(owner).some((entry) => entry != null) ? owner : null;
};

const normalizeIdentity = (raw: unknown, lane: UnknownRecord): IBrowserLaneIdentity | null => {
  const value = asRecord(raw);
  const mode = firstString(value, 'mode', 'identity_mode') ?? firstString(lane, 'identity_mode');
  if (!mode) return null;
  return {
    mode: mode as BrowserIdentityMode,
    label: nullableString(value, 'label'),
    generation: firstNumber(value, 'generation', 'identity_generation'),
    shared_live: firstBoolean(value, 'shared_live', 'is_shared_live'),
  };
};

const normalizeQueue = (raw: unknown, lane: UnknownRecord): IBrowserLaneQueue | null => {
  const value = asRecord(raw);
  const queue: IBrowserLaneQueue = {
    position: firstNumber(value, 'position', 'queue_position') ?? firstNumber(lane, 'queue_position'),
    reason_code:
      nullableString(value, 'reason_code', 'capacity_reason_code') ??
      nullableString(lane, 'capacity_reason_code', 'queue_reason_code'),
    reason: nullableString(value, 'reason', 'message'),
    retry_delay_ms: firstNumber(value, 'retry_delay_ms', 'retry_after_ms'),
    recommended_concurrency: firstNumber(value, 'recommended_concurrency'),
    owner_active: firstNumber(value, 'owner_active'),
    owner_queued: firstNumber(value, 'owner_queued'),
    global_active: firstNumber(value, 'global_active'),
    global_queued: firstNumber(value, 'global_queued'),
  };
  return Object.values(queue).some((entry) => entry != null) ? queue : null;
};

export const normalizeBrowserLane = (raw: unknown): IBrowserLane | null => {
  const value = asRecord(raw);
  const laneId = firstString(value, 'lane_id', 'id');
  if (!laneId) return null;
  const rawTabs = Array.isArray(value.tabs)
    ? value.tabs
    : Array.isArray(value.targets)
      ? value.targets
      : [];
  const tabs = rawTabs.map(normalizeTab).filter((tab): tab is IBrowserTab => tab != null);
  const lifecycle =
    firstString(value, 'lifecycle_state', 'state', 'status') ?? 'failed';
  const owner = normalizeOwner(value.owner, value);

  return {
    lane_id: laneId,
    lane_name: nullableString(value, 'lane_name', 'name'),
    lifecycle_state: lifecycle as BrowserLaneLifecycleState,
    browser_epoch: nonNegativeNumber(value, 'browser_epoch', 'epoch'),
    conversation_id:
      nullableString(value, 'conversation_id') ?? owner?.conversation_id,
    conversation_title: nullableString(value, 'conversation_title'),
    runtime_instance_id:
      nullableString(value, 'runtime_instance_id', 'runtime_id') ?? owner?.runtime_instance_id,
    runtime_label: nullableString(value, 'runtime_label'),
    execution_id: nullableString(value, 'execution_id') ?? owner?.execution_id,
    attempt_id: nullableString(value, 'attempt_id') ?? owner?.attempt_id,
    cluster_node_id:
      nullableString(value, 'cluster_node_id') ?? owner?.cluster_node_id,
    cluster_node_label: nullableString(value, 'cluster_node_label'),
    owner,
    identity: normalizeIdentity(value.identity, value),
    queue: normalizeQueue(value.queue, value),
    tabs,
    active_tab_id: nullableString(value, 'active_tab_id', 'selected_tab_id'),
    title: nullableString(value, 'title'),
    url: nullableString(value, 'url'),
    last_active_at: firstNumber(value, 'last_active_at', 'updated_at'),
    created_at: firstNumber(value, 'created_at'),
    resource_estimate_bytes: firstNumber(value, 'resource_estimate_bytes', 'estimated_bytes'),
    active_operation: firstBoolean(value, 'active_operation', 'has_active_operation'),
    active_operation_count: firstNumber(value, 'active_operation_count'),
    error_code: nullableString(value, 'error_code', 'code'),
    error_message: (() => {
      const message = nullableString(value, 'error_message', 'message');
      return typeof message === 'string' ? redactSensitiveText(message) : message;
    })(),
    recoverable: firstBoolean(value, 'recoverable', 'retryable'),
  };
};

export const normalizeBrowserLanes = (raw: unknown): IBrowserLane[] => {
  const value = asRecord(raw);
  const items = Array.isArray(raw)
    ? raw
    : Array.isArray(value.lanes)
      ? value.lanes
      : Array.isArray(value.items)
        ? value.items
        : [];
  return items.map(normalizeBrowserLane).filter((lane): lane is IBrowserLane => lane != null);
};

const normalizeCapacity = (raw: unknown): IBrowserCapacityOverview | null => {
  const value = asRecord(raw);
  const result: IBrowserCapacityOverview = {
    active: firstNumber(value, 'active'),
    queued: firstNumber(value, 'queued'),
    max_active: firstNumber(value, 'max_active', 'active_limit'),
    max_open_lanes: firstNumber(value, 'max_open_lanes', 'open_lane_limit'),
    recommended_concurrency: firstNumber(value, 'recommended_concurrency'),
    reason_code: nullableString(value, 'reason_code'),
  };
  return Object.values(result).some((entry) => entry != null) ? result : null;
};

const nonNegativeNumber = (record: UnknownRecord, ...keys: string[]): number | undefined => {
  const value = firstNumber(record, ...keys);
  return value != null && value >= 0 ? value : undefined;
};

/**
 * Project only the renderer-safe Host diagnostics allow-list. Do not spread
 * the backend object here: operational response additions must be reviewed
 * before they can become renderer-visible fields.
 */
export const normalizeBrowserHost = (raw: unknown): IBrowserHost | null => {
  const value = asRecord(raw);
  const hostId = firstString(value, 'host_id', 'id');
  if (!hostId) return null;

  return {
    host_id: hostId,
    state: (firstString(value, 'state', 'lifecycle_state', 'status') ?? 'unknown') as
      IBrowserHost['state'],
    epoch: nonNegativeNumber(value, 'epoch', 'browser_epoch'),
    headful: firstBoolean(value, 'headful', 'is_headful'),
    identity_mode: firstString(value, 'identity_mode', 'mode') as
      | IBrowserHost['identity_mode']
      | undefined,
    lane_count: nonNegativeNumber(value, 'lane_count', 'lanes', 'open_lanes'),
    rss_bytes: nonNegativeNumber(value, 'rss_bytes', 'memory_rss_bytes'),
  };
};

export const normalizeBrowserHosts = (raw: unknown): IBrowserHost[] => {
  const value = asRecord(raw);
  const items = Array.isArray(raw)
    ? raw
    : Array.isArray(value.hosts)
      ? value.hosts
      : [];
  return items.map(normalizeBrowserHost).filter((host): host is IBrowserHost => host != null);
};

export const normalizeBrowserOverview = (raw: unknown): IBrowserOverview => {
  const value = asRecord(raw);
  const counts = asRecord(value.counts);
  const running =
    firstNumber(value, 'running_lanes', 'running') ??
    firstNumber(counts, 'running_lanes', 'running') ??
    0;
  const queued =
    firstNumber(value, 'queued_lanes', 'queued') ??
    firstNumber(counts, 'queued_lanes', 'queued') ??
    0;
  return {
    supported: firstBoolean(value, 'supported', 'available'),
    enabled: firstBoolean(value, 'enabled'),
    running_lanes: running,
    queued_lanes: queued,
    total_lanes: firstNumber(value, 'total_lanes', 'total') ?? running + queued,
    pressure_state: nullableString(value, 'pressure_state', 'resource_state') as
      | BrowserResourcePressureState
      | null
      | undefined,
    capacity: normalizeCapacity(value.capacity),
    hosts: normalizeBrowserHosts(value.hosts),
    managed_host_count: nonNegativeNumber(
      value,
      'managed_host_count',
      'managed_hosts'
    ),
    pending_cleanup_count: nonNegativeNumber(
      value,
      'pending_cleanup_count',
      'pending_cleanups'
    ),
    can_close_all: firstBoolean(value, 'can_close_all'),
    can_manage_browser_settings: firstBoolean(value, 'can_manage_browser_settings'),
    can_manage_primary_identity: firstBoolean(value, 'can_manage_primary_identity'),
    updated_at: firstNumber(value, 'updated_at'),
  };
};

const normalizeBrowserDisplayModePolicy = (raw: unknown): IBrowserDisplayModePolicy => {
  const root = asRecord(raw);
  const value = asRecord(root.data);
  const displayMode =
    firstString(value, 'display_mode', 'mode') ??
    firstString(root, 'display_mode', 'mode');
  if (displayMode !== 'headless' && displayMode !== 'external') {
    throw new Error('The browser manager returned an invalid display mode.');
  }
  return { display_mode: displayMode };
};

export const browserSession = {
  overview: withResponseMap(
    httpGet<unknown, void>('/api/browser/overview', { silentStatuses: [404, 501] }),
    normalizeBrowserOverview
  ),
  lanes: withResponseMap(
    httpGet<unknown, void>('/api/browser/lanes', { silentStatuses: [404, 501] }),
    normalizeBrowserLanes
  ),
  closeLane: httpPost<BrowserCloseResult, { lane_id: string }>(
    ({ lane_id }) => `/api/browser/lanes/${encodeURIComponent(lane_id)}/close`,
    () => undefined
  ),
  foregroundLane: httpPost<IBrowserForegroundResult, { lane_id: string }>(
    ({ lane_id }) => `/api/browser/lanes/${encodeURIComponent(lane_id)}/foreground`,
    () => undefined
  ),
  backgroundLane: httpPost<IBrowserBackgroundResult, { lane_id: string }>(
    ({ lane_id }) => `/api/browser/lanes/${encodeURIComponent(lane_id)}/background`,
    () => undefined
  ),
  displayMode: {
    get: withResponseMap(
      httpGet<unknown, void>('/api/browser/display-mode', {
        silentStatuses: [404, 501],
      }),
      normalizeBrowserDisplayModePolicy
    ),
    put: withResponseMap(
      httpPut<unknown, { display_mode: BrowserDisplayMode }>(
        '/api/browser/display-mode',
        ({ display_mode }) => ({ display_mode })
      ),
      normalizeBrowserDisplayModePolicy
    ),
  },
  closeConversation: httpPost<BrowserCloseResult, { conversation_id: string }>(
    ({ conversation_id }) =>
      `/api/browser/conversations/${encodeURIComponent(conversation_id)}/close`,
    () => undefined
  ),
  closeAll: httpPost<BrowserCloseResult, void>('/api/browser/close-all'),
  events: {
    inventoryChanged: wsEmitter<IBrowserInventoryChangedEvent>('browser.inventory.changed'),
    lifecycleChanged: wsEmitter<IBrowserInventoryChangedEvent>('browser.lifecycle.changed'),
  },
};
