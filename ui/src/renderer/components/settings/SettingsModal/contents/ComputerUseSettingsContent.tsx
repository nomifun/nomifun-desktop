/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { configService } from '@/common/config/configService';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { Switch } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsViewMode } from '../settingsViewContext';
import PreferenceRow from './SystemModalContent/PreferenceRow';

const ComputerUseSettingsContent: React.FC = () => {
  const { t } = useTranslation();
  const viewMode = useSettingsViewMode();
  const isPageMode = viewMode === 'page';
  const [computerUse, setComputerUse] = useState(true);

  useEffect(() => {
    setComputerUse(configService.get('agent.computerUse') ?? true);
  }, []);

  const handleComputerUseChange = useCallback((checked: boolean) => {
    setComputerUse(checked);
    configService.set('agent.computerUse', checked).catch(() => {
      setComputerUse(!checked);
      configService.setLocal('agent.computerUse', !checked);
    });
  }, []);

  return (
    <div className='flex flex-col h-full w-full'>
      <NomiScrollArea className='flex-1 min-h-0 pb-16px' disableOverflow={isPageMode}>
        <div className='px-[12px] md:px-[32px] py-16px bg-2 rd-16px space-y-12px'>
          <div className='text-13px font-600 text-t-secondary'>{t('settings.computerUseSection')}</div>
          <div className='w-full flex flex-col divide-y divide-border-2'>
            <PreferenceRow label={t('settings.computerUse')} description={t('settings.computerUseDesc')}>
              <Switch checked={computerUse} onChange={handleComputerUseChange} />
            </PreferenceRow>
          </div>
        </div>
      </NomiScrollArea>
    </div>
  );
};

export default ComputerUseSettingsContent;
