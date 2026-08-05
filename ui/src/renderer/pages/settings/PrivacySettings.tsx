/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * 数据采集 (`/settings/privacy`) — what this machine records about your work.
 *
 * These controls are app-level, not companion-level: `CollectConfig` is one global
 * shared-config object, so this page is its only editor (the 进化 tab links here
 * rather than duplicating the switches). Three questions, in the order a user asks
 * them: what is recorded, how long it is kept, and how to stop all of it.
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Spin } from '@arco-design/web-react';
import SettingsPageWrapper from './components/SettingsPageWrapper';
import CollectionSourcesSection from './privacy/CollectionSourcesSection';
import RetentionSection from './privacy/RetentionSection';
import StopAllSection from './privacy/StopAllSection';
import { useCollectSettings } from './privacy/useCollectSettings';

const PrivacySettings: React.FC = () => {
  const { t } = useTranslation();
  const settings = useCollectSettings();

  // No early return: the branches are a value, so every hook above stays
  // unconditional regardless of which state renders (Rules of Hooks).
  const body = (() => {
    if (settings.loading && !settings.collect) {
      return (
        <div className='flex justify-center py-40px'>
          <Spin />
        </div>
      );
    }
    if (!settings.collect) {
      return (
        <div className='flex flex-col items-center gap-10px py-40px text-center'>
          <span className='text-13px leading-19px text-t-secondary'>
            {t('settings.privacy.loadFailed', { defaultValue: '暂时读不到数据采集设置。' })}
          </span>
          {settings.error && (
            <span className='max-w-420px break-all text-12px leading-18px text-t-tertiary'>{settings.error}</span>
          )}
          <Button size='small' onClick={settings.retry}>
            {t('common.retry', { defaultValue: '重试' })}
          </Button>
        </div>
      );
    }
    return (
      <>
        <CollectionSourcesSection settings={settings} />
        <RetentionSection settings={settings} />
        <StopAllSection settings={settings} />
      </>
    );
  })();

  return (
    <SettingsPageWrapper contentClassName='max-w-860px'>
      <div className='flex flex-col gap-20px'>
        <header className='flex flex-col gap-3px'>
          <h1 className='m-0 text-16px leading-24px font-600 text-t-primary'>
            {t('settings.privacy.title', { defaultValue: '数据采集' })}
          </h1>
          <p className='m-0 text-12px leading-18px text-t-tertiary'>
            {t('settings.privacy.desc', {
              defaultValue:
                '这台设备把哪些工作数据记到本地、保留多久，以及如何一次全部停止。记录只写入本机的伙伴事件目录，不会上传。',
            })}
          </p>
        </header>
        {body}
      </div>
    </SettingsPageWrapper>
  );
};

export default PrivacySettings;
