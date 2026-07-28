/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Button, Tag } from '@arco-design/web-react';
import { Delete, Refresh, WebPage } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import type { BrowserResourcePressureState } from '@/common/browser/browserTypes';

interface BrowserPageHeaderProps {
  runningCount: number;
  queuedCount: number;
  pressureState?: BrowserResourcePressureState | null;
  refreshing: boolean;
  closingAll: boolean;
  hasManagedResources: boolean;
  controlsDisabled?: boolean;
  canCloseAll: boolean;
  closeAllLabel?: string;
  onRefresh: () => void;
  onCloseAll: () => void;
}

const BrowserPageHeader: React.FC<BrowserPageHeaderProps> = ({
  runningCount,
  queuedCount,
  pressureState,
  refreshing,
  closingAll,
  hasManagedResources,
  controlsDisabled = false,
  canCloseAll,
  closeAllLabel,
  onRefresh,
  onCloseAll,
}) => {
  const { t } = useTranslation();
  const pressureStateLabel = (() => {
    switch (pressureState) {
      case 'normal':
        return t('browser.state.pressure.normal');
      case 'pressured':
        return t('browser.state.pressure.pressured');
      case 'critical':
        return t('browser.state.pressure.critical');
      default:
        return pressureState;
    }
  })();

  return (
    <header className='shrink-0 flex flex-wrap items-center gap-10px mb-12px'>
      <div className='size-34px rd-9px bg-primary-1 text-primary-6 flex items-center justify-center'>
        <WebPage theme='outline' size='18' />
      </div>
      <div className='min-w-0 flex-1'>
        <h1 className='m-0 text-20px leading-28px'>{t('browser.page.title')}</h1>
        <div className='text-12px text-t-secondary'>
          {t('browser.page.description')}
        </div>
      </div>
      <Tag color='green'>{t('browser.page.runningCount', { count: runningCount })}</Tag>
      <Tag color='orange'>{t('browser.page.queuedCount', { count: queuedCount })}</Tag>
      {pressureState && pressureState !== 'normal' && (
        <Tag color={pressureState === 'critical' ? 'red' : 'orange'}>
          {t('browser.page.pressure', { state: pressureStateLabel })}
        </Tag>
      )}
      <Button
        type='outline'
        loading={refreshing}
        disabled={controlsDisabled}
        icon={<Refresh theme='outline' size='14' />}
        onClick={onRefresh}
      >
        {t('browser.page.refresh')}
      </Button>
      {canCloseAll === true && (
        <Button
          status='danger'
          type='outline'
          disabled={!hasManagedResources || controlsDisabled}
          loading={closingAll}
          icon={<Delete theme='outline' size='14' />}
          onClick={onCloseAll}
        >
          {closeAllLabel ?? t('browser.page.closeAll')}
        </Button>
      )}
    </header>
  );
};

export default BrowserPageHeader;
