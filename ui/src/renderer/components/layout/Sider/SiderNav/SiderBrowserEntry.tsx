/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Tooltip } from '@arco-design/web-react';
import { WebPage } from '@icon-park/react';
import classNames from 'classnames';
import { useTranslation } from 'react-i18next';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';

interface SiderBrowserEntryProps {
  isMobile: boolean;
  isActive: boolean;
  collapsed: boolean;
  runningCount: number;
  queuedCount: number;
  siderTooltipProps: SiderTooltipProps;
  onClick: () => void;
}

const SiderBrowserEntry: React.FC<SiderBrowserEntryProps> = ({
  isMobile,
  isActive,
  collapsed,
  runningCount,
  queuedCount,
  siderTooltipProps,
  onClick,
}) => {
  const { t } = useTranslation();
  const label = t('browser.sider.label');
  const countLabel = t('browser.sider.counts', { running: runningCount, queued: queuedCount });
  const tooltip = t('browser.sider.tooltip', { label, counts: countLabel });

  if (collapsed) {
    return (
      <Tooltip {...siderTooltipProps} content={tooltip} position='right'>
        <div
          className={classNames(
            'relative w-full h-32px flex items-center justify-center cursor-pointer transition-colors rd-8px text-t-primary',
            isActive ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
          )}
          onClick={onClick}
        >
          <WebPage
            theme='outline'
            size='20'
            fill='currentColor'
            className='block leading-none shrink-0'
            style={{ lineHeight: 0 }}
          />
          {runningCount + queuedCount > 0 && (
            <span className='absolute right-5px top-4px min-w-12px h-12px px-2px rd-full bg-primary-6 text-white text-8px leading-12px text-center'>
              {Math.min(99, runningCount + queuedCount)}
            </span>
          )}
        </div>
      </Tooltip>
    );
  }

  return (
    <Tooltip {...siderTooltipProps} content={tooltip} position='right'>
      <div
        className={classNames(
          'box-border group h-32px w-full flex items-center justify-start gap-8px pl-10px pr-8px rd-0.5rem cursor-pointer shrink-0 transition-all text-t-primary',
          isMobile && 'sider-action-btn-mobile',
          isActive ? '!bg-primary-1 !text-primary-6' : 'hover:bg-fill-2 active:bg-fill-3'
        )}
        onClick={onClick}
      >
        <span className='size-22px flex items-center justify-center shrink-0'>
          <WebPage
            theme='outline'
            size='16'
            fill='currentColor'
            className='block leading-none'
            style={{ lineHeight: 0 }}
          />
        </span>
        <span className='collapsed-hidden min-w-0 flex-1 truncate text-14px font-[500] leading-24px'>
          {label}
        </span>
        <span
          className='collapsed-hidden shrink-0 text-10px leading-18px text-t-tertiary'
          aria-label={countLabel}
        >
          {runningCount}/{queuedCount}
        </span>
      </div>
    </Tooltip>
  );
};

export default SiderBrowserEntry;
