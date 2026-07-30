/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { IBrowserHost, IBrowserLane } from '@/common/browser/browserTypes';
import { parseSessionRoute } from '@/renderer/utils/routes/sessionRoute';

const UNASSIGNED_CONVERSATION = '__browser_unassigned__';

/**
 * Resolve the Host serving a primary Lane. Lanes and overview arrive from two
 * independent snapshot requests, so a display-mode restart can leave one side
 * carrying a stale epoch. Match by the browser epoch — the only host linkage
 * lane payloads carry (BrowserLaneDto serializes no host_id) — and tolerate
 * skew by accepting the single primary Host, but never guess between multiple
 * candidates. If the backend ever serializes a stable lane host_id, reintroduce
 * it here as the preferred match ahead of the epoch.
 */
export const matchBrowserLaneHost = (
  lane: IBrowserLane | null | undefined,
  hosts: IBrowserHost[] | null | undefined
): IBrowserHost | null => {
  if (!lane || lane.identity?.mode !== 'primary' || !hosts?.length) return null;
  const primaryHosts = hosts.filter((host) => host.identity_mode === 'primary');
  if (primaryHosts.length === 0) return null;

  if (lane.browser_epoch != null) {
    const byEpoch = primaryHosts.find((host) => host.epoch === lane.browser_epoch);
    if (byEpoch) return byEpoch;
  }
  return primaryHosts.length === 1 ? primaryHosts[0] : null;
};

export interface BrowserOwnerGroup {
  key: string;
  label: string;
  lanes: IBrowserLane[];
  lastActiveAt: number;
}

export interface BrowserConversationGroup {
  conversationId: string | null;
  key: string;
  label: string;
  owners: BrowserOwnerGroup[];
  lanes: IBrowserLane[];
  runningCount: number;
  queuedCount: number;
  lastActiveAt: number;
}

export interface BrowserInventoryLabels {
  clusterNode: (id: string) => string;
  attempt: (id: string) => string;
  runtime: (id: string) => string;
  execution: (id: string) => string;
  owner: string;
  laneOwner: string;
  conversation: (id: string) => string;
  unassigned: string;
}

const conversationIdFromLocationState = (state: unknown): string | null => {
  if (state == null || typeof state !== 'object' || Array.isArray(state)) return null;
  const value = state as Record<string, unknown>;
  const direct = value.conversation_id ?? value.conversationId;
  if (typeof direct === 'string' && direct.trim()) return direct.trim();

  if (value.conversation && typeof value.conversation === 'object') {
    const conversation = value.conversation as Record<string, unknown>;
    const nested = conversation.id ?? conversation.conversation_id;
    if (typeof nested === 'string' && nested.trim()) return nested.trim();
  }
  return null;
};

/**
 * Resolve the conversation context without inventing another global store.
 *
 * Explicit Browser query scope always wins. When a caller has supplied an
 * existing router state, use that next; a canonical conversation route is
 * also accepted for deep-link routing. Otherwise return null and let the
 * status-only inventory ordering use its existing activity-based fallback.
 */
export const resolveBrowserConversationId = ({
  requestedConversationId,
  pathname,
  locationState,
}: {
  requestedConversationId?: string | null;
  pathname?: string;
  locationState?: unknown;
}): string | null => {
  const requested = requestedConversationId?.trim();
  if (requested) return requested;

  const fromState = conversationIdFromLocationState(locationState);
  if (fromState) return fromState;

  if (pathname) {
    const route = parseSessionRoute(pathname);
    if (route?.kind === 'conversation') return String(route.id);
  }

  return null;
};

const shortId = (value: string): string =>
  value.length > 14 ? `${value.slice(0, 8)}…${value.slice(-4)}` : value;

const laneLastActiveAt = (lane: IBrowserLane): number =>
  lane.last_active_at ?? lane.created_at ?? 0;

export const browserLaneConversationId = (lane: IBrowserLane): string | null =>
  lane.conversation_id ?? lane.owner?.conversation_id ?? null;

export const browserConversationSearchParamsForLane = (
  current: URLSearchParams,
  lane: IBrowserLane
): URLSearchParams => {
  const next = new URLSearchParams(current);
  const conversationId = browserLaneConversationId(lane);
  if (conversationId) next.set('conversation_id', conversationId);
  else next.delete('conversation_id');
  return next;
};

