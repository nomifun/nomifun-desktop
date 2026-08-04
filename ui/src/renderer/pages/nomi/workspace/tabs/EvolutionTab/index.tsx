/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Spin } from '@arco-design/web-react';
import LearningSection from './LearningSection';
import QuietHoursSection from './QuietHoursSection';
import SkillGenerationSection from './SkillGenerationSection';
import { useEvolutionConfig } from './useEvolutionConfig';
import type { WorkspaceTabProps } from '../../types';

/**
 * 进化 — how this companion learns and grows new skills. Three calm sections:
 * what it learns from (学习配置), what it does with the patterns it finds
 * (技能生成配置), and when it must stay quiet (休眠时段).
 *
 * The first two still read install-wide values; `useEvolutionConfig` is the one
 * place that knows, and each affected section prints one honest note.
 */
const EvolutionTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t } = useTranslation();
  const config = useEvolutionConfig(companionId);
  const { profile, patchCompanion } = companion;

  // Attention = something the user must act on: learning (or skill generation)
  // is switched on but has no model, so neither will ever run.
  const wantsModel = Boolean(config.learn?.enabled || config.evolve?.enabled);
  const needsModel = wantsModel && config.learn != null && config.learn.model == null;

  const attentionRef = useRef(onAttentionChange);
  attentionRef.current = onAttentionChange;
  useEffect(() => {
    attentionRef.current?.(needsModel);
  }, [needsModel]);
  useEffect(() => () => attentionRef.current?.(false), []);

  if ((config.loading && !config.learn) || !profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  // Config unreadable: say so and offer a retry, rather than rendering a tab
  // whose learning and skill sections have silently vanished.
  if (!config.learn || !config.evolve || !config.sources) {
    return (
      <div className='flex flex-col items-center gap-10px py-40px text-center'>
        <span className='text-13px leading-19px text-t-secondary'>
          {t('nomi.evolution.loadFailed', { defaultValue: '暂时读不到学习与进化设置。' })}
        </span>
        {config.error && (
          <span className='max-w-420px break-all text-12px leading-18px text-t-tertiary'>{config.error}</span>
        )}
        <Button size='small' onClick={config.retry}>
          {t('common.retry', { defaultValue: '重试' })}
        </Button>
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-16px py-8px'>
      <LearningSection config={config} needsModel={needsModel} />
      <SkillGenerationSection config={config} />
      <QuietHoursSection profile={profile} patchCompanion={patchCompanion} />
    </div>
  );
};

export default EvolutionTab;
