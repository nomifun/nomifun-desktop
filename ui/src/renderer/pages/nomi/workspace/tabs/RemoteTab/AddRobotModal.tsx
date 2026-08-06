/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { webui, type IApiRobotEndpoints } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import CopyIconButton from '@/renderer/components/base/CopyIconButton';
import NomiModal from '@/renderer/components/base/NomiModal';

interface AddRobotModalProps {
  visible: boolean;
  companionId: CompanionId;
  companionName: string;
  onCancel: () => void;
  onClaimed: () => void;
}

/**
 * 「添加机器人」弹窗：两步——把设备指向本机的 OTA 地址，然后输入设备屏上的 6 位激活码。
 *
 * Every non-loopback NIC is listed because the machine the robot can reach is
 * not necessarily the one the user thinks of as "the" address. The LAN listener
 * gate is stated here rather than hidden: with it off the device cannot connect
 * whatever it is configured with, so the dialog offers to switch it on.
 */
const AddRobotModal: React.FC<AddRobotModalProps> = ({
  visible,
  companionId,
  companionName,
  onCancel,
  onClaimed,
}) => {
  const { t } = useTranslation();
  const [endpoints, setEndpoints] = useState<IApiRobotEndpoints | null>(null);
  const [code, setCode] = useState('');
  const [claiming, setClaiming] = useState(false);
  const [enablingLan, setEnablingLan] = useState(false);

  const refreshEndpoints = useCallback(async () => {
    try {
      setEndpoints(await ipcBridge.robot.endpoints.invoke());
    } catch (error) {
      console.error('[RobotConnect] Failed to read the robot endpoints:', error);
      setEndpoints(null);
    }
  }, []);

  useEffect(() => {
    if (!visible) return;
    setCode('');
    void refreshEndpoints();
  }, [visible, refreshEndpoints]);

  const enableLan = useCallback(async () => {
    setEnablingLan(true);
    try {
      const status = await webui.start.invoke();
      if (status.error) throw new Error(status.error);
      Message.success(t('nomi.robot.lanEnabled'));
      await refreshEndpoints();
    } catch (error) {
      console.error('[RobotConnect] Failed to enable LAN access:', error);
      Message.error(t('nomi.robot.lanEnableFailed'));
    } finally {
      setEnablingLan(false);
    }
  }, [refreshEndpoints, t]);

  const claim = useCallback(async () => {
    setClaiming(true);
    try {
      await ipcBridge.robot.claim.invoke({ code: code.trim(), companion_id: companionId });
      Message.success(t('nomi.robot.claimOk', { companionName }));
      onClaimed();
    } catch (error) {
      console.error('[RobotConnect] Failed to claim a robot:', error);
      const status = isBackendHttpError(error) ? error.status : 0;
      if (status === 404) {
        Message.error(t('nomi.robot.claimNotFound'));
      } else if (status === 409) {
        Message.error(t('nomi.robot.claimTaken'));
      } else {
        Message.error(t('nomi.robot.claimFailed'));
      }
    } finally {
      setClaiming(false);
    }
  }, [code, companionId, companionName, onClaimed, t]);

  const lanOff = endpoints != null && !endpoints.lan_enabled;

  return (
    <NomiModal
      visible={visible}
      onCancel={onCancel}
      header={{ title: t('nomi.robot.addTitle'), showClose: true }}
      footer={null}
      style={{ width: 560 }}
    >
      <div className='flex flex-col gap-14px py-4px'>
        {lanOff && (
          <div className='flex flex-wrap items-center gap-8px rd-8px border border-solid border-[rgba(var(--warning-6),0.32)] bg-[rgba(var(--warning-6),0.08)] px-12px py-8px'>
            <span className='min-w-0 flex-1 text-12px leading-18px text-t-primary'>
              {t('nomi.robot.lanOff')}
            </span>
            {webui.lifecycleSupported ? (
              <Button
                size='mini'
                type='primary'
                loading={enablingLan}
                onClick={() => void enableLan()}
              >
                {t('nomi.robot.lanEnable')}
              </Button>
            ) : (
              <span className='text-12px text-t-tertiary'>{t('nomi.robot.lanUnavailable')}</span>
            )}
          </div>
        )}

        <div className='flex flex-col gap-6px'>
          <span className='text-12px leading-18px text-t-secondary'>{t('nomi.robot.otaStep')}</span>
          {endpoints == null || endpoints.ota_urls.length === 0 ? (
            <span className='text-12px text-t-tertiary'>{t('nomi.robot.otaNone')}</span>
          ) : (
            endpoints.ota_urls.map((url) => (
              <div
                key={url}
                className='flex min-w-0 items-center gap-8px rd-8px border border-solid border-[var(--color-border-2)] px-10px py-6px'
              >
                <span className='min-w-0 flex-1 truncate font-mono text-12px text-t-primary'>
                  {url}
                </span>
                <CopyIconButton text={url} size={14} className='h-22px w-22px shrink-0' />
              </div>
            ))
          )}
        </div>

        <div className='flex flex-col gap-6px'>
          <span className='text-12px leading-18px text-t-secondary'>{t('nomi.robot.codeStep')}</span>
          <div className='flex items-center gap-8px'>
            <Input
              value={code}
              maxLength={6}
              placeholder={t('nomi.robot.codePlaceholder')}
              className='max-w-160px'
              onChange={(next: string) => setCode(next.replace(/\D/g, ''))}
            />
            <Button
              type='primary'
              loading={claiming}
              disabled={code.trim().length !== 6}
              onClick={() => void claim()}
            >
              {t('nomi.robot.claim')}
            </Button>
          </div>
        </div>
      </div>
    </NomiModal>
  );
};

export default AddRobotModal;
