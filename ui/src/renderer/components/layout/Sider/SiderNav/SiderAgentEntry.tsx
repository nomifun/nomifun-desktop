/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Robot } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderAgentEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderAgentEntry: React.FC<SiderAgentEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('agentSettings.navigation.railTitle', { defaultValue: 'Agent Workbench' })}
    icon={<Robot theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderAgentEntry;
