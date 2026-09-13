/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useCallback, useState } from 'react';

type Layout = 'side' | 'bottom';

/** Layout survives app restarts independently of the session-scoped creation draft. */
export function useWorkbenchLayout(kind: 'image' | 'video', fallback: Layout) {
  const key = `nomifun:${kind}-workbench-layout`;
  const [layout, setLayoutState] = useState<Layout>(() => {
    try {
      const stored = localStorage.getItem(key);
      if (stored === 'side' || stored === 'bottom') return stored;
    } catch { /* Use the restored draft when preferences cannot be read. */ }
    return fallback;
  });
  const setLayout = useCallback((value: Layout) => {
    setLayoutState(value);
    try { localStorage.setItem(key, value); } catch { /* Keep the current session usable. */ }
  }, [key]);
  return [layout, setLayout] as const;
}
