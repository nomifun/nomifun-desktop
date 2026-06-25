/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';

import type { SessionKind } from '../utils/workpathTree';

/**
 * Persisted UI preferences for the unified workpath session list.
 *
 * Storage model (localStorage + CustomEvent broadcast, so multiple mounted
 * instances — e.g. desktop sider + mobile drawer — stay in sync):
 * - `nomifun:workpath-pinned`              string[]; array order is the manual pin order
 * - `nomifun:workpath-expansion`           Record<workpathKey, boolean>; drawers default to COLLAPSED
 * - `nomifun:workpath-subgroup-expansion`  Record<`${workpathKey}:${kind}`, boolean>; subgroups default to EXPANDED
 */
export const WORKPATH_PINNED_STORAGE_KEY = 'nomifun:workpath-pinned';
export const WORKPATH_EXPANSION_STORAGE_KEY = 'nomifun:workpath-expansion';
export const WORKPATH_SUBGROUP_STORAGE_KEY = 'nomifun:workpath-subgroup-expansion';

const WORKPATH_UI_EVENT = 'nomifun:workpath-ui-changed';

type WorkpathUiChangeDetail = {
  storageKey: string;
};

const readJson = <T>(storageKey: string, fallback: T): T => {
  if (typeof window === 'undefined') return fallback;
  try {
    const raw = localStorage.getItem(storageKey);
    if (!raw) return fallback;
    const parsed = JSON.parse(raw) as unknown;
    if (parsed === null || typeof parsed !== 'object') return fallback;
    return parsed as T;
  } catch {
    return fallback;
  }
};

const writeJson = (storageKey: string, value: unknown): void => {
  if (typeof window === 'undefined') return;
  try {
    localStorage.setItem(storageKey, JSON.stringify(value));
  } catch {
    // ignore storage errors (quota / privacy mode)
  }
  window.dispatchEvent(new CustomEvent<WorkpathUiChangeDetail>(WORKPATH_UI_EVENT, { detail: { storageKey } }));
};

const readPinned = (): string[] => {
  const parsed = readJson<unknown>(WORKPATH_PINNED_STORAGE_KEY, []);
  return Array.isArray(parsed) ? parsed.filter((item): item is string => typeof item === 'string') : [];
};

const readExpansion = (): Record<string, boolean> => readJson<Record<string, boolean>>(WORKPATH_EXPANSION_STORAGE_KEY, {});

const readSubgroup = (): Record<string, boolean> => readJson<Record<string, boolean>>(WORKPATH_SUBGROUP_STORAGE_KEY, {});

const subgroupKey = (workpathKey: string, kind: SessionKind): string => `${workpathKey}:${kind}`;

export type WorkpathUiState = {
  /** Pinned workpath keys; array order = manual pin order (most recently pinned first). */
  pinnedKeys: string[];
  togglePinned: (workpathKey: string) => void;
  /** First-level drawer expansion. Default: collapsed. */
  isExpanded: (workpathKey: string) => boolean;
  toggleExpanded: (workpathKey: string) => void;
  /** Idempotently expand a drawer (used by reveal-on-create). */
  expand: (workpathKey: string) => void;
  /** Second-level kind subgroup expansion. Default: expanded. */
  isSubgroupExpanded: (workpathKey: string, kind: SessionKind) => boolean;
  toggleSubgroup: (workpathKey: string, kind: SessionKind) => void;
  /** Idempotently expand a kind subgroup (used by reveal-on-create). */
  expandSubgroup: (workpathKey: string, kind: SessionKind) => void;
};

export const useWorkpathUiState = (): WorkpathUiState => {
  const [pinnedKeys, setPinnedKeys] = useState<string[]>(() => readPinned());
  const [expansion, setExpansion] = useState<Record<string, boolean>>(() => readExpansion());
  const [subgroup, setSubgroup] = useState<Record<string, boolean>>(() => readSubgroup());

  // Cross-instance sync: same-window via CustomEvent, cross-window via 'storage'.
  useEffect(() => {
    const reload = (storageKey: string | null) => {
      if (!storageKey || storageKey === WORKPATH_PINNED_STORAGE_KEY) setPinnedKeys(readPinned());
      if (!storageKey || storageKey === WORKPATH_EXPANSION_STORAGE_KEY) setExpansion(readExpansion());
      if (!storageKey || storageKey === WORKPATH_SUBGROUP_STORAGE_KEY) setSubgroup(readSubgroup());
    };
    const handleUiEvent = (event: Event) => {
      reload((event as CustomEvent<WorkpathUiChangeDetail>).detail?.storageKey ?? null);
    };
    const handleStorage = (event: StorageEvent) => {
      if (
        event.key === WORKPATH_PINNED_STORAGE_KEY ||
        event.key === WORKPATH_EXPANSION_STORAGE_KEY ||
        event.key === WORKPATH_SUBGROUP_STORAGE_KEY
      ) {
        reload(event.key);
      }
    };
    window.addEventListener(WORKPATH_UI_EVENT, handleUiEvent as EventListener);
    window.addEventListener('storage', handleStorage);
    return () => {
      window.removeEventListener(WORKPATH_UI_EVENT, handleUiEvent as EventListener);
      window.removeEventListener('storage', handleStorage);
    };
  }, []);

  const togglePinned = useCallback((workpathKey: string) => {
    // Read-modify-write against the latest persisted value so concurrent
    // instances don't clobber each other; the broadcast updates local state.
    const current = readPinned();
    const next = current.includes(workpathKey)
      ? current.filter((key) => key !== workpathKey)
      : // Most recently pinned first (节点排序按数组序，置顶时间倒序 == 头插)
        [workpathKey, ...current];
    writeJson(WORKPATH_PINNED_STORAGE_KEY, next);
    setPinnedKeys(next);
  }, []);

  const isExpanded = useCallback((workpathKey: string) => expansion[workpathKey] === true, [expansion]);

  const toggleExpanded = useCallback((workpathKey: string) => {
    const current = readExpansion();
    const next = { ...current, [workpathKey]: !(current[workpathKey] === true) };
    writeJson(WORKPATH_EXPANSION_STORAGE_KEY, next);
    setExpansion(next);
  }, []);

  const expand = useCallback((workpathKey: string) => {
    const current = readExpansion();
    if (current[workpathKey] === true) return;
    const next = { ...current, [workpathKey]: true };
    writeJson(WORKPATH_EXPANSION_STORAGE_KEY, next);
    setExpansion(next);
  }, []);

  const isSubgroupExpanded = useCallback(
    (workpathKey: string, kind: SessionKind) => subgroup[subgroupKey(workpathKey, kind)] !== false,
    [subgroup]
  );

  const toggleSubgroup = useCallback((workpathKey: string, kind: SessionKind) => {
    const current = readSubgroup();
    const key = subgroupKey(workpathKey, kind);
    const next = { ...current, [key]: current[key] === false };
    writeJson(WORKPATH_SUBGROUP_STORAGE_KEY, next);
    setSubgroup(next);
  }, []);

  const expandSubgroup = useCallback((workpathKey: string, kind: SessionKind) => {
    const current = readSubgroup();
    const key = subgroupKey(workpathKey, kind);
    if (current[key] !== false) return;
    const next = { ...current, [key]: true };
    writeJson(WORKPATH_SUBGROUP_STORAGE_KEY, next);
    setSubgroup(next);
  }, []);

  return {
    pinnedKeys,
    togglePinned,
    isExpanded,
    toggleExpanded,
    expand,
    isSubgroupExpanded,
    toggleSubgroup,
    expandSubgroup,
  };
};
