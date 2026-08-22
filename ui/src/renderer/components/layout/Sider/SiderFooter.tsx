/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tooltip } from '@arco-design/web-react';
import { ArrowCircleLeft, CloseOne, SettingTwo } from '@icon-park/react';
import classNames from 'classnames';
import { iconColors } from '@renderer/styles/colors';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import SiderThemeControl from './SiderThemeControl';

interface SiderFooterProps {
  isMobile: boolean;
  isSettings: boolean;
  collapsed?: boolean;
  siderTooltipProps: SiderTooltipProps;
  onSettingsClick: () => void;
  backLabel?: string;
  showLogout?: boolean;
  onLogoutClick?: () => void;
}

const SiderFooter: React.FC<SiderFooterProps> = ({
  isMobile,
  isSettings,
  collapsed = false,
  siderTooltipProps,
  onSettingsClick,
  backLabel,
  showLogout = false,
  onLogoutClick,
}) => {
  const { t } = useTranslation();

  const isBackAction = isSettings || Boolean(backLabel);
  const actionLabel = isBackAction ? backLabel || t('common.back') : t('common.settings');
  const settingsIcon = isBackAction ? (
    <ArrowCircleLeft
      theme='outline'
      size='16'
      fill='currentColor'
      className='block leading-none'
      style={{ lineHeight: 0 }}
    />
  ) : (
    <SettingTwo
      theme='outline'
      size='16'
      fill='currentColor'
      className='block leading-none'
      style={{ lineHeight: 0 }}
    />
  );

  return (
    <div className='shrink-0 sider-footer pb-5px'>
      <div className={classNames('flex', collapsed ? 'flex-col gap-1px' : 'items-center gap-1px')}>
        <Tooltip {...siderTooltipProps} content={actionLabel} position='right'>
          <div
            onClick={onSettingsClick}
            className={classNames(
              'group h-28px flex items-center rd-0.5rem cursor-pointer transition-colors',
              collapsed ? 'w-full justify-center' : 'flex-1 min-w-0 justify-start gap-8px pl-10px pr-8px',
              isMobile && 'sider-footer-btn-mobile',
              {
                '!bg-primary-1 !text-primary-6': isSettings,
                'hover:bg-fill-2 active:bg-fill-3': !isSettings,
              }
            )}
          >
            <span className={classNames('size-22px flex items-center justify-center shrink-0', isSettings ? 'text-primary-6' : 'text-t-secondary')}>{settingsIcon}</span>
            <span className={classNames('collapsed-hidden text-14px font-[500] leading-24px truncate', isSettings ? 'text-primary-6' : 'text-t-primary')}>
              {actionLabel}
            </span>
          </div>
        </Tooltip>

        {/* 主题（明暗 + 缩放 + CSS 预设）/ Theme (light-dark + scaling + CSS preset) */}
        <SiderThemeControl
          isMobile={isMobile}
          collapsed={collapsed}
          siderTooltipProps={siderTooltipProps}
        />

        {showLogout && onLogoutClick && (
          <Tooltip {...siderTooltipProps} content={t('settings.googleLogout')} position='right'>
            <div
              onClick={onLogoutClick}
              className={classNames(
                'h-28px flex items-center rd-0.5rem cursor-pointer transition-colors hover:bg-[rgba(var(--primary-6),0.14)] active:bg-fill-2',
                collapsed ? 'w-full justify-center' : 'flex-1 min-w-0 justify-start gap-10px px-14px',
                isMobile && 'sider-footer-btn-mobile'
              )}
            >
              <span className='size-20px flex items-center justify-center shrink-0'>
                <CloseOne
                  theme='outline'
                  size='16'
                  fill={iconColors.primary}
                  className='block leading-none'
                  style={{ lineHeight: 0 }}
                />
              </span>
              <span className='collapsed-hidden text-t-primary text-14px font-[500] leading-24px truncate'>
                {t('settings.googleLogout')}
              </span>
            </div>
          </Tooltip>
        )}
      </div>
    </div>
  );
};

export default SiderFooter;
