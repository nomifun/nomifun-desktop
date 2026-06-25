/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { useEffect, useState } from 'react';

import type { WorkpathNode } from '../utils/workpathTree';

// ── P3 终态（spec §7）──
// 知识库绑定已收敛到 workpath 级（一个 workpath 一份绑定，见 KnowledgeControl）。
// 抽屉的知识库能力 icon 因此只需查该 workpath 的单条 binding：
// `getBinding('workpath', node.key)`，enabled && kb_ids 非空 → 点亮。
// 不再逐成员 N+1 查询（P2 临时实现已删）。
// - 模块级缓存（卸载重挂不重查），in-flight 去重；
// - knowledge.binding-changed WS 事件增量刷新缓存；
// - 懒查时机：抽屉展开时（expanded）。
const enabledCache = new Map<string, boolean>();
const inflight = new Set<string>();
const listeners = new Set<() => void>();
let subscribed = false;

const notify = () => listeners.forEach((listener) => listener());

const ensureSubscribed = () => {
  if (subscribed) return;
  subscribed = true;
  // App-lifetime module subscription (deliberately never unsubscribed).
  ipcBridge.knowledge.onBindingChanged.on((payload) => {
    if (payload.target_kind !== 'workpath') return;
    enabledCache.set(payload.target_id, Boolean(payload.enabled && payload.kb_ids?.length));
    notify();
  });
};

/**
 * Whether the workpath drawer's knowledge-base capability icon is lit: the
 * workpath's single binding is enabled with at least one base. Fetched lazily
 * once per workpath while `expanded` (one request per workpath, never per
 * member session). Cached module-wide and refreshed by `knowledge.binding-changed`.
 */
export function useWorkpathKnowledgeLit(node: WorkpathNode, expanded: boolean): boolean {
  const [, setTick] = useState(0);

  useEffect(() => {
    ensureSubscribed();
    const listener = () => setTick((tick) => tick + 1);
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);

  const key = node.key;
  // Lazy fill: only the expanded drawer queries, only cache misses, deduped.
  const needsFetch = expanded && !enabledCache.has(key) && !inflight.has(key);

  useEffect(() => {
    if (!needsFetch) return;
    inflight.add(key);
    void (async () => {
      try {
        const binding = await ipcBridge.knowledge.getBinding.invoke({ kind: 'workpath', target_id: key });
        enabledCache.set(key, Boolean(binding?.enabled && binding.kb_ids?.length));
      } catch {
        // Transient failure: leave uncached so a later expand retries.
      } finally {
        inflight.delete(key);
        notify();
      }
    })();
  }, [needsFetch, key]);

  return enabledCache.get(key) === true;
}
