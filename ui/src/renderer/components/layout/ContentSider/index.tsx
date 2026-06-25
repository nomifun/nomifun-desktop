/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import classNames from 'classnames';
import styles from './ContentSider.module.css';

export { useContentSiderCollapse } from './useContentSiderCollapse';
export type { ContentSiderCollapseState } from './useContentSiderCollapse';

export interface ContentSiderProps {
  /** Expanded panel width in px. */
  width?: number;
  /**
   * Sticky header region (title + toolbar). Does not scroll. Typically holds
   * the create / search / batch / collapse actions.
   */
  header?: React.ReactNode;
  /** Scrollable body — the list / tree content. */
  children: React.ReactNode;
  /** Accessible label for the complementary region. */
  ariaLabel?: string;
  className?: string;
  /**
   * Optional drag-to-resize handle, absolutely positioned on the right edge.
   * Supply the element from `useResizableSplit().createDragHandle(...)`; the
   * panel root is `relative`, so the handle just needs `right-0` placement.
   */
  resizeHandle?: React.ReactNode;
}

/**
 * ContentSider — a content-area secondary sidebar (二级 / 内容区侧边栏).
 *
 * A generic, reusable collapsible panel that sits at the left edge of a
 * content-area page (inside the router `<Outlet/>`), as opposed to the
 * app-level primary `Sider`. Purely presentational: the owner decides whether
 * to render it based on collapse state (see `useContentSiderCollapse`) and
 * supplies the header actions. Currently used by the conversation section via
 * `ConversationShell`, but intentionally domain-agnostic.
 *
 * Visual language matches the primary rail: a recessed `bg-2` surface so it
 * reads as part of the left navigation region rather than a hard-bordered box
 * floating on the content. The single soft right edge (a hairline plus a faint
 * shadow) is the only boundary — internal compartments are separated by spacing
 * and tone, never hard lines, so the panel feels soft rather than boxy.
 */
const ContentSider: React.FC<ContentSiderProps> = ({
  width = 300,
  header,
  children,
  ariaLabel,
  className,
  resizeHandle,
}) => {
  return (
    <aside
      aria-label={ariaLabel}
      className={classNames('content-sider relative z-[1] shrink-0 h-full min-h-0 flex flex-col bg-2', className)}
      style={{
        width,
        // Soft right edge: a gentle shadow bleeding onto the content area instead
        // of a stark 1px divider. Paired with the recessed bg-2 tone, the boundary
        // reads柔和 in both themes — no hard line.
        boxShadow: '6px 0 14px -8px rgba(0, 0, 0, 0.14)',
      }}
    >
      {header ? <div className='shrink-0'>{header}</div> : null}
      <div className={classNames('flex-1 min-h-0 overflow-y-auto', styles.scrollArea)}>{children}</div>
      {resizeHandle}
    </aside>
  );
};

export default ContentSider;
