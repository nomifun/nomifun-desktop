/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ExpandLeft, ExpandRight } from '@icon-park/react';
import React, { useSyncExternalStore } from 'react';
import InstantHoverTooltip from '@/renderer/components/base/InstantHoverTooltip';
import type { ContentSiderChannel } from './createContentSiderChannel';

interface ContentSiderTitlebarToggleProps {
  channel: ContentSiderChannel;
  expandLabel: string;
  collapseLabel: string;
  strokeWidth?: number;
}

/** Stable titlebar entry shared by conversations, agents and creative workbenches. */
const ContentSiderTitlebarToggle: React.FC<ContentSiderTitlebarToggleProps> = ({
  channel, expandLabel, collapseLabel, strokeWidth,
}) => {
  const { collapsed, available, position } = useSyncExternalStore(channel.subscribe, channel.getSnapshot, channel.getServerSnapshot);
  if (!available) return null;
  const label = collapsed ? expandLabel : collapseLabel;

  return (
    <InstantHoverTooltip content={label} position='bottom'>
      <button
        type='button'
        className='app-titlebar__button app-titlebar__button--nav'
        data-content-sider-toggle
        data-panel-position={position}
        aria-label={label}
        aria-expanded={!collapsed}
        onClick={channel.dispatchToggle}
      >
        <span style={{ display: 'inline-flex', transform: position === 'bottom' ? 'rotate(-90deg)' : undefined }}>
          {collapsed ? (
            <ExpandRight theme='outline' size={18} fill='currentColor' strokeWidth={strokeWidth} />
          ) : (
            <ExpandLeft theme='outline' size={18} fill='currentColor' strokeWidth={strokeWidth} />
          )}
        </span>
      </button>
    </InstantHoverTooltip>
  );
};

export default ContentSiderTitlebarToggle;
