/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useEffect, useState } from 'react';

/**
 * Collapse state for a {@link ContentSider} (content-area secondary sidebar).
 *
 * Persisted to localStorage under `storageKey` so the user's expand/collapse
 * preference survives reloads and route changes. Mirrors the lightweight
 * Context-free pattern used across the app (no zustand/redux); a `storage`
 * event listener keeps multiple mounts of the same key in sync.
 */
export interface ContentSiderCollapseState {
  collapsed: boolean;
  toggle: () => void;
  setCollapsed: (value: boolean) => void;
}

const readPersisted = (storageKey: string, fallback: boolean): boolean => {
  try {
    const stored = localStorage.getItem(storageKey);
    if (stored === 'collapsed') return true;
    if (stored === 'expanded') return false;
  } catch {
    // ignore storage access errors (private mode / quota)
  }
  return fallback;
};

export function useContentSiderCollapse(storageKey: string, defaultCollapsed = false): ContentSiderCollapseState {
  const [collapsed, setCollapsedState] = useState<boolean>(() => readPersisted(storageKey, defaultCollapsed));

  const persist = useCallback(
    (value: boolean) => {
      try {
        localStorage.setItem(storageKey, value ? 'collapsed' : 'expanded');
      } catch {
        // ignore storage write errors
      }
    },
    [storageKey]
  );

  const setCollapsed = useCallback(
    (value: boolean) => {
      setCollapsedState(value);
      persist(value);
    },
    [persist]
  );

  const toggle = useCallback(() => {
    setCollapsedState((prev) => {
      const next = !prev;
      persist(next);
      return next;
    });
  }, [persist]);

  // Keep multiple mounts (and other tabs) consistent.
  useEffect(() => {
    if (typeof window === 'undefined') return undefined;
    const handler = (event: StorageEvent) => {
      if (event.key !== storageKey) return;
      if (event.newValue === 'collapsed') setCollapsedState(true);
      else if (event.newValue === 'expanded') setCollapsedState(false);
    };
    window.addEventListener('storage', handler);
    return () => window.removeEventListener('storage', handler);
  }, [storageKey]);

  return { collapsed, toggle, setCollapsed };
}

export default useContentSiderCollapse;
