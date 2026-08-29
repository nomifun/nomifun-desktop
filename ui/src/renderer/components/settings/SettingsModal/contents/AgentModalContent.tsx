/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React from 'react';
import LocalAgents from '@/renderer/pages/settings/AgentSettings/LocalAgents';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { Button } from '@arco-design/web-react';
import { Right, SettingTwo } from '@icon-park/react';
import { useTranslation } from 'react-i18next';

/**
 * Execution-engine settings. With the native `nomi` runtime as the only engine
 * there is a single surface here, so the former local/runtime tab strip is gone
 * — the runtime tab only ever hosted the retired engine's timeout sliders.
 */
const AgentModalContent: React.FC = () => {
  const [, agentMessageContext] = useArcoMessage({ maxCount: 10 });
  const { t } = useTranslation();

  return (
    <div className='flex flex-col h-full w-full'>
      {agentMessageContext}
      <NomiScrollArea className='flex-1 min-h-0 pb-16px scrollbar-hide' disableOverflow>
        <div className='mx-16px mt-12px flex items-center gap-12px rounded-8px border border-solid border-[var(--color-border-2)] bg-[var(--color-bg-2)] px-12px py-10px'>
          <span className='flex size-32px shrink-0 items-center justify-center rounded-7px bg-primary-1 text-primary-6'>
            <SettingTwo theme='outline' size='17' />
          </span>
          <div className='min-w-0 flex-1'>
            <div className='text-13px font-600 text-t-primary'>
              {t('agentSettings.title')}
            </div>
            <div className='mt-2px text-11px leading-16px text-t-tertiary'>
              {t('agentSettings.navigation.entryDescription')}
            </div>
          </div>
          <Button
            type='secondary'
            size='small'
            icon={<Right theme='outline' size='14' />}
            onClick={() => {
              window.location.hash = '/settings/agent-presets';
            }}
          >
            {t('agentSettings.navigation.open')}
          </Button>
        </div>
        <LocalAgents />
      </NomiScrollArea>
    </div>
  );
};

export default AgentModalContent;
