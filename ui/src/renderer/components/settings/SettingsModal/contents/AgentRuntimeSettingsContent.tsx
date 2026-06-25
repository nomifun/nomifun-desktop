/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { configService } from '@/common/config/configService';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { InputNumber } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsViewMode } from '../settingsViewContext';
import PreferenceRow from './SystemModalContent/PreferenceRow';

const AgentRuntimeSettingsContent: React.FC = () => {
  const { t } = useTranslation();
  const viewMode = useSettingsViewMode();
  const isPageMode = viewMode === 'page';
  const [promptTimeout, setPromptTimeout] = useState<number>(300);
  const [agentIdleTimeout, setAgentIdleTimeout] = useState<number>(5);

  useEffect(() => {
    const pt = configService.get('acp.promptTimeout');
    if (pt && pt > 0) setPromptTimeout(pt);
    const ait = configService.get('acp.agentIdleTimeout');
    if (ait && ait > 0) setAgentIdleTimeout(ait);
  }, []);

  const handlePromptTimeoutChange = useCallback((val: number | undefined) => {
    setPromptTimeout(val as number);
  }, []);

  const handlePromptTimeoutBlur = useCallback(() => {
    const clamped = Math.max(30, Math.min(3600, promptTimeout || 300));
    setPromptTimeout(clamped);
    configService.set('acp.promptTimeout', clamped).catch(() => {});
  }, [promptTimeout]);

  const handleAgentIdleTimeoutChange = useCallback((val: number | undefined) => {
    setAgentIdleTimeout(val as number);
  }, []);

  const handleAgentIdleTimeoutBlur = useCallback(() => {
    const clamped = Math.max(1, Math.min(60, agentIdleTimeout || 5));
    setAgentIdleTimeout(clamped);
    configService.set('acp.agentIdleTimeout', clamped).catch(() => {});
  }, [agentIdleTimeout]);

  return (
    <div className='flex flex-col h-full w-full'>
      <NomiScrollArea className='flex-1 min-h-0 pb-16px' disableOverflow={isPageMode}>
        <div className='px-[12px] md:px-[32px] py-16px bg-2 rd-16px space-y-12px'>
          <div className='text-13px font-600 text-t-secondary'>{t('settings.agentRuntimeSection')}</div>
          <div className='w-full flex flex-col divide-y divide-border-2'>
            <PreferenceRow label={t('settings.promptTimeout')} description={t('settings.promptTimeoutDesc')}>
              <InputNumber
                value={promptTimeout}
                onChange={handlePromptTimeoutChange}
                onBlur={handlePromptTimeoutBlur}
                min={30}
                max={3600}
                step={30}
                style={{ width: 120 }}
                suffix='s'
              />
            </PreferenceRow>
            <PreferenceRow label={t('settings.agentIdleTimeout')} description={t('settings.agentIdleTimeoutDesc')}>
              <InputNumber
                value={agentIdleTimeout}
                onChange={handleAgentIdleTimeoutChange}
                onBlur={handleAgentIdleTimeoutBlur}
                min={1}
                max={60}
                step={5}
                style={{ width: 120 }}
                suffix='min'
              />
            </PreferenceRow>
          </div>
        </div>
      </NomiScrollArea>
    </div>
  );
};

export default AgentRuntimeSettingsContent;
