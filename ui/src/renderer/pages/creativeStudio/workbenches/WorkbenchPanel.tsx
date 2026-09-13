/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useId } from 'react';
import { useTranslation } from 'react-i18next';
import { useContentSiderCollapse } from '@/renderer/components/layout/ContentSider';
import contentSiderStyles from '@/renderer/components/layout/ContentSider/ContentSider.module.css';
import { useResizableSplit } from '@/renderer/hooks/ui/useResizableSplit';
import { workbenchSiderChannels } from '@/renderer/utils/workspace/workbenchSiderEvents';
import { useWorkbenchBottomResize } from './useWorkbenchBottomResize';
import styles from './WorkbenchPanel.module.css';

const DEFAULT_WIDTH = 380;
const MIN_WIDTH = 310;
const MAX_WIDTH = 480;

interface WorkbenchPanelProps {
  kind: 'image' | 'video';
  position?: 'side' | 'bottom';
  children: React.ReactNode;
}

/** Layout preferences are independent of generation drafts and task state. */
const WorkbenchPanel: React.FC<WorkbenchPanelProps> = ({ kind, position = 'side', children }) => {
  const { t } = useTranslation();
  const panelId = useId();
  const { collapsed, toggle } = useContentSiderCollapse(`nomifun:${kind}-workbench-sider-collapsed`);
  const channel = workbenchSiderChannels[kind];
  useEffect(() => {
    window.addEventListener(channel.toggleEvent, toggle);
    return () => {
      window.removeEventListener(channel.toggleEvent, toggle);
      channel.setUnavailable();
    };
  }, [channel, toggle]);
  useEffect(() => channel.dispatchState(collapsed, true, position), [channel, collapsed, position]);
  const resize = useResizableSplit({
    unit: 'px',
    defaultWidth: DEFAULT_WIDTH,
    minWidth: MIN_WIDTH,
    maxWidth: MAX_WIDTH,
    storageKey: `nomifun:${kind}-workbench-sider-width`,
  });
  const bottom = useWorkbenchBottomResize(`nomifun:${kind}-workbench-bottom-height`, position === 'bottom' && !collapsed);
  // The titlebar remains mounted and owns the expand entry; reserve no rail here.
  if (collapsed) return null;

  return (
    <div
      ref={bottom.panelRef}
      className={`${styles.container} ${contentSiderStyles.surface} ${position === 'bottom' ? styles.bottom : styles.side}`}
      style={position === 'bottom' ? { height: bottom.height, maxHeight: bottom.maxHeight } : { width: resize.splitRatio }}
      data-workbench-panel={kind}
      data-panel-position={position}
      data-workbench-sider={position === 'side' ? kind : undefined}
    >
      <div id={panelId} className={`${styles.viewport} ${contentSiderStyles.scrollArea}`}>
        <div ref={bottom.contentRef} className={styles.content}>{children}</div>
      </div>
      {position === 'bottom' ? (
        <div
          className={styles.bottomResizeHandle}
          role='separator'
          tabIndex={0}
          aria-label={t('creativeStudio.workbenchSider.resizeHeight')}
          title={t('creativeStudio.workbenchSider.resizeHeightHint')}
          aria-orientation='horizontal'
          aria-controls={panelId}
          aria-valuemin={bottom.minHeight}
          aria-valuemax={bottom.maxHeight}
          aria-valuenow={bottom.currentHeight}
          onPointerDown={bottom.onPointerDown}
          onKeyDown={bottom.onKeyDown}
          onDoubleClick={bottom.reset}
        />
      ) : (
        <div
          className={styles.resizeHandle}
          role='separator'
          tabIndex={0}
          aria-label={t('creativeStudio.workbenchSider.resize')}
          title={t('creativeStudio.workbenchSider.resizeHint')}
          aria-orientation='vertical'
          aria-controls={panelId}
          aria-valuemin={MIN_WIDTH}
          aria-valuemax={MAX_WIDTH}
          aria-valuenow={resize.splitRatio}
          onKeyDown={(event) => {
            const widths: Record<string, number> = {
              ArrowLeft: resize.splitRatio - 10,
              ArrowRight: resize.splitRatio + 10,
              Home: MIN_WIDTH,
              End: MAX_WIDTH,
            };
            const nextWidth = widths[event.key];
            if (nextWidth === undefined) return;
            event.preventDefault();
            resize.setSplitRatio(Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, nextWidth)));
          }}
        >
          {resize.createDragHandle({ className: 'right-0', style: { touchAction: 'none' } })}
        </div>
      )}
    </div>
  );
};

export default WorkbenchPanel;
