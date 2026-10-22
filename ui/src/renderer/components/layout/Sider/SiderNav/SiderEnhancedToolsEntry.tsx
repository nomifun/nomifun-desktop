/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Puzzle } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderEnhancedToolsEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderEnhancedToolsEntry: React.FC<SiderEnhancedToolsEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('settings.enhancedTools.title')}
    icon={<Puzzle theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderEnhancedToolsEntry;
