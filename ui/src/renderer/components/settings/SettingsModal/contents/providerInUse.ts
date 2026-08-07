/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  parseCompanionId,
  parseConversationId,
  parseCsAgentId,
  parseExecutionId,
  type CompanionId,
  type ConversationId,
  type CsAgentId,
  type ExecutionId,
} from '@/common/types/ids';

export type ProviderUsageFeature =
  | 'desktopCompanion'
  | 'customerService'
  | 'conversation'
  | 'agentExecution';

export type ProviderUsageTargetId = CompanionId | CsAgentId | ConversationId | ExecutionId;

export interface ProviderUsage {
  feature: ProviderUsageFeature;
  label: string;
  targetId?: ProviderUsageTargetId;
}

/** Deep-link route for a feature's unbind location. Verify against Router.tsx. */
export function featureRoute(feature: ProviderUsageFeature, targetId?: ProviderUsageTargetId): string {
  switch (feature) {
    case 'desktopCompanion':
      // Desktop-companion model control (CompanionModelControl → profile.model)
      // lives on the Nomi config page; /companion is the transparent pet overlay.
      return '/nomi';
    case 'customerService':
      return targetId ? `/customer-service/` : '/customer-service';
    case 'conversation':
      return targetId ? `/conversation/${targetId}` : '/guid';
    case 'agentExecution':
      return '/guid';
  }
}

export interface ProviderUsageGroup {
  feature: ProviderUsageFeature;
  labels: string[];
  targetId?: ProviderUsageTargetId;
}

export function groupUsagesByFeature(usages: ProviderUsage[]): ProviderUsageGroup[] {
  const map = new Map<ProviderUsageFeature, ProviderUsageGroup>();
  for (const u of usages) {
    const g = map.get(u.feature) ?? { feature: u.feature, labels: [], targetId: u.targetId };
    g.labels.push(u.label);
    map.set(u.feature, g);
  }
  return [...map.values()];
}

/** Extract usages from a BackendHttpError.details payload. */
export function parseProviderInUseDetails(details: unknown): ProviderUsage[] {
  if (details && typeof details === 'object' && Array.isArray((details as { usages?: unknown }).usages)) {
    return (details as { usages: unknown[] }).usages.flatMap((item): ProviderUsage[] => {
      if (!item || typeof item !== 'object') return [];
      const raw = item as { feature?: unknown; label?: unknown; targetId?: unknown };
      if (typeof raw.feature !== 'string' || typeof raw.label !== 'string') return [];
      const feature = raw.feature as ProviderUsageFeature;
      if (!['desktopCompanion', 'customerService', 'conversation', 'agentExecution'].includes(feature)) {
        return [];
      }
      if (raw.targetId == null) return [{ feature, label: raw.label }];
      const targetId = (() => {
        switch (feature) {
          case 'desktopCompanion': return parseCompanionId(raw.targetId);
          case 'customerService': return parseCsAgentId(raw.targetId);
          case 'conversation': return parseConversationId(raw.targetId);
          case 'agentExecution': return parseExecutionId(raw.targetId);
        }
      })();
      return [{ feature, label: raw.label, ...(targetId ? { targetId } : {}) }];
    });
  }
  return [];
}
