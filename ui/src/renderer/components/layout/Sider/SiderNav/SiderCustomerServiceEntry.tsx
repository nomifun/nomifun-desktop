/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Headset } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderCustomerServiceEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderCustomerServiceEntry: React.FC<SiderCustomerServiceEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('customerService.siderTitle', { defaultValue: '客服' })}
    icon={<Headset theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderCustomerServiceEntry;
