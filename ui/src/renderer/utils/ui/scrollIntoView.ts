/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Scroll a sidebar row into view by its DOM id, waiting for layout to settle.
 *
 * Expanding a section (or an async list refresh) mounts the target row on a later
 * tick, so a synchronous scroll would miss it. A double requestAnimationFrame defers
 * the scroll until after the browser has laid out the freshly-rendered rows — the
 * same approach used for scrolling the active conversation into view.
 *
 * Returns a cancel function so callers inside a useEffect can abort a pending scroll
 * on cleanup.
 */
export const scrollSidebarItemIntoView = (domId: string): (() => void) => {
  let outerRaf = 0;
  let innerRaf = 0;
  let cancelled = false;
  outerRaf = requestAnimationFrame(() => {
    innerRaf = requestAnimationFrame(() => {
      if (cancelled) return;
      document.getElementById(domId)?.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    });
  });
  return () => {
    cancelled = true;
    cancelAnimationFrame(outerRaf);
    cancelAnimationFrame(innerRaf);
  };
};
