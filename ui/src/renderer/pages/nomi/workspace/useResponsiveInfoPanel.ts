/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useLayoutEffect, useState } from 'react';

const COLLAPSE_WIDTH = 840;
const RESTORE_WIDTH = 872;
// The existing docked panel is 282px wide. Keep at least 480px for chat when
// someone explicitly restores the panel inside a narrower workspace.
const MIN_EXPANDED_WIDTH = 282 + 480;

interface LayoutState {
  width: number;
  automaticallyCollapsed: boolean;
  manuallyExpanded: boolean;
}

interface Options {
  manuallyCollapsed: boolean;
  setManuallyCollapsed: (collapsed: boolean) => void;
}

/**
 * Measure the whole workspace, whose width is independent of the info panel.
 * Automatic hiding never changes the saved manual preference. A manual restore
 * in the narrow range lasts until the workspace returns to the wide range.
 */
export function useResponsiveInfoPanel({ manuallyCollapsed, setManuallyCollapsed }: Options) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const [layout, setLayout] = useState<LayoutState>({
    width: 0,
    automaticallyCollapsed: false,
    manuallyExpanded: false,
  });

  // A callback ref also observes the workspace when it appears after loading.
  const containerRef = useCallback((element: HTMLDivElement | null) => setContainer(element), []);

  useLayoutEffect(() => {
    if (!container) return undefined;
    const measure = () => {
      const width = container.getBoundingClientRect().width;
      if (!Number.isFinite(width) || width <= 0) return;
      setLayout((previous) => {
        const automaticallyCollapsed = previous.automaticallyCollapsed
          ? width < RESTORE_WIDTH
          : width < COLLAPSE_WIDTH;
        const manuallyExpanded = automaticallyCollapsed && previous.manuallyExpanded && width >= MIN_EXPANDED_WIDTH;
        if (previous.width === width
          && previous.automaticallyCollapsed === automaticallyCollapsed
          && previous.manuallyExpanded === manuallyExpanded) return previous;
        return { width, automaticallyCollapsed, manuallyExpanded };
      });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [container]);

  const canExpand = layout.width >= MIN_EXPANDED_WIDTH;
  const collapsed = manuallyCollapsed || !canExpand || (layout.automaticallyCollapsed && !layout.manuallyExpanded);

  const toggle = useCallback(() => {
    if (collapsed) {
      if (!canExpand) return;
      setManuallyCollapsed(false);
      setLayout((previous) => ({ ...previous, manuallyExpanded: previous.automaticallyCollapsed }));
    } else {
      setManuallyCollapsed(true);
      setLayout((previous) => ({ ...previous, manuallyExpanded: false }));
    }
  }, [canExpand, collapsed, setManuallyCollapsed]);

  return { containerRef, collapsed, canExpand, toggle };
}
