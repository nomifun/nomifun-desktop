/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Alert, Radio, Spin, Tag } from '@arco-design/web-react';
import { useTranslation } from 'react-i18next';
import type { BrowserDisplayMode } from '@/common/browser/browserTypes';

const RadioGroup = Radio.Group;

export type BrowserDisplayModeControlStatus =
  | 'loading'
  | 'ready'
  | 'unavailable'
  | 'error';

interface BrowserDisplayModeControlProps {
  displayMode: BrowserDisplayMode;
  status: BrowserDisplayModeControlStatus;
  saving: boolean;
  disabled?: boolean;
  error?: string | null;
  onChange: (mode: BrowserDisplayMode) => void;
}

const BrowserDisplayModeControl: React.FC<BrowserDisplayModeControlProps> = ({
  displayMode,
  status,
  saving,
  disabled = false,
  error,
  onChange,
}) => {
  const { t } = useTranslation();
  const unavailable = status === 'unavailable' || status === 'error';

  return (
    <section
      className='mb-12px shrink-0 rd-12px border border-solid border-[var(--color-border-2)] bg-1 px-14px py-12px'
      data-browser-display-mode-control
    >
      <div className='flex flex-wrap items-center gap-10px'>
        <div className='min-w-220px flex-1'>
          <div className='flex flex-wrap items-center gap-6px'>
            <h2 className='m-0 text-13px font-600 leading-20px'>
              {t('browser.displayMode.title')}
            </h2>
            <Tag color={displayMode === 'headless' ? 'green' : 'orange'}>
              {displayMode === 'headless'
                ? t('browser.displayMode.headlessShort')
                : t('browser.displayMode.externalShort')}
            </Tag>
          </div>
          <div className='mt-2px text-11px leading-17px text-t-tertiary'>
            {t('browser.displayMode.description')}
          </div>
        </div>
        {status === 'loading' ? (
          <Spin size={18} aria-label={t('browser.displayMode.loading')} />
        ) : (
          <RadioGroup
            type='button'
            value={displayMode}
            disabled={unavailable || saving || disabled}
            onChange={(value) =>
              onChange(value === 'external' ? 'external' : 'headless')
            }
            data-browser-display-mode-radio
          >
            <Radio value='headless'>{t('browser.displayMode.headless')}</Radio>
            <Radio value='external'>{t('browser.displayMode.external')}</Radio>
          </RadioGroup>
        )}
      </div>
      {(unavailable || error) && (
        <Alert
          className='mt-10px'
          type='warning'
          showIcon
          content={
            error
              ? t('browser.displayMode.errorWithDetails', { error })
              : t('browser.displayMode.unavailable')
          }
        />
      )}
    </section>
  );
};

export default BrowserDisplayModeControl;
