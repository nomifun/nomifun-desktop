/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Comment } from '@icon-park/react';
import type { TExecutionModelRef, TExecutionParticipant, TExecutionStep } from '@/common/types/agentExecution/agentExecutionTypes';
import composerStyles from './executionPlanEditor.module.css';
import StepModelPill from './StepModelPill';
import StepPresetPill from './StepPresetPill';

type StepConfigBarProps = {
  step: TExecutionStep;
  participant?: TExecutionParticipant;
  /** Persist the model override (`null` = follow auto). Throws on failure. */
  onApplyModel: (ref: TExecutionModelRef | null) => Promise<void>;
  /** Persist the preset requirement. Throws on failure. */
  onApplyPreset: (preset: string) => Promise<void>;
};

/** Pre-start configuration surface for a pending execution task. */
const StepConfigBar: React.FC<StepConfigBarProps> = ({ step, participant, onApplyModel, onApplyPreset }) => {
  const { t } = useTranslation();
  return (
    <div className='flex flex-1 min-h-0 flex-col'>
      {/* Explain that the collaborator has not started and what can be configured. */}
      <div className='flex flex-1 min-h-0 flex-col items-center justify-center gap-10px px-20px text-center'>
        <span
          className='flex size-48px items-center justify-center rd-14px'
          style={{
            color: 'rgb(var(--primary-6))',
            background: 'color-mix(in srgb, rgb(var(--primary-6)) 12%, transparent)',
          }}
        >
          <Comment theme='outline' size='24' strokeWidth={3} />
        </span>
        <div className='text-14px font-600 text-[var(--color-text-1)]'>
          {t('agentExecution.transcript.notStarted', {
            defaultValue: '任务尚未开始',
          })}
        </div>
        <div className='max-w-360px text-12px leading-18px text-[var(--color-text-3)]'>
          {t('agentExecution.configure.pendingHint', {
            defaultValue: '为该任务指定模型和预置要求，开始时自动生效。',
          })}
        </div>
      </div>

      {/* Composer-shaped config bar — same pills + transparent skin as a real composer. */}
      <div className='shrink-0 border-t border-solid border-[var(--color-border-2)] px-16px py-12px'>
        <div className={composerStyles.composerToolbar}>
          <StepModelPill
            currentModel={
              step.assignment_source === 'manual' && participant?.provider_id && participant.model
                ? {
                    provider_id: participant.provider_id,
                    model: participant.model,
                  }
                : undefined
            }
            onApply={onApplyModel}
          />
          <StepPresetPill initialPreset={step.preset_prompt ?? ''} onApply={onApplyPreset} />
        </div>
      </div>
    </div>
  );
};

export default StepConfigBar;
