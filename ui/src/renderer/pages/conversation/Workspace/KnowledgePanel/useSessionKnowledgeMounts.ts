/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import type { IKnowledgeBase, IKnowledgeBinding } from '@/common/adapter/ipcBridge';
import { browserStorageGenerationKey } from '@/common/utils/browserStorageKey';
import type { KnowledgeBaseId } from '@/common/types/ids';
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
 * "resolve these kb_ids" endpoint, so this needs `getBinding` for the ids and
 * `listBases` for their names.
 *
 * Three properties matter and each cost a bug during review:
 *
 * 1. **The first render must already know.** The rail tab list is what
 *    `WorkspaceRailBody` validates the persisted active tab against; if the
 *    knowledge tab is missing on the first render it normalizes the stored key
 *    to `files` and PERSISTS that, desyncing the rail (which keeps its own copy)
 *    from the body. So the last known mount state is mirrored into
 *    localStorage and read synchronously on mount.
 * 2. **Never blank while refreshing.** Dropping the cached list on a base event
 *    would flip `mounted` to false for a beat, which unmounts the panel and
 *    loses its tree. Refreshes replace, they do not clear.
 * 3. **A failed read must retry.** The fetch guards live inside the effects and
 *    are re-armed by an attempt counter, so a transient 500 does not wedge the
 *    entry off for the lifetime of the view.
 */

const bindingCache = new Map<string, IKnowledgeBinding>();
const bindingInflight = new Set<string>();
let basesCache: IKnowledgeBase[] | null = null;
let basesInflight = false;

/** Listeners are keyed by binding target so an unrelated binding change does not re-render every open session. */
const listeners = new Map<string, Set<() => void>>();

const notifyTarget = (targetKey: string) => listeners.get(targetKey)?.forEach((listener) => listener());
const notifyAll = () => listeners.forEach((set) => set.forEach((listener) => listener()));

let subscribed = false;
const ensureSubscribed = () => {
  if (subscribed) return;
  subscribed = true;
  // App-lifetime module subscriptions, same shape as useWorkpathKnowledge's.
  ipcBridge.knowledge.onBindingChanged.on((payload) => {
    const { target_kind, target_id, ...binding } = payload;
    const key = knowledgeBindingTargetKey({ kind: target_kind, target_id: String(target_id) });
    // The event carries the whole binding, so this is an update, not an
    // invalidation — no refetch needed.
    bindingCache.set(key, binding);
    notifyTarget(key);
  });
  const refreshBases = () => {
    // Replace-on-success rather than clear-then-fetch: clearing would make
    // `mounted` momentarily false and unmount the open panel.
    void ipcBridge.knowledge.listBases
      .invoke()
      .then((next) => {
        basesCache = next;
        notifyAll();
      })
      .catch(() => {
        /* keep the previous list; a later event or mount retries */
      });
  };
  ipcBridge.knowledge.onBaseCreated.on(refreshBases);
  ipcBridge.knowledge.onBaseUpdated.on(refreshBases);
  ipcBridge.knowledge.onBaseDeleted.on(refreshBases);
};

/**
 * Last known mounted ids per target, so the very first render of a returning
 * session already reports the right `mounted` value. Only ids are stored — names
 * and file counts always come from the live `listBases`.
 */
const seedStorageKey = (targetKey: string) => browserStorageGenerationKey(`knowledge-mounted:${targetKey}`);

function readSeed(targetKey: string): KnowledgeBaseId[] {
  if (typeof window === 'undefined') return [];
  try {
    const raw = localStorage.getItem(seedStorageKey(targetKey));
    if (!raw) return [];
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed.filter((id) => typeof id === 'string') as KnowledgeBaseId[]) : [];
  } catch {
    return [];
  }
}

function writeSeed(targetKey: string, ids: KnowledgeBaseId[]): void {
  if (typeof window === 'undefined') return;
  try {
    if (ids.length === 0) localStorage.removeItem(seedStorageKey(targetKey));
    else localStorage.setItem(seedStorageKey(targetKey), JSON.stringify(ids));
  } catch {
    /* private mode */
  }
}

