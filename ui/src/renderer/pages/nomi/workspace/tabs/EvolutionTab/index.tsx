/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Spin } from '@arco-design/web-react';
import CollectionSourcesSection from './CollectionSourcesSection';
import LearningSection from './LearningSection';
import QuietHoursSection from './QuietHoursSection';
import RetentionSection from './RetentionSection';
import SkillGenerationSection from './SkillGenerationSection';
import StopAllSection from './StopAllSection';
import { useCollectSettings } from './useCollectSettings';
import { useEvolutionConfig } from './useEvolutionConfig';
import type { WorkspaceTabProps } from '../../types';

/**
 * 进化 — how this companion learns and grows new skills, in the order the user
 * asks it: whether it reviews your work (学习配置), what material there is to
 * review (采集来源 / 保留策略), what it does with the patterns it finds
 * (技能生成配置), when it must stay quiet (休眠时段), and how to stop all of it
 * (全部停止).
 *
 * 学习 / 进化 / 休眠 are per-companion profile fields. 采集 is not: it is one
 * device-wide config shared by every companion, and it lives here rather than in
 * app settings because feeding this companion's learning is the only reason it
 * exists — splitting the two across a page boundary made configuring learning a
 * trip out of the workspace. `useCollectSettings` is the single writer, and each
 * collect section's copy states the device-wide scope itself, because a control
 * that edits every companion's records while sitting in one companion's tab has
 * to say so where the user is looking. 全部停止 reaches furthest of all — it stops
 * learning across the whole roster — and its copy names that explicitly.
 */
const EvolutionTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { t } = useTranslation();
  const config = useEvolutionConfig(companionId);
  const collect = useCollectSettings();
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
  if (!config.learn || !config.evolve) {
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

  // The collect config fails independently of the profile, so it gets its own
  // notice instead of blanking the whole tab: 学习配置 and the per-companion
  // sections are still readable and editable when only this read failed. The
  // three collect sections return null on missing config, so without a visible
  // notice they would simply vanish — the exact silent disappearance these
  // controls exist to prevent.
  const collectBody = collect.collect ? (
    <>
      <CollectionSourcesSection settings={collect} />
      <RetentionSection settings={collect} />
    </>
  ) : collect.loading ? (
    <div className='flex justify-center py-20px'>
      <Spin />
    </div>
  ) : (
    <div className='flex flex-col items-center gap-8px py-20px text-center'>
      <span className='text-13px leading-19px text-t-secondary'>
        {t('nomi.collect.loadFailed', { defaultValue: '暂时读不到数据采集设置。' })}
      </span>
      {collect.error && (
        <span className='max-w-420px break-all text-12px leading-18px text-t-tertiary'>{collect.error}</span>
      )}
      <Button size='small' onClick={collect.retry}>
        {t('common.retry', { defaultValue: '重试' })}
      </Button>
    </div>
  );

  return (
    <div className='flex flex-col gap-16px py-8px'>
      <LearningSection config={config} needsModel={needsModel} />
      {collectBody}
      <SkillGenerationSection config={config} />
      <QuietHoursSection profile={profile} patchCompanion={patchCompanion} />
      {/* Last, and OUTSIDE `collectBody`, on purpose. Last because it is the
          master switch for everything above it, including 技能生成配置. Outside
          because the kill switch needs no current config to work — it only calls
          `disableAll` — and a panic switch that disappears exactly when something
          is wrong is worse than useless. `shell.structure.test.ts` pins both. */}
      <StopAllSection settings={collect} />
    </div>
  );
};

export default EvolutionTab;
