/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { MessageOne } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderConversationEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderConversationEntry: React.FC<SiderConversationEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('sessionList.siderTitle')}
    icon={<MessageOne theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderConversationEntry;
