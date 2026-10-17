/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Tooltip } from '@arco-design/web-react';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import type { useCompanion } from './useNomi';

interface Props {
  /** 伙伴 profile + 乐观 patch 通道。 */
  companion: ReturnType<typeof useCompanion>;
  /** 总览的“基础配置”行已经渲染标题时，隐藏内联重复标签。 */
  showLabel?: boolean;
  role?: 'companion' | 'robot';
}

/**
 * 伙伴总览中的表单式模型配置入口；聊天页使用标准输入框模型选择器。
 *
 * 写入 profile.model，供远程连接重建和后续新 AgentSession 使用。已经打开的
 * 本地会话保留创建时冻结的模型。
 *
 * 供应商下拉用 `scope='all-enabled'`（列出所有已启用供应商，而不只是「含 chat 模型」的
 * 那些）：这样用户始终看得到自己配置的供应商，已存储的当前供应商也能显示名字而不是生
 * provider id；只有图像/视频/嵌入类模型的供应商也可见，其模型下拉为空并给出说明。失效
 * 引用一律由 TaskModelSelect 渲染成禁用的「(不可用)」项。
 */
const CompanionModelControl: React.FC<Props> = ({ companion, showLabel = true, role = 'companion' }) => {
  const { t } = useTranslation();
  const { profile, patchCompanion } = companion;
  const [saving, setSaving] = useState(false);
  const model = role === 'robot' ? profile?.robot?.model ?? profile?.model : profile?.model;
  const configured = Boolean(model?.provider_id && model?.model);
  const inherited = role === 'robot' && !profile?.robot?.model;
  const save = async (next: typeof model) => {
    if (saving) return;
    setSaving(true);
    try {
      await patchCompanion(role === 'robot' ? { robot: { model: next ?? null } } : { model: next ?? null }, { optimistic: false });
    } catch { Message.error(t('agentSettings.productBinding.saveFailed')); }
    finally { setSaving(false); }
  };

  if (!profile) return null;

  return (
    <div className='flex flex-col gap-6px'>
      <div className='flex items-center gap-6px flex-wrap'>
        {showLabel && (
          <Tooltip content={t('nomi.chat.modelConfigHint')}>
            <span className='flex items-center gap-4px text-12px text-t-tertiary shrink-0 cursor-help'>
              <span
                className='w-7px h-7px rd-full shrink-0'
                style={{
                  background: configured ? 'rgb(var(--success-6))' : 'rgb(var(--warning-6))',
                }}
              />
              {t('nomi.chat.modelConfig')}
            </span>
          </Tooltip>
        )}
        <TaskModelSelect
          task='chat'
          scope='all-enabled'
          value={model ?? null}
          disabled={saving}
          placeholder={inherited ? t('nomi.cohabit.robotInherited') : undefined}
          emptyHint={t('nomi.chat.modelNoTextModel')}
          onChange={({ provider_id, model }) => void save({ provider_id, model })}
        />
      </div>
      {role === 'robot' && <span className='text-11px text-t-tertiary'>{t(inherited ? 'nomi.cohabit.robotInherited' : 'nomi.cohabit.robotIndependent')}</span>}
      {role === 'robot' && !inherited && <Button size='mini' type='text' disabled={saving} onClick={() => void save(null)}>
        {t('nomi.cohabit.inheritForFuture')}
      </Button>}
    </div>
  );
};

export default CompanionModelControl;
