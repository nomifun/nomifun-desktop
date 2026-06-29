/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { ipcBridge } from '@/common';
import type { ComputerPermissionKind, ComputerPermissionStatus } from '@/common/adapter/ipcBridge';
import { configService } from '@/common/config/configService';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { Button, Switch } from '@arco-design/web-react';
import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsViewMode } from '../settingsViewContext';
import PreferenceRow from './SystemModalContent/PreferenceRow';

const ComputerUseSettingsContent: React.FC = () => {
  const { t } = useTranslation();
  const viewMode = useSettingsViewMode();
  const isPageMode = viewMode === 'page';
  const [computerUse, setComputerUse] = useState(true);
  const [perm, setPerm] = useState<ComputerPermissionStatus | null>(null);

  useEffect(() => {
    setComputerUse(configService.get('agent.computerUse') ?? true);
  }, []);

  const refreshPerm = useCallback(() => {
    ipcBridge.computerPermissions.get
      .invoke()
      .then(setPerm)
      .catch(() => setPerm(null));
  }, []);

  useEffect(() => {
    refreshPerm();
    // Re-probe when the user returns from System Settings (the grant state is
    // the whole reason to revisit this panel).
    const onFocus = () => refreshPerm();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [refreshPerm]);

  const handleComputerUseChange = useCallback((checked: boolean) => {
    setComputerUse(checked);
    configService.set('agent.computerUse', checked).catch(() => {
      setComputerUse(!checked);
      configService.setLocal('agent.computerUse', !checked);
    });
  }, []);

  // Register the app in the relevant TCC list (and show the OS prompt), then
  // jump to the exact System Settings pane so the user can flip the toggle.
  const grant = useCallback((kind: ComputerPermissionKind) => {
    ipcBridge.computerPermissions.request
      .invoke({ kind })
      .then(setPerm)
      .catch(() => {});
    ipcBridge.computerPermissions.openSettings.invoke({ kind }).catch(() => {});
  }, []);

  const isMac = perm?.platform === 'macos';
  const appLabel = perm?.app_label || 'NomiFun';

  const permRow = (kind: ComputerPermissionKind, granted: boolean | null, label: string, description: string) => (
    <PreferenceRow label={label} description={description}>
      <div className='flex items-center gap-8px'>
        <span className={granted ? 'text-#16a34a font-600' : 'text-#ef4444 font-600'}>{granted ? t('settings.computerUsePermGranted') : t('settings.computerUsePermNotInEffect')}</span>
        <Button size='small' onClick={() => grant(kind)}>
          {t('settings.computerUseOpenSettings')}
        </Button>
      </div>
    </PreferenceRow>
  );

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

        {isMac && (
          <div className='px-[12px] md:px-[32px] py-16px bg-2 rd-16px space-y-12px mt-16px'>
            <div className='flex items-center justify-between'>
              <div className='text-13px font-600 text-t-secondary'>{t('settings.computerUsePermSection')}</div>
              <Button size='mini' onClick={refreshPerm}>
                {t('settings.computerUsePermRefresh')}
              </Button>
            </div>
            <div className='w-full flex flex-col divide-y divide-border-2'>
              {permRow('accessibility', perm?.accessibility ?? null, t('settings.computerUseAccessibility'), t('settings.computerUseAccessibilityDesc'))}
              {permRow('screen_recording', perm?.screen_recording ?? null, t('settings.computerUseScreenRecording'), t('settings.computerUseScreenRecordingDesc'))}
            </div>
            <div className='text-12px text-t-tertiary leading-relaxed'>{t('settings.computerUseRestartHint', { app: appLabel })}</div>
          </div>
        )}
      </NomiScrollArea>
    </div>
  );
};

export default ComputerUseSettingsContent;
