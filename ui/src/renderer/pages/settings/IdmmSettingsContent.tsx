/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Message, Select } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { isHandledAuthExpiredHttpError } from '@/common/adapter/httpBridge';
import type { IIdmmSettings } from '@/common/adapter/ipcBridge';
import { useProvidersQuery } from '@renderer/hooks/agent/useModelProviderList';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import type { ProviderId } from '@/common/types/ids';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';

/**
 * Global IDMM defaults: the backup provider/model the sidecar uses when a
 * session does not pick its own, plus a default steering prompt. Embedded as a
 * tab in the Capabilities settings page.
 */
const IdmmSettingsContent: React.FC = () => {
  const { t } = useTranslation();
  const { data: providers } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const [settings, setSettings] = useState<IIdmmSettings>({ default_steering_prompt: '' });
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    void ipcBridge.idmm.getSettings
      .invoke()
      .then((s) => setSettings(s))
      .catch(() => {});
  }, []);

  const providerOptions = useMemo(
    () => (providers ?? []).map((p) => ({ label: providerLabel(p), value: p.id })),
    [providers, providerLabel]
  );

  // 备用模型做对话补全 —— 只列 chat-capable 模型（统一 catalog resolve；此前
  // 列出 provider 的全部模型，首次获得任务过滤）。
  const { groups: chatGroups } = useModelsForTask('chat');
  const modelOptions = useMemo(() => {
    const group = chatGroups.find((g) => g.provider.id === settings.backup_provider_id);
    return (group?.models ?? []).map((m) => ({ label: m, value: m }));
  }, [chatGroups, settings.backup_provider_id]);

  const save = async () => {
    setSaving(true);
    try {
      const saved = await ipcBridge.idmm.updateSettings.invoke(settings);
      setSettings(saved);
      Message.success(t('idmm.settings.saved'));
    } catch (e) {
      if (isHandledAuthExpiredHttpError(e)) return;
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className='flex flex-col gap-16px max-w-640px'>
      <div>
        <div className='text-t-primary text-15px font-600'>{t('idmm.settings.title')}</div>
        <div className='text-t-tertiary text-12px leading-18px mt-4px'>{t('idmm.settings.desc')}</div>
      </div>

      <div className='flex flex-col gap-6px'>
        <span className='text-t-secondary text-13px'>{t('idmm.settings.backupProvider')}</span>
        <Select
          placeholder={t('idmm.settings.selectProvider')}
          value={settings.backup_provider_id}
          onChange={(v: ProviderId | undefined) =>
            setSettings((s) => ({ ...s, backup_provider_id: v, backup_model: undefined }))
          }
          options={providerOptions}
          allowClear
        />
      </div>

      <div className='flex flex-col gap-6px'>
        <span className='text-t-secondary text-13px'>{t('idmm.settings.backupModel')}</span>
        <Select
          placeholder={t('idmm.settings.selectModel')}
          value={settings.backup_model}
          onChange={(v: string) => setSettings((s) => ({ ...s, backup_model: v }))}
          options={modelOptions}
          disabled={!settings.backup_provider_id}
          allowClear
        />
      </div>

      <div className='flex flex-col gap-6px'>
        <span className='text-t-secondary text-13px'>{t('idmm.settings.defaultSteering')}</span>
        <Input.TextArea
          placeholder={t('idmm.settings.defaultSteeringPlaceholder')}
          value={settings.default_steering_prompt}
          autoSize={{ minRows: 3, maxRows: 8 }}
          onChange={(v) => setSettings((s) => ({ ...s, default_steering_prompt: v }))}
        />
      </div>

      <div>
        <Button type='primary' loading={saving} onClick={save}>
          {t('common.save', { defaultValue: 'Save' })}
        </Button>
      </div>
    </div>
  );
};

export default IdmmSettingsContent;
