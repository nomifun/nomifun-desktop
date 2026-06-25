/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Tabs, Typography } from '@arco-design/web-react';
import { Info } from '@icon-park/react';
import IdmmSettingsContent from '@/renderer/pages/settings/IdmmSettingsContent';
import IdmmActivityContent from '@/renderer/pages/modelHub/IdmmActivityContent';
import ModelFailoverContent from '@/renderer/pages/modelHub/ModelFailoverContent';

/**
 * GlobalModelConfig — the "Global Model Config" section of Model Management.
 * Houses the global default-model conversation settings (IDMM, the bypass
 * model, memory summarization, companion chat). For now it surfaces the IDMM
 * defaults as its single second-level tab; more global defaults can be added
 * as further tabs over time.
 */
const GlobalModelConfig: React.FC = () => {
  const { t } = useTranslation();

  return (
    <div className='flex flex-col gap-14px'>
      <div className='flex items-start gap-8px rounded-10px border border-solid border-[rgba(var(--primary-6),0.18)] bg-[rgba(var(--primary-6),0.06)] px-12px py-10px'>
        <Info theme='outline' size='15' fill='currentColor' className='mt-2px shrink-0 text-primary-6' />
        <Typography.Text className='text-12px leading-18px text-t-secondary'>
          {t('settings.modelHub.globalConfigTip')}
        </Typography.Text>
      </div>

      <Tabs type='line' defaultActiveTab='idmm' className='flex flex-col flex-1 min-h-0 [&>.arco-tabs-content]:pt-16px'>
        <Tabs.TabPane key='idmm' title={t('settings.modelHub.idmmTab')}>
          <IdmmSettingsContent />
        </Tabs.TabPane>
        <Tabs.TabPane key='failover' title={t('settings.modelHub.failoverTab')}>
          <ModelFailoverContent />
        </Tabs.TabPane>
        <Tabs.TabPane key='activity' title={t('settings.modelHub.activityTab')}>
          <IdmmActivityContent />
        </Tabs.TabPane>
      </Tabs>
    </div>
  );
};

export default GlobalModelConfig;
