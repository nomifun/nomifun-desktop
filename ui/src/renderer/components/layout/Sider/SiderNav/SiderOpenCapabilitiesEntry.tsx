/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { LinkCloud } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderOpenCapabilitiesEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderOpenCapabilitiesEntry: React.FC<SiderOpenCapabilitiesEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('settings.openCapabilities.railTitle', { defaultValue: '远程&开放能力' })}
    icon={<LinkCloud theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderOpenCapabilitiesEntry;
