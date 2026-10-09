/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import './SiderNav.css';

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
  'aria-hidden'?: React.AriaAttributes['aria-hidden'];
}

/**
 * SiderSectionHeader — the small-text group label that segments the primary
 * navigation rail (常用 / 数据空间 / 自动化 / 增强工具 / 设置).
 *
 * Shared by the app rail, Settings and workspace navigation. Collapsed rails
 * retain a separator in place of the group caption.
 */
const SiderSectionHeader: React.FC<SiderSectionHeaderProps> = ({ label, collapsed, collapsedRule = true, 'aria-hidden': ariaHidden }) => {
  if (collapsed) {
    if (!collapsedRule) return null;
    return <div className='sider-nav-rule' aria-hidden='true' />;
  }

  return (
    <div className='sider-nav-group' aria-hidden={ariaHidden}>
      {label}
    </div>
  );
};

export default SiderSectionHeader;
