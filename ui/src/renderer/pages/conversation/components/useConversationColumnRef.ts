/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback } from 'react';

/**
 * Match the composer to the transcript's actual content box, including native
 * scrollbar gutters in narrow desktop panes. Only the composer consumes this
 * measurement, so updating it cannot shrink the observed transcript in a loop.
 */
export function useConversationColumnRef(
  onNode?: (node: HTMLDivElement | null) => void
) {
  return useCallback((node: HTMLDivElement | null) => {
    onNode?.(node);
    if (!node) return;
    const host = node.closest<HTMLElement>('[data-conversation-layout]');
    if (!host) return () => onNode?.(null);
    let previousWidth = -1;
    const syncWidth = () => {
      const width = node.getBoundingClientRect().width;
      if (width <= 0 || width === previousWidth) return;
      previousWidth = width;
      host.style.setProperty('--conversation-content-width', `${width}px`);
    };
    syncWidth();
    const observer = new ResizeObserver(syncWidth);
    observer.observe(node);
    return () => {
      observer.disconnect();
      host.style.removeProperty('--conversation-content-width');
      onNode?.(null);
    };
  }, [onNode]);
}
