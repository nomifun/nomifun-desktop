/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** A sidebar owns its state; the titlebar reads it and requests toggles. */
export function createContentSiderChannel(name: string, initiallyAvailable = true) {
  const toggleEvent = `nomifun-${name}-sider-toggle`;
  const stateEvent = `nomifun-${name}-sider-state`;
  const initialSnapshot = { collapsed: false, available: initiallyAvailable, position: 'side' as 'side' | 'bottom' };
  let snapshot = initialSnapshot;
  const listeners = new Set<() => void>();

  const dispatchState = (collapsed: boolean, available = true, position: 'side' | 'bottom' = 'side') => {
    if (snapshot.collapsed !== collapsed || snapshot.available !== available || snapshot.position !== position) {
      snapshot = { collapsed, available, position };
      listeners.forEach((listener) => listener());
    }
    if (typeof window !== 'undefined') {
      window.dispatchEvent(new CustomEvent(stateEvent, { detail: snapshot }));
    }
  };

  return {
    toggleEvent,
    stateEvent,
    getSnapshot: () => snapshot,
    getServerSnapshot: () => initialSnapshot,
    subscribe: (listener: () => void) => {
      listeners.add(listener);
      return () => { listeners.delete(listener); };
    },
    dispatchState,
    setUnavailable: () => dispatchState(snapshot.collapsed, false, snapshot.position),
    dispatchToggle: () => {
      if (typeof window !== 'undefined') window.dispatchEvent(new CustomEvent(toggleEvent));
    },
  };
}

export type ContentSiderChannel = ReturnType<typeof createContentSiderChannel>;