const ownerDescriptor = (
  lane: IBrowserLane,
  labels: BrowserInventoryLabels
): { key: string; label: string } => {
  const owner = lane.owner;
  const nodeId = lane.cluster_node_id ?? owner?.cluster_node_id;
  if (nodeId) {
    return {
      key: `cluster:${nodeId}`,
      label: lane.cluster_node_label || owner?.label || labels.clusterNode(shortId(nodeId)),
    };
  }

  const attemptId = lane.attempt_id ?? owner?.attempt_id;
  if (attemptId) {
    return {
      key: `attempt:${attemptId}`,
      label: owner?.agent_name || owner?.label || labels.attempt(shortId(attemptId)),
    };
  }

  const runtimeId = lane.runtime_instance_id ?? owner?.runtime_instance_id;
  if (runtimeId) {
    return {
      key: `runtime:${runtimeId}`,
      label: lane.runtime_label || owner?.agent_name || owner?.label || labels.runtime(shortId(runtimeId)),
    };
  }

  const executionId = lane.execution_id ?? owner?.execution_id;
  if (executionId) {
    return {
      key: `execution:${executionId}`,
      label: owner?.agent_name || owner?.label || labels.execution(shortId(executionId)),
    };
  }

  const surface = owner?.surface;
  if (surface || owner?.agent_id || owner?.agent_name || owner?.label) {
    return {
      key: `owner:${surface ?? owner?.agent_id ?? owner?.agent_name ?? owner?.label}`,
      label: owner?.agent_name || owner?.label || surface || labels.owner,
    };
  }

  return { key: 'owner:unassigned', label: labels.laneOwner };
};

/**
 * Builds the renderer hierarchy from the authoritative lane inventory.
 * Execution/conversation records only enrich labels; they never invent lanes.
 */
export const buildBrowserInventoryTree = (
  lanes: IBrowserLane[],
  conversationNames: Readonly<Record<string, string>>,
  labels: BrowserInventoryLabels,
  currentConversationId?: string | null
): BrowserConversationGroup[] => {
  const grouped = new Map<string, IBrowserLane[]>();

  for (const lane of lanes) {
    const conversationId = browserLaneConversationId(lane);
    const key = conversationId ?? UNASSIGNED_CONVERSATION;
    const existing = grouped.get(key);
    if (existing) existing.push(lane);
    else grouped.set(key, [lane]);
  }

  const conversations = [...grouped.entries()].map(([key, conversationLanes]) => {
    const conversationId = key === UNASSIGNED_CONVERSATION ? null : key;
    const ownerMap = new Map<string, { label: string; lanes: IBrowserLane[] }>();

    for (const lane of conversationLanes) {
      const descriptor = ownerDescriptor(lane, labels);
      const existing = ownerMap.get(descriptor.key);
      if (existing) existing.lanes.push(lane);
      else ownerMap.set(descriptor.key, { label: descriptor.label, lanes: [lane] });
    }

    const owners: BrowserOwnerGroup[] = [...ownerMap.entries()]
      .map(([ownerKey, owner]) => {
        const sortedLanes = [...owner.lanes].sort(
          (left, right) => laneLastActiveAt(right) - laneLastActiveAt(left)
        );
        return {
          key: ownerKey,
          label: owner.label,
          lanes: sortedLanes,
          lastActiveAt: Math.max(0, ...sortedLanes.map(laneLastActiveAt)),
        };
      })
      .sort((left, right) => right.lastActiveAt - left.lastActiveAt);

    const firstBackendTitle = conversationLanes.find(
      (lane) => lane.conversation_title?.trim()
    )?.conversation_title;
    return {
      conversationId,
      key,
      label:
        firstBackendTitle?.trim() ||
        (conversationId ? conversationNames[conversationId] : undefined) ||
        (conversationId ? labels.conversation(shortId(conversationId)) : labels.unassigned),
      owners,
      lanes: conversationLanes,
      runningCount: conversationLanes.filter((lane) =>
        ['starting', 'running', 'frozen'].includes(lane.lifecycle_state)
      ).length,
      queuedCount: conversationLanes.filter((lane) => lane.lifecycle_state === 'queued').length,
      lastActiveAt: Math.max(0, ...conversationLanes.map(laneLastActiveAt)),
    };
  });

  return conversations.sort((left, right) => {
    if (currentConversationId) {
      if (left.conversationId === currentConversationId) return -1;
      if (right.conversationId === currentConversationId) return 1;
    }
    return right.lastActiveAt - left.lastActiveAt;
  });
};

export const pickDefaultBrowserLaneId = (
  groups: BrowserConversationGroup[],
  requestedConversationId?: string | null
): string | null => {
  const requested = requestedConversationId
    ? groups.find((group) => group.conversationId === requestedConversationId)
    : undefined;
  return requested?.owners[0]?.lanes[0]?.lane_id ?? groups[0]?.owners[0]?.lanes[0]?.lane_id ?? null;
};

export const browserLaneCounts = (lanes: IBrowserLane[]) => ({
  running: lanes.filter((lane) => ['starting', 'running', 'frozen'].includes(lane.lifecycle_state)).length,
  queued: lanes.filter((lane) => lane.lifecycle_state === 'queued').length,
});
