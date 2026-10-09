/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Puzzle } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderSkillsEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderSkillsEntry: React.FC<SiderSkillsEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('settings.skillsHub.railTitle', { defaultValue: 'Skills' })}
    icon={<Puzzle theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderSkillsEntry;
