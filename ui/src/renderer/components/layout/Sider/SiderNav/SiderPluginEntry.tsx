/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Plug } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderPluginEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderPluginEntry: React.FC<SiderPluginEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('pluginPlatform.library.title')}
    icon={<Plug theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderPluginEntry;
