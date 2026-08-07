/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IKnowledgeBase, IKnowledgeBinding } from '@/common/adapter/ipcBridge';
import { useEffect, useMemo, useState } from 'react';
import {
  knowledgeBindingTargetKey,
  resolveKnowledgeBindingTarget,
  type SessionKnowledgeSource,
} from './knowledgeBindingTarget';

/**
 * Which knowledge bases a session actually has mounted.
 *
 * Neither the conversation nor the terminal payload carries any knowledge field
 * (see `TChatConversation.extra`, `ITerminalSession`), and there is no batch
 * "resolve these kb_ids" endpoint — so this needs `getBinding` for the ids and
 * `listBases` for their names. Both are cached module-wide with in-flight
 * de-duplication and refreshed from the knowledge WS events, mirroring
 * `SessionList/hooks/useWorkpathKnowledge.ts`: a rail entry must not cost a
 * request per re-mount, and several consumers on one screen must not multiply
 * them.
 */

const bindingCache = new Map<string, IKnowledgeBinding>();
const bindingInflight = new Set<string>();
let basesCache: IKnowledgeBase[] | null = null;
let basesInflight = false;

const listeners = new Set<() => void>();
let subscribed = false;

const notify = () => listeners.forEach((listener) => listener());

const ensureSubscribed = () => {
  if (subscribed) return;
  subscribed = true;
  // App-lifetime module subscriptions (deliberately never unsubscribed), same
  // shape as useWorkpathKnowledge's.
  ipcBridge.knowledge.onBindingChanged.on((payload) => {
    const { target_kind, target_id, ...binding } = payload;
    // The event carries the whole binding, so this is an update, not an
    // invalidation — no refetch needed.
    bindingCache.set(knowledgeBindingTargetKey({ kind: target_kind, target_id: String(target_id) }), binding);
    notify();
  });
  const invalidateBases = () => {
    basesCache = null;
    notify();
  };
  ipcBridge.knowledge.onBaseCreated.on(invalidateBases);
  ipcBridge.knowledge.onBaseUpdated.on(invalidateBases);
  ipcBridge.knowledge.onBaseDeleted.on(invalidateBases);
};

/** Drop every cached binding and base. Exported for tests only. */
export function resetSessionKnowledgeCacheForTests(): void {
  bindingCache.clear();
  bindingInflight.clear();
  basesCache = null;
  basesInflight = false;
}

export interface SessionKnowledgeMounts {
  /** True when the session has an enabled binding with at least one base. */
  mounted: boolean;
  /** The mounted bases, in the binding's own `kb_ids` order. */
  bases: IKnowledgeBase[];
  loading: boolean;
}

export function useSessionKnowledgeMounts(source: SessionKnowledgeSource | undefined): SessionKnowledgeMounts {
  const [, setTick] = useState(0);

  useEffect(() => {
    ensureSubscribed();
    const listener = () => setTick((tick) => tick + 1);
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);

  const target = useMemo(() => (source ? resolveKnowledgeBindingTarget(source) : null), [source]);
  const targetKey = target ? knowledgeBindingTargetKey(target) : null;

  const binding = targetKey ? bindingCache.get(targetKey) : undefined;
  const needsBinding = Boolean(targetKey) && binding === undefined && !(targetKey && bindingInflight.has(targetKey));

  useEffect(() => {
    if (!needsBinding || !target || !targetKey) return;
    bindingInflight.add(targetKey);
    void (async () => {
      try {
        const next = await ipcBridge.knowledge.getBinding.invoke({
          kind: target.kind,
          target_id: target.target_id,
        });
        bindingCache.set(targetKey, next);
      } catch {
        // Transient failure: leave uncached so a later mount retries. The rail
        // entry stays hidden meanwhile, which is the safe default.
      } finally {
        bindingInflight.delete(targetKey);
        notify();
      }
    })();
  }, [needsBinding, target, targetKey]);

  const mountedIds = binding?.enabled ? binding.kb_ids : [];
  const hasMountedIds = mountedIds.length > 0;
  const needsBases = hasMountedIds && basesCache === null && !basesInflight;

  useEffect(() => {
    if (!needsBases) return;
    basesInflight = true;
    void (async () => {
      try {
        basesCache = await ipcBridge.knowledge.listBases.invoke();
      } catch {
        // Leave null so the next consumer retries.
      } finally {
        basesInflight = false;
        notify();
      }
    })();
  }, [needsBases]);

  const bases = useMemo(() => {
    if (!hasMountedIds || basesCache === null) return [];
    const byId = new Map(basesCache.map((base) => [base.knowledge_base_id, base]));
    // Preserve the binding's order — it is the order the user picked them in.
    return mountedIds.map((id) => byId.get(id)).filter((base): base is IKnowledgeBase => Boolean(base));
  }, [hasMountedIds, mountedIds.join(','), basesCache]);

  return {
    // Gate on the resolved bases, not just the ids: a binding that still lists a
    // deleted base must not light up an entry that would open an empty tree.
    mounted: bases.length > 0,
    bases,
    loading: (needsBinding || (hasMountedIds && basesCache === null)) && bases.length === 0,
  };
}
