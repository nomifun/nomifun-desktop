/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { ImageFiles } from '@icon-park/react';
import SiderNavItem, { type SiderNavItemProps } from './SiderNavItem';

type SiderAssetLibraryEntryProps = Pick<SiderNavItemProps, 'isActive' | 'collapsed' | 'siderTooltipProps' | 'onClick'>;

const SiderAssetLibraryEntry: React.FC<SiderAssetLibraryEntryProps> = ({ collapsed = false, ...props }) => {
  const { t } = useTranslation();
  return <SiderNavItem {...props} collapsed={collapsed} label={t('assetLibrary.siderTitle', { defaultValue: t('creativeStudio.navigation.assets') })}
    icon={<ImageFiles theme='outline' size={collapsed ? 20 : 16} fill='currentColor' />} />;
};

export default SiderAssetLibraryEntry;
