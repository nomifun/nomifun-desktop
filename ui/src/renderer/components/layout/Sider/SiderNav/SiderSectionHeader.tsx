/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import classNames from 'classnames';

interface SiderSectionHeaderProps {
  /** Already-translated section label (e.g. "常用"). */
  label: string;
  /** Icon-only rail mode: show a hairline rule instead of the text label. */
  collapsed: boolean;
  /**
   * Whether to draw the hairline rule in collapsed mode. Defaults to true.
   * Set false where an enclosing `border-t` already separates the region
   * (e.g. the bottom-pinned group), to avoid doubling the line.
   */
  collapsedRule?: boolean;
}

/**
 * SiderSectionHeader — the small-text group label that segments the primary
 * navigation rail (常用 / 数据空间 / 自动化 / 增强工具 / 设置).
 *
 * Mirrors the Settings sider group-header (`text-t-tertiary font-[500]`), sized
 * down to 12px to read as a quiet section divider. In the collapsed icon-only
 * rail there is no room for text, so it degrades to a hairline rule that keeps
 * the groups visually distinct.
 */
const SiderSectionHeader: React.FC<SiderSectionHeaderProps> = ({ label, collapsed, collapsedRule = true }) => {
  if (collapsed) {
    if (!collapsedRule) return null;
    return <div className='shrink-0 mt-6px mb-2px h-1px bg-[var(--color-border-2)] mx-6px' />;
  }

  return (
    <div className='shrink-0 mt-8px mb-2px px-12px h-22px flex items-center text-12px font-[500] leading-none text-t-tertiary select-none'>
      {label}
    </div>
  );
};

export default SiderSectionHeader;
