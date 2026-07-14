/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Dropdown, Input } from '@arco-design/web-react';
import { Write } from '@icon-park/react';
import { iconColors } from '@/renderer/styles/colors';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import composerStyles from './executionPlanEditor.module.css';

type StepPresetPillProps = {
  /** Current persisted preset — seeds the draft and drives the dirty/highlight state. */
  initialPreset: string;
  /** Persist the trimmed requirement. Throws on failure so the pill can toast. */
  onApply: (preset: string) => Promise<void>;
  className?: string;
};

/** Compact control for requirements applied when a pending task starts. */
const StepPresetPill: React.FC<StepPresetPillProps> = ({ initialPreset, onApply, className }) => {
  const { t } = useTranslation();
  const [message, msgCtx] = useArcoMessage();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [preset, setPreset] = useState(initialPreset);

  // Seed the draft from the persisted value on each OPEN transition only — never while
  // the popover is open, so a background task refresh can't clobber in-progress typing.
  const onVisibleChange = (v: boolean) => {
    if (v) setPreset(initialPreset);
    setOpen(v);
  };

  const dirty = preset !== initialPreset;
  const hasPreset = initialPreset.trim().length > 0;

  const save = async () => {
    if (saving || !dirty) return;
    setSaving(true);
    try {
      await onApply(preset.trim());
      message.success(
        t('agentExecution.configure.saved', {
          defaultValue: '已保存，启动时自动生效',
        }),
      );
      setOpen(false);
    } catch (e) {
      message.error(
        t('agentExecution.configure.saveError', {
          defaultValue: '保存失败：{{error}}',
          error: String(e),
        }),
      );
    } finally {
      setSaving(false);
    }
  };

  const panel = (
    <div className={composerStyles.composerPopover}>
      <div className='flex flex-col gap-10px'>
        <div className='flex items-center gap-8px'>
          <Write theme='outline' size='14' fill='rgb(var(--primary-6))' className='shrink-0' />
          <span className={composerStyles.composerPopoverTitle}>
            {t('agentExecution.configure.presetLabel', {
              defaultValue: '预置要求',
            })}
          </span>
        </div>
        <Input.TextArea
          value={preset}
          onChange={setPreset}
          autoSize={{ minRows: 3, maxRows: 10 }}
          placeholder={t('agentExecution.configure.presetPlaceholder', {
            defaultValue: '写下该任务必须遵守的额外要求或偏好。',
          })}
        />
        <div className='flex items-center justify-between gap-8px'>
          <span className={composerStyles.composerHint}>
            {t('agentExecution.configure.presetPillHint', {
              defaultValue: '任务启动时生效',
            })}
          </span>
          <Button type='primary' size='mini' loading={saving} disabled={!dirty} onClick={() => void save()}>
            {t('agentExecution.configure.save', { defaultValue: '保存配置' })}
          </Button>
        </div>
      </div>
    </div>
  );

  return (
    <>
      {msgCtx}
      <Dropdown trigger='click' popupVisible={open} onVisibleChange={onVisibleChange} droplist={panel} position='tr'>
        <Button
          className={`sendbox-model-btn ${className ?? ''}`}
          shape='round'
          size='small'
          aria-label={t('agentExecution.configure.presetLabel', {
            defaultValue: '预置要求',
          })}
        >
          <span className='flex items-center gap-6px min-w-0'>
            <Write theme='outline' size='14' className='shrink-0' fill={hasPreset ? 'rgb(var(--primary-6))' : iconColors.secondary} />
            <span className='truncate' style={hasPreset ? { color: 'rgb(var(--primary-6))' } : undefined}>
              {t('agentExecution.configure.presetPill', {
                defaultValue: '预置要求',
              })}
            </span>
          </span>
        </Button>
      </Dropdown>
    </>
  );
};

export default StepPresetPill;
