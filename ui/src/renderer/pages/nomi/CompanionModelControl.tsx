/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { Select, Tooltip } from '@arco-design/web-react';
import { useModelProviderList } from '@renderer/hooks/agent/useModelProviderList';
import type { useCompanion } from './useNomi';

interface Props {
  /** 伙伴 profile + 乐观 patch 通道。 */
  companion: ReturnType<typeof useCompanion>;
}

/**
 * 桌面伙伴对话模型的【唯一】配置入口（紧凑内联，置于「对话」会话头部）。
 *
 * 写入 profile.model —— 全局唯一事实源：本地专属会话与远程连接(IM 机器人)都跟随此模型，
 * 切换后所有会话即时跟随（后端 service.patch_companion 会同步会话行并清空渠道会话）。
 *
 * 历史上分散的模型入口（「模型&知识」Tab、远程连接渠道表单的「默认模型」）已全部删除，
 * 此处是桌面伙伴对话模型的唯一可配置点。内联两枚 Select（不走 Popover，规避下拉门户
 * 与外层弹层的点击穿透问题），始终可见于会话上方（badcase 1：模型配置在「对话」处）。
 */
const CompanionModelControl: React.FC<Props> = ({ companion }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const { providers, getAvailableModels } = useModelProviderList();

  const currentProvider = useMemo(
    () => providers.find((p) => p.id === profile?.model.provider_id),
    [providers, profile?.model.provider_id]
  );

  if (!profile) return null;

  const configured = Boolean(profile.model.provider_id && profile.model.model);

  return (
    <div className='flex items-center gap-6px flex-wrap'>
      <Tooltip content={t('nomi.chat.modelConfigHint')}>
        <span className='flex items-center gap-4px text-12px text-t-tertiary shrink-0 cursor-help'>
          <span
            className='w-7px h-7px rd-full shrink-0'
            style={{ background: configured ? 'rgb(var(--success-6))' : 'rgb(var(--warning-6))' }}
          />
          {t('nomi.chat.modelConfig')}
        </span>
      </Tooltip>
      <Select
        size='mini'
        style={{ width: 148 }}
        placeholder={t('nomi.chat.modelProvider')}
        value={profile.model.provider_id || undefined}
        onChange={(provider_id: string) => void patchCompanion({ model: { provider_id, model: '' } })}
      >
        {providers.map((p) => (
          <Select.Option key={p.id} value={p.id}>
            {p.name}
          </Select.Option>
        ))}
      </Select>
      <Select
        size='mini'
        style={{ width: 176 }}
        placeholder={t('nomi.chat.modelName')}
        value={profile.model.model || undefined}
        disabled={!currentProvider}
        onChange={(model: string) => void patchCompanion({ model: { model } })}
      >
        {(currentProvider ? getAvailableModels(currentProvider) : []).map((m) => (
          <Select.Option key={m} value={m}>
            {m}
          </Select.Option>
        ))}
      </Select>
    </div>
  );
};

export default CompanionModelControl;
