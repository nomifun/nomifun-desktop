/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Dropdown } from '@arco-design/web-react';
import { Brain, Down } from '@icon-park/react';
import type { TExecutionModelRef } from '@/common/types/agentExecution/agentExecutionTypes';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { decodePair, encodePair, useExecutionModelPool } from './useExecutionModelPool';
import { iconColors } from '@/renderer/styles/colors';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import composerStyles from './executionPlanEditor.module.css';

/** Sentinel select value = "跟随自动路由" (clears the per-task model override). */
const FOLLOW_AUTO = '__follow_auto__';

/** Case-insensitive substring match on an option's text. */
const filterByLabel = (input: string, option: React.ReactNode): boolean => {
  const children = (option as React.ReactElement<{ children?: React.ReactNode }>)?.props?.children;
  return String(children ?? '')
    .toLowerCase()
    .includes(input.toLowerCase());
};

type StepModelPillProps = {
  currentModel?: TExecutionModelRef;
  /** Persist the model override (`null` = follow auto-routing / clear). THROWS on
   * failure so the pill can toast. The parent merges this against the preset — this
   * pill never touches the preset, so it cannot wipe it. */
  onApply: (ref: TExecutionModelRef | null) => Promise<void>;
  className?: string;
};

/** Model override control for a task that has not started yet. */
const StepModelPill: React.FC<StepModelPillProps> = ({ currentModel, onApply, className }) => {
  const { t } = useTranslation();
  const [message, msgCtx] = useArcoMessage();
  const { providers, getAvailableModels, formatModelLabel, hasModels } = useExecutionModelPool();
  const [open, setOpen] = useState(false);

  const value = currentModel ? encodePair(currentModel) : FOLLOW_AUTO;

  const pillLabel =
    value === FOLLOW_AUTO
      ? t('agentExecution.configure.followAutomatic', {
          defaultValue: '自动选择',
        })
      : (currentModel?.model ?? '');

  const persist = async (next: string) => {
    try {
      await onApply(next !== FOLLOW_AUTO ? decodePair(next) : null);
      setOpen(false);
    } catch (e) {
      message.error(
        t('agentExecution.configure.saveError', {
          defaultValue: '保存失败：{{error}}',
          error: String(e),
        }),
      );
    }
  };

  const panel = (
    <div className={composerStyles.composerPopover}>
      <div className='flex flex-col gap-10px'>
        <div className='flex items-center gap-8px'>
          <Brain theme='outline' size='14' fill='rgb(var(--primary-6))' className='shrink-0' />
          <span className={composerStyles.composerPopoverTitle}>{t('agentExecution.configure.model', { defaultValue: '指定模型' })}</span>
        </div>
        {hasModels ? (
          <NomiSelect value={value} onChange={(v) => void persist(v as string)} showSearch filterOption={filterByLabel} className='w-full'>
            <NomiSelect.Option value={FOLLOW_AUTO}>
              {t('agentExecution.configure.followAutomatic', {
                defaultValue: '自动选择',
              })}
            </NomiSelect.Option>
            {providers.map((p) => (
              <NomiSelect.OptGroup key={p.id} label={p.name || p.platform}>
                {getAvailableModels(p).map((m) => {
                  const ref: TExecutionModelRef = {
                    provider_id: p.id,
                    model: m,
                  };
                  return (
                    <NomiSelect.Option key={encodePair(ref)} value={encodePair(ref)}>
                      {formatModelLabel(p, m)}
                    </NomiSelect.Option>
                  );
                })}
              </NomiSelect.OptGroup>
            ))}
          </NomiSelect>
        ) : (
          <span className='text-12px leading-18px text-[rgb(var(--warning-6))]'>
            {t('agentExecution.configure.noModels', {
              defaultValue: '暂无可用模型，请先配置模型。',
            })}
          </span>
        )}
        <span className={composerStyles.composerHint}>
          {t('agentExecution.configure.modelHint', {
            defaultValue: '可为该任务单独指定任意已配置模型。',
          })}
        </span>
      </div>
    </div>
  );

  return (
    <>
      {msgCtx}
      <Dropdown trigger='click' popupVisible={open} onVisibleChange={setOpen} droplist={panel} position='tr'>
        <Button
          className={`sendbox-model-btn ${className ?? ''}`}
          shape='round'
          size='small'
          aria-label={t('agentExecution.configure.model', {
            defaultValue: '指定模型',
          })}
        >
          <span className='flex items-center gap-6px min-w-0'>
            <Brain theme='outline' size='14' className='shrink-0' fill={iconColors.secondary} />
            <span className='truncate max-w-[160px]'>{pillLabel}</span>
            <Down theme='outline' size='12' className='shrink-0' fill={iconColors.secondary} />
          </span>
        </Button>
      </Dropdown>
    </>
  );
};

export default StepModelPill;
