/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */
import React from 'react';
import { useTranslation } from 'react-i18next';
import { ArrowCircleLeft, CloseOne, SettingTwo } from '@icon-park/react';
import classNames from 'classnames';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import SiderThemeControl from './SiderThemeControl';
import SiderNavItem from './SiderNav/SiderNavItem';

interface SiderFooterProps {
  isSettings: boolean;
  collapsed?: boolean;
  siderTooltipProps: SiderTooltipProps;
  onSettingsClick: () => void;
  backLabel?: string;
  showLogout?: boolean;
  onLogoutClick?: () => void;
}

const SiderFooter: React.FC<SiderFooterProps> = ({
  isSettings, collapsed = false, siderTooltipProps, onSettingsClick, backLabel,
  showLogout = false, onLogoutClick,
}) => {
  const { t } = useTranslation();
  const isBackAction = isSettings || Boolean(backLabel);
  const actionLabel = isBackAction ? backLabel || t('common.back') : t('common.settings');
  const Icon = isBackAction ? ArrowCircleLeft : SettingTwo;
  return <div className='shrink-0 sider-footer pb-5px'>
    <div className={classNames('flex gap-1px', collapsed ? 'flex-col' : 'items-center')}>
      <SiderNavItem className='flex-1' label={actionLabel} icon={<Icon theme='outline' size={16} />}
        collapsed={collapsed} siderTooltipProps={siderTooltipProps} onClick={onSettingsClick} />
      <SiderThemeControl collapsed={collapsed} siderTooltipProps={siderTooltipProps} />
      {showLogout && onLogoutClick && <SiderNavItem className='flex-1' label={t('settings.googleLogout')}
        icon={<CloseOne theme='outline' size={16} />} collapsed={collapsed}
        siderTooltipProps={siderTooltipProps} onClick={onLogoutClick} />}
    </div>
  </div>;
};

export default SiderFooter;