const mountedIdsOf = (binding: IKnowledgeBinding | undefined): KnowledgeBaseId[] =>
  binding?.enabled ? binding.kb_ids : [];

export interface SessionKnowledgeMounts {
  /**
   * True when the session's binding is enabled with at least one base — the same
   * rule as the session-list capability dot (`useWorkpathKnowledgeLit`).
   *
   * Optimistic on the first render of a known session (seeded from
   * localStorage), then authoritative.
   */
  mounted: boolean;
  /** The mounted bases in the binding's own `kb_ids` order; empty until `listBases` lands. */
  bases: IKnowledgeBase[];
}

export function useSessionKnowledgeMounts(source: SessionKnowledgeSource | undefined): SessionKnowledgeMounts {
  const [, setTick] = useState(0);
  const [attempt, setAttempt] = useState(0);

  const target = useMemo(() => (source ? resolveKnowledgeBindingTarget(source) : null), [source]);
  const targetKey = target ? knowledgeBindingTargetKey(target) : null;

  useEffect(() => {
    if (!targetKey) return undefined;
    ensureSubscribed();
    const listener = () => setTick((tick) => tick + 1);
    const set = listeners.get(targetKey) ?? new Set();
    set.add(listener);
    listeners.set(targetKey, set);
    return () => {
      set.delete(listener);
      if (set.size === 0) listeners.delete(targetKey);
    };
  }, [targetKey]);

  const binding = targetKey ? bindingCache.get(targetKey) : undefined;

  // Fetch the binding. The in-flight check lives INSIDE the effect: computing it
  // during render lets two consumers mounted in the same commit both pass it.
  useEffect(() => {
    if (!target || !targetKey) return;
    if (bindingCache.has(targetKey) || bindingInflight.has(targetKey)) return;
    bindingInflight.add(targetKey);
    let cancelled = false;
    void (async () => {
      try {
        const next = await ipcBridge.knowledge.getBinding.invoke({
          kind: target.kind,
          target_id: target.target_id,
        });
        // A `binding-changed` event that landed mid-flight is newer than this
        // response — do not clobber it.
        if (!bindingCache.has(targetKey)) bindingCache.set(targetKey, next);
        if (!cancelled) writeSeed(targetKey, mountedIdsOf(bindingCache.get(targetKey)));
      } catch {
        // Re-arm so the next attempt tick retries instead of wedging.
        if (!cancelled) setAttempt((n) => n + 1);
      } finally {
        bindingInflight.delete(targetKey);
        notifyTarget(targetKey);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [target, targetKey, attempt]);

  const liveIds = mountedIdsOf(binding);
  const seedIds = useMemo(() => (targetKey ? readSeed(targetKey) : []), [targetKey, attempt]);
  // Before the binding resolves, trust the seed; afterwards the binding wins
  // (including when it says "nothing mounted any more").
  const mountedIds = binding ? liveIds : seedIds;
  const hasMountedIds = mountedIds.length > 0;

  useEffect(() => {
    if (!hasMountedIds) return;
    if (basesCache !== null || basesInflight) return;
    basesInflight = true;
    void (async () => {
      try {
        basesCache = await ipcBridge.knowledge.listBases.invoke();
      } catch {
        setAttempt((n) => n + 1);
      } finally {
        basesInflight = false;
        notifyAll();
      }
    })();
  }, [hasMountedIds, attempt]);

  const mountedKey = mountedIds.join(',');
  const bases = useMemo(() => {
    if (!hasMountedIds || basesCache === null) return [];
    const byId = new Map(basesCache.map((base) => [base.knowledge_base_id, base]));
    // Preserve the binding's order — it is the order the user picked them in.
    return mountedIds.map((id) => byId.get(id)).filter((base): base is IKnowledgeBase => Boolean(base));
    // basesCache is module state; every mutation is followed by a notify() that
    // re-renders subscribers, so its identity is a valid dep here.
  }, [hasMountedIds, mountedKey, basesCache]);

  return { mounted: hasMountedIds, bases };
}
